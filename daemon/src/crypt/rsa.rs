// standard crates
use std::fmt::Write;

// internal crates
use crate::crypt::errors::*;
use crate::filesys::{self, files, Atomic, Overwrite, WriteOptions};
use crate::trace;

// external crates
use aws_lc_rs::digest;
use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::PublicKey;
use aws_lc_rs::signature::{self, KeyPair as _, RsaKeyPair, UnparsedPublicKey};
use pem_rfc7468::LineEnding;
use secrecy::ExposeSecret;

/// Supported RSA modulus sizes for [`gen_key_pair`], re-exported so callers name
/// sizes in the type system instead of passing raw bit counts.
pub use aws_lc_rs::rsa::KeySize;

/// PEM armor label for PKCS#1 `RSAPrivateKey` — what every device provisioned
/// before the aws-lc-rs migration has on disk. Read-only; never written anymore.
const PKCS1_LABEL: &str = "RSA PRIVATE KEY";
/// PEM armor label for PKCS#8 `PrivateKeyInfo` — what `gen_key_pair` writes.
const PKCS8_LABEL: &str = "PRIVATE KEY";
/// PEM armor label for SPKI `SubjectPublicKeyInfo` — the only public-key format
/// read or written. The backend stores this PEM verbatim at (re)provision.
const SPKI_LABEL: &str = "PUBLIC KEY";

/// Generate an RSA key pair and write the private key (PKCS#8, mode 600) and
/// public key (SPKI, mode 640) to the given files. Returns an error if a file
/// exists and `overwrite` is [`Overwrite::Deny`]. Keys are written to disk
/// rather than returned so they spend as little time in memory as possible.
/// Pre-migration private keys on disk are PKCS#1 and remain readable via
/// [`read_private_key`].
pub async fn gen_key_pair(
    size: KeySize,
    private_key_file: &filesys::File,
    public_key_file: &filesys::File,
    overwrite: Overwrite,
) -> Result<(), CryptErr> {
    // Generate the RSA key pair on a blocking thread so the 4096-bit keygen
    // (hundreds of ms of pure CPU) does not pin an async worker thread and stall
    // concurrent tasks (MQTT loop, poller, local socket server). Only the raw
    // `RsaKeyPair::generate` moves into the closure; the error mapping stays in
    // the async body so its `trace!()`/`?` machinery runs in the async context. A
    // JoinError only occurs if the blocking task panics, which would have
    // propagated inline before this change too, so we let it propagate.
    let key_pair = tokio::task::spawn_blocking(move || RsaKeyPair::generate(size))
        .await
        .expect("rsa keygen task panicked");
    let key_pair = key_pair.map_err(|e| {
        CryptErr::GenerateRSAKeyPairErr(GenerateRSAKeyPairErr {
            source: e,
            trace: trace!(),
        })
    })?;

    // Extract and write the private key (PKCS#8; keys written before the
    // aws-lc-rs migration are PKCS#1 and stay readable via `read_private_key`'s
    // label dispatch)
    let private_key_pem = private_key_to_pem(&key_pair)?;
    files::write_bytes(
        private_key_file,
        private_key_pem.as_bytes(),
        WriteOptions {
            overwrite,
            atomic: Atomic::Yes,
            mode: Some(0o600),
        },
    )
    .await?;

    // Extract and write the public key
    let public_key_pem = public_key_to_pem(&key_pair)?;
    files::write_bytes(
        public_key_file,
        public_key_pem.as_bytes(),
        WriteOptions {
            overwrite,
            atomic: Atomic::Yes,
            mode: Some(0o640),
        },
    )
    .await?;

    Ok(())
}

/// Encode the private key as PKCS#8 PEM. The intermediate DER buffer zeroizes on
/// drop; the returned PEM `String` is written to disk immediately by the caller.
fn private_key_to_pem(key_pair: &RsaKeyPair) -> Result<String, CryptErr> {
    let der = AsDer::<Pkcs8V1Der>::as_der(key_pair).map_err(|e| {
        CryptErr::ConvertPrivateKeyToDERErr(ConvertPrivateKeyToDERErr {
            source: e,
            trace: trace!(),
        })
    })?;
    pem_rfc7468::encode_string(PKCS8_LABEL, LineEnding::LF, der.as_ref()).map_err(|e| {
        CryptErr::ConvertPrivateKeyToPEMErr(ConvertPrivateKeyToPEMErr {
            source: e,
            trace: trace!(),
        })
    })
}

