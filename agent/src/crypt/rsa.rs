// standard crates
use std::fmt::Write;

// internal crates
use crate::crypt::errors::*;
use crate::filesys::{self, files, Atomic, Overwrite, PathExt, WriteOptions};
use crate::trace;

// external crates
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private, Public};
use openssl::rsa::Rsa;
use openssl::sha::sha256;
use openssl::sign::{Signer, Verifier};
use secrecy::ExposeSecret;

/// Maps an `openssl::error::ErrorStack` to a `CryptErr` variant. The variant name and
/// inner struct name must match (e.g. `SignDataErr` maps to `CryptErr::SignDataErr(SignDataErr { .. })`).
macro_rules! ssl_err {
    ($variant:ident, $expr:expr) => {
        $expr.map_err(|e| {
            CryptErr::$variant($variant {
                source: e,
                trace: trace!(),
            })
        })
    };
}

/// Generate an RSA key pair and write the private and public keys to the specified
/// files. If the files exists, an error is returned. Files are returned instead of
/// variables holding the keys to avoid keeping sensitive information in memory. In
/// general you shouldn't interact directly with the keys but let individual functions
/// read and write to their respective files so their existence in memory is as brief as
/// possible. The public key technically doesn't need such security measures since it
/// can be shared publicly, but it's simpler to treat both keys the same. The private
/// key file is created with read/write permissions only for the owner (600) and the
/// public key file with read/write for the owner and read for the group (640). These
/// modes are applied at file-creation time, so the keys are never briefly written at a
/// more permissive mode and no follow-up chmod is required.
/// https://www.redhat.com/sysadmin/linux-file-permissions-explained
pub async fn gen_key_pair(
    num_bits: u32,
    private_key_file: &filesys::File,
    public_key_file: &filesys::File,
    overwrite: Overwrite,
) -> Result<(), CryptErr> {
    // Generate the RSA key pair on a blocking thread so the 4096-bit keygen
    // (hundreds of ms of pure CPU) does not pin an async worker thread and stall
    // concurrent tasks (MQTT loop, poller, local socket server). Only the raw
    // `Rsa::generate` moves into the closure; the `ssl_err!` mapping stays in the
    // async body so its `trace!()`/`?` machinery runs in the async context. A
    // JoinError only occurs if the blocking task panics, which would have
    // propagated inline before this change too, so we let it propagate.
    let rsa = tokio::task::spawn_blocking(move || Rsa::generate(num_bits))
        .await
        .expect("rsa keygen task panicked");
    let rsa = ssl_err!(GenerateRSAKeyPairErr, rsa)?;

    // Extract and write the private key
    let private_key_pem = ssl_err!(ConvertPrivateKeyToPEMErr, rsa.private_key_to_pem())?;
    files::write_bytes(
        private_key_file,
        &private_key_pem,
        WriteOptions {
            overwrite,
            atomic: Atomic::Yes,
            mode: Some(0o600),
        },
    )
    .await?;

    // Extract and write the public key
    let public_key_pem = ssl_err!(ConvertPublicKeyToPEMErr, rsa.public_key_to_pem())?;
    files::write_bytes(
        public_key_file,
        &public_key_pem,
        WriteOptions {
            overwrite,
            atomic: Atomic::Yes,
            mode: Some(0o640),
        },
    )
    .await?;

    Ok(())
}

/// Read an RSA private key from the specified file.
pub async fn read_private_key(private_key_file: &filesys::File) -> Result<Rsa<Private>, CryptErr> {
    private_key_file.assert_exists()?;
    let private_key_pem = files::read_secret_bytes(private_key_file).await?;
    ssl_err!(
        ReadKeyErr,
        Rsa::private_key_from_pem(private_key_pem.expose_secret())
    )
}

/// Read an RSA public key from the specified file.
pub async fn read_public_key(public_key_file: &filesys::File) -> Result<Rsa<Public>, CryptErr> {
    public_key_file.assert_exists()?;
    let public_key_pem = files::read_secret_bytes(public_key_file).await?;
    ssl_err!(
        ReadKeyErr,
        Rsa::public_key_from_pem(public_key_pem.expose_secret())
    )
}

/// Canonical fingerprint of an RSA public key: lowercase hex SHA-256 over the
/// DER-encoded SubjectPublicKeyInfo
pub fn fingerprint(key: &Rsa<Public>) -> Result<String, CryptErr> {
    let der = ssl_err!(ConvertPublicKeyToDERErr, key.public_key_to_der())?;
    let digest = sha256(&der);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

async fn sign(
    private_key_file: &filesys::File,
    data: &[u8],
    digest: MessageDigest,
) -> Result<Vec<u8>, CryptErr> {
    let rsa_private_key = read_private_key(private_key_file).await?;
    let private_key = ssl_err!(RSAToPKeyErr, PKey::from_rsa(rsa_private_key))?;

    let mut signer = ssl_err!(SignDataErr, Signer::new(digest, &private_key))?;
    ssl_err!(SignDataErr, signer.update(data))?;
    let signature = ssl_err!(SignDataErr, signer.sign_to_vec())?;
    Ok(signature)
}

/// Create an RSASSA-PKCS1-v1_5 (RFC 7518 §3.2) signature using SHA-256.
pub async fn sign_rs256(
    private_key_file: &filesys::File,
    data: &[u8],
) -> Result<Vec<u8>, CryptErr> {
    sign(private_key_file, data, MessageDigest::sha256()).await
}

/// Create an RSASSA-PKCS1-v1_5 (RFC 7518 §3.3) signature using SHA-512.
pub async fn sign_rs512(
    private_key_file: &filesys::File,
    data: &[u8],
) -> Result<Vec<u8>, CryptErr> {
    sign(private_key_file, data, MessageDigest::sha512()).await
}

/// Verify a signature using the public key stored in the specified file
pub async fn verify(
    public_key_file: &filesys::File,
    data: &[u8],
    signature: &[u8],
) -> Result<bool, CryptErr> {
    let rsa_public_key = read_public_key(public_key_file).await?;
    let public_key = ssl_err!(RSAToPKeyErr, PKey::from_rsa(rsa_public_key))?;

    let mut verifier = ssl_err!(
        VerifyDataErr,
        Verifier::new(MessageDigest::sha256(), &public_key)
    )?;
    ssl_err!(VerifyDataErr, verifier.update(data))?;
    let is_valid = ssl_err!(VerifyDataErr, verifier.verify(signature))?;
    Ok(is_valid)
}
