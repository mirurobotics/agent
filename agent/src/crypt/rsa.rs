// standard crates
use std::os::unix::fs::PermissionsExt;

// internal crates
use crate::crypt::errors::*;
use crate::filesys::{self, Atomic, Overwrite, PathExt, WriteOptions};
use crate::trace;

// external crates
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private, Public};
use openssl::rsa::Rsa;
use openssl::sign::{Signer, Verifier};
use secrecy::ExposeSecret;
use serde::Serialize;

/// JSON Web Key (RFC 7517) representation of an RSA public key.
///
/// Field order on the wire is `kty`, `n`, `e` because `serde_json` preserves
/// struct field declaration order; the server parses the JSON without caring
/// about order, but tests assert stability for fingerprinting.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Jwk {
    pub kty: &'static str,
    pub n: String,
    pub e: String,
}

/// Serialize an RSA public key as a JWK (RFC 7517) with `n` and `e` as
/// big-endian base64url-no-pad-encoded byte strings.
pub fn rsa_public_key_to_jwk(key: &Rsa<Public>) -> Jwk {
    Jwk {
        kty: "RSA",
        n: crate::crypt::base64::encode_bytes_url_safe_no_pad(&key.n().to_vec()),
        e: crate::crypt::base64::encode_bytes_url_safe_no_pad(&key.e().to_vec()),
    }
}

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
/// key file is given read/write permissions only to the owner (600). The public key
/// file is given read/write permissions for the owner and read permissions for the
/// group (640). https://www.redhat.com/sysadmin/linux-file-permissions-explained
pub async fn gen_key_pair(
    num_bits: u32,
    private_key_file: &filesys::File,
    public_key_file: &filesys::File,
    overwrite: Overwrite,
) -> Result<(), CryptErr> {
    // Generate the RSA key pair
    let rsa = ssl_err!(GenerateRSAKeyPairErr, Rsa::generate(num_bits))?;

    // Extract and write the private key
    let private_key_pem = ssl_err!(ConvertPrivateKeyToPEMErr, rsa.private_key_to_pem())?;
    private_key_file
        .write_bytes(
            &private_key_pem,
            WriteOptions {
                overwrite,
                atomic: Atomic::Yes,
            },
        )
        .await?;
    // 600 gives the owner read/write permissions. Permissions to the group and others
    // are not granted.
    let permissions = std::fs::Permissions::from_mode(0o600);
    private_key_file.set_permissions(permissions).await?;

    // Extract and write the public key
    let public_key_pem = ssl_err!(ConvertPublicKeyToPEMErr, rsa.public_key_to_pem())?;
    public_key_file
        .write_bytes(
            &public_key_pem,
            WriteOptions {
                overwrite,
                atomic: Atomic::Yes,
            },
        )
        .await?;
    // 640 gives the owner read/write permissions, the group read permissions, and
    // nothing for other
    let permissions = std::fs::Permissions::from_mode(0o640);
    public_key_file.set_permissions(permissions).await?;

    Ok(())
}

/// Read an RSA private key from the specified file.
pub async fn read_private_key(private_key_file: &filesys::File) -> Result<Rsa<Private>, CryptErr> {
    private_key_file.assert_exists()?;
    let private_key_pem = private_key_file.read_secret_bytes().await?;
    ssl_err!(
        ReadKeyErr,
        Rsa::private_key_from_pem(private_key_pem.expose_secret())
    )
}

/// Read an RSA public key from the specified file.
pub async fn read_public_key(public_key_file: &filesys::File) -> Result<Rsa<Public>, CryptErr> {
    public_key_file.assert_exists()?;
    let public_key_pem = public_key_file.read_secret_bytes().await?;
    ssl_err!(
        ReadKeyErr,
        Rsa::public_key_from_pem(public_key_pem.expose_secret())
    )
}

/// Shared signing core. Reads the private key, converts to a PKey, then
/// signs `data` with the supplied digest. SHA-256 is used by the public
/// `sign`; SHA-512 is used by `sign_rs512` (for RS512 JWTs). Centralizing
/// the OpenSSL plumbing here keeps the per-digest entry points to a
/// single delegation each.
async fn sign_with_digest(
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

/// Create a signature from the provided data using the private key stored in the
/// specified file. Uses SHA-256.
pub async fn sign(private_key_file: &filesys::File, data: &[u8]) -> Result<Vec<u8>, CryptErr> {
    sign_with_digest(private_key_file, data, MessageDigest::sha256()).await
}

/// Create an RS512 (RSASSA-PKCS1-v1_5 with SHA-512) signature, per RFC 7518
/// §3.3. Used to sign self-issued JWTs for the `/devices/issue_token`
/// endpoint.
pub async fn sign_rs512(
    private_key_file: &filesys::File,
    data: &[u8],
) -> Result<Vec<u8>, CryptErr> {
    sign_with_digest(private_key_file, data, MessageDigest::sha512()).await
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