/// Encode the public key as SPKI PEM.
fn public_key_to_pem(key_pair: &RsaKeyPair) -> Result<String, CryptErr> {
    let der = public_key_spki_der(key_pair.public_key())?;
    pem_rfc7468::encode_string(SPKI_LABEL, LineEnding::LF, der.as_ref()).map_err(|e| {
        CryptErr::ConvertPublicKeyToPEMErr(ConvertPublicKeyToPEMErr {
            source: e,
            trace: trace!(),
        })
    })
}

/// Read an RSA private key from the specified file.
pub async fn read_private_key(private_key_file: &filesys::File) -> Result<RsaKeyPair, CryptErr> {
    let private_key_pem = files::read_secret_bytes(private_key_file).await?;
    parse_private_key_pem(private_key_pem.expose_secret())
}

/// Parse a PEM private key, dispatching on the armor label: PKCS#1 (pre-migration
/// keys on device disks) or PKCS#8 (what `gen_key_pair` writes). This matches the
/// dual-format acceptance of the previous OpenSSL generic reader.
fn parse_private_key_pem(pem: &[u8]) -> Result<RsaKeyPair, CryptErr> {
    let (label, der) = decode_pem(pem)?;
    private_key_from_der(label, &der)
}

/// PKCS#1 (`RSA PRIVATE KEY`) or PKCS#8 (`PRIVATE KEY`).
fn private_key_from_der(label: &str, der: &[u8]) -> Result<RsaKeyPair, CryptErr> {
    let parse = match label {
        PKCS1_LABEL => RsaKeyPair::from_der(der),
        PKCS8_LABEL => RsaKeyPair::from_pkcs8(der),
        other => return Err(unsupported_pem_label(other)),
    };
    parse.map_err(|e| {
        CryptErr::ParsePrivateKeyErr(ParsePrivateKeyErr {
            source: e,
            trace: trace!(),
        })
    })
}

/// Read an RSA public key from the specified file.
pub async fn read_public_key(public_key_file: &filesys::File) -> Result<PublicKey, CryptErr> {
    let public_key_pem = files::read_bytes(public_key_file).await?;
    parse_public_key_pem(&public_key_pem)
}

/// Parse a PEM public key. SPKI armor only.
fn parse_public_key_pem(pem: &[u8]) -> Result<PublicKey, CryptErr> {
    public_key_from_spki_der(&decode_spki_pem(pem)?)
}

/// Canonical fingerprint of an RSA public key: lowercase hex SHA-256 over the
/// DER-encoded SubjectPublicKeyInfo
pub fn fingerprint(key: &PublicKey) -> Result<String, CryptErr> {
    // Must hash the SPKI DER (`PublicKeyX509Der`) — the fingerprint is the JWT
    // `kid` the backend looks up devices by, so it must stay byte-stable.
    // (`key.as_ref()` would yield PKCS#1 `RSAPublicKey` DER: a different hash.)
    let der = public_key_spki_der(key)?;
    let digest = digest::digest(&digest::SHA256, der.as_ref());
    let digest = digest.as_ref();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

/// Create an RSASSA-PKCS1-v1_5 (RFC 7518 §3.2) signature using SHA-256.
pub async fn sign_rs256(
    private_key_file: &filesys::File,
    data: &[u8],
) -> Result<Vec<u8>, CryptErr> {
    sign(private_key_file, data, &signature::RSA_PKCS1_SHA256).await
}

/// Create an RSASSA-PKCS1-v1_5 (RFC 7518 §3.3) signature using SHA-512.
pub async fn sign_rs512(
    private_key_file: &filesys::File,
    data: &[u8],
) -> Result<Vec<u8>, CryptErr> {
    sign(private_key_file, data, &signature::RSA_PKCS1_SHA512).await
}

async fn sign(
    private_key_file: &filesys::File,
    data: &[u8],
    padding: &'static dyn signature::RsaEncoding,
) -> Result<Vec<u8>, CryptErr> {
    let key_pair = read_private_key(private_key_file).await?;
    // The buffer must be exactly `public_modulus_len()` bytes: `sign` panics on a
    // wrong-sized buffer rather than returning an error. The RNG argument is
    // required by the signature but unused (PKCS#1 v1.5 is deterministic).
    let mut sig = vec![0u8; key_pair.public_modulus_len()];
    key_pair
        .sign(padding, &SystemRandom::new(), data, &mut sig)
        .map_err(|e| {
            CryptErr::SignDataErr(SignDataErr {
                source: e,
                trace: trace!(),
            })
        })?;
    Ok(sig)
}

/// Verify an RSASSA-PKCS1-v1_5 (RFC 7518 §3.2) SHA-256 signature.
pub async fn verify_rs256(
    public_key_file: &filesys::File,
    data: &[u8],
    signature_bytes: &[u8],
) -> Result<bool, CryptErr> {
    verify(
        public_key_file,
        data,
        signature_bytes,
        &signature::RSA_PKCS1_2048_8192_SHA256,
    )
    .await
}

/// Verify an RSASSA-PKCS1-v1_5 (RFC 7518 §3.3) SHA-512 signature.
pub async fn verify_rs512(
    public_key_file: &filesys::File,
    data: &[u8],
    signature_bytes: &[u8],
) -> Result<bool, CryptErr> {
    verify(
        public_key_file,
        data,
        signature_bytes,
        &signature::RSA_PKCS1_2048_8192_SHA512,
    )
    .await
}

/// Verify a signature with the given algorithm. Returns `Ok(false)` on an
/// invalid signature; `Err` only for key/file problems.
async fn verify(
    public_key_file: &filesys::File,
    data: &[u8],
    signature_bytes: &[u8],
    alg: &'static dyn signature::VerificationAlgorithm,
) -> Result<bool, CryptErr> {
    let pem = files::read_bytes(public_key_file).await?;
    let der = decode_spki_pem(&pem)?;
    // Reject unparseable keys as Err (key problem), not Ok(false) (bad signature).
    public_key_from_spki_der(&der)?;
    let unparsed = UnparsedPublicKey::new(alg, &der);
    Ok(unparsed.verify(data, signature_bytes).is_ok())
}

fn public_key_spki_der(key: &PublicKey) -> Result<PublicKeyX509Der<'static>, CryptErr> {
    AsDer::<PublicKeyX509Der>::as_der(key).map_err(|e| {
        CryptErr::ConvertPublicKeyToDERErr(ConvertPublicKeyToDERErr {
            source: e,
            trace: trace!(),
        })
    })
}

fn public_key_from_spki_der(der: &[u8]) -> Result<PublicKey, CryptErr> {
    PublicKey::from_der(der).map_err(|e| {
        CryptErr::ParsePublicKeyErr(ParsePublicKeyErr {
            source: e,
            trace: trace!(),
        })
    })
}

/// SPKI DER from a public-key PEM. Label must be [`SPKI_LABEL`].
fn decode_spki_pem(pem: &[u8]) -> Result<Vec<u8>, CryptErr> {
    let (label, der) = decode_pem(pem)?;
    if label != SPKI_LABEL {
        return Err(unsupported_pem_label(label));
    }
    Ok(der)
}

fn unsupported_pem_label(label: &str) -> CryptErr {
    CryptErr::UnsupportedPEMLabelErr(UnsupportedPEMLabelErr {
        label: label.to_string(),
        trace: trace!(),
    })
}

/// Decode PEM to `(label, der)`, trimming trailing ASCII whitespace first:
/// `pem_rfc7468` rejects any bytes after the END line, but PEM files commonly
/// end with extra newlines.
fn decode_pem(pem: &[u8]) -> Result<(&str, Vec<u8>), CryptErr> {
    pem_rfc7468::decode_vec(trim_trailing_whitespace(pem)).map_err(|e| {
        CryptErr::DecodePEMErr(DecodePEMErr {
            source: e,
            trace: trace!(),
        })
    })
}

fn trim_trailing_whitespace(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    &bytes[..end]
}
