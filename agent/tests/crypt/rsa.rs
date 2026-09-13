// standard crates
use std::os::unix::fs::PermissionsExt;
pub use std::path::PathBuf;

// internal crates
use crate::tests::test_utils::filesys::dirs as test_dirs;
pub use crate::tests::test_utils::testdata::testdata_dir;
use miru_agent::crypt::{rsa, CryptErr};
use miru_agent::filesys::{self, files, Overwrite, PathExt, WriteOptions};

async fn temp_key_pair() -> (test_dirs::TempDir, filesys::File, filesys::File) {
    let dir = test_dirs::temp("crypt_rsa_test").unwrap();
    let private_key_file = dir.file("private_key.pem");
    let public_key_file = dir.file("public_key.pem");
    rsa::gen_key_pair(
        rsa::KeySize::Rsa2048,
        &private_key_file,
        &public_key_file,
        Overwrite::Allow,
    )
    .await
    .unwrap();
    (dir, private_key_file, public_key_file)
}

fn crypt_testdata() -> filesys::Dir {
    testdata_dir().subdir(PathBuf::from("crypt"))
}

pub mod fingerprint {
    use super::*;
    use miru_agent::crypt::rsa::fingerprint;

    #[tokio::test]
    async fn success_deterministic_for_known_key() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(
            rsa::KeySize::Rsa2048,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let public_key = rsa::read_public_key(&public_key_file).await.unwrap();
        let fp_a = fingerprint(&public_key).unwrap();
        let fp_b = fingerprint(&public_key).unwrap();

        // deterministic: two calls yield the same fingerprint
        assert_eq!(fp_a, fp_b);

        // shape: 64-char lowercase hex
        assert_eq!(fp_a.len(), 64);
        assert!(fp_a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[tokio::test]
    async fn differs_across_keys() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let priv1 = filesys::File::new(crypt_dir.path().join("priv1.pem"));
        let pub1 = filesys::File::new(crypt_dir.path().join("pub1.pem"));
        rsa::gen_key_pair(rsa::KeySize::Rsa2048, &priv1, &pub1, Overwrite::Allow)
            .await
            .unwrap();
        let priv2 = filesys::File::new(crypt_dir.path().join("priv2.pem"));
        let pub2 = filesys::File::new(crypt_dir.path().join("pub2.pem"));
        rsa::gen_key_pair(rsa::KeySize::Rsa2048, &priv2, &pub2, Overwrite::Allow)
            .await
            .unwrap();

        let key1 = rsa::read_public_key(&pub1).await.unwrap();
        let key2 = rsa::read_public_key(&pub2).await.unwrap();
        assert_ne!(fingerprint(&key1).unwrap(), fingerprint(&key2).unwrap());
    }
}

pub mod gen_key_pair {
    use super::*;

    #[tokio::test]
    async fn success_doesnt_exist_overwrite_true() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());

        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();

        let result = rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await;
        assert!(result.is_ok());

        assert!(private_key_file.exists());
        assert!(public_key_file.exists());
    }

    #[tokio::test]
    async fn success_doesnt_exist_overwrite_false() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();

        let result = rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Deny,
        )
        .await;
        assert!(result.is_ok());

        assert!(private_key_file.exists());
        assert!(public_key_file.exists());
    }

    #[tokio::test]
    async fn success_existing_files_overwrite_true() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();

        // public key file exists
        files::write_bytes(&public_key_file, &[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await
        .unwrap();
        assert!(public_key_file.exists());

        // private key file exists
        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();
        files::write_bytes(
            &private_key_file,
            &[4, 4],
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
        rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Deny,
        )
        .await
        .unwrap_err();

        assert!(private_key_file.exists());
    }

    #[tokio::test]
    async fn failure_existing_files_overwrite_false() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();

        // public key file exists
        files::write_bytes(&public_key_file, &[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Deny,
        )
        .await
        .unwrap_err();
        files::delete(&public_key_file).await.unwrap();

        // private key file exists
        files::write_bytes(
            &private_key_file,
            &[4, 4],
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
        rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Deny,
        )
        .await
        .unwrap_err();
    }

    #[tokio::test]
    async fn file_permissions() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());

        rsa::gen_key_pair(
            rsa::KeySize::Rsa2048,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let private_perms = files::permissions(&private_key_file).await.unwrap();
        let public_perms = files::permissions(&public_key_file).await.unwrap();
        assert_eq!(
            private_perms.mode() & 0o777,
            0o600,
            "private key should be 600"
        );
        assert_eq!(
            public_perms.mode() & 0o777,
            0o640,
            "public key should be 640"
        );
    }

    #[tokio::test]
    async fn writes_pkcs8_private_and_spki_public_pem() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(
            rsa::KeySize::Rsa2048,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        // New private keys are PKCS#8 (the aws-lc-rs migration's write-format
        // flip; pre-migration keys are PKCS#1 and remain readable). Public keys
        // stay SPKI: the backend stores that PEM verbatim.
        let private_pem = files::read_string(&private_key_file).await.unwrap();
        assert!(private_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        let public_pem = files::read_string(&public_key_file).await.unwrap();
        assert!(public_pem.starts_with("-----BEGIN PUBLIC KEY-----"));

        // Full round trip on the freshly written pair: read → sign → verify.
        let data = b"pkcs8 round trip";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        assert!(rsa::verify_rs256(&public_key_file, data, &signature)
            .await
            .unwrap());
    }
}

pub mod read_private_key {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();

        rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        files::delete(&private_key_file).await.unwrap();

        files::write_bytes(
            &private_key_file,
            &[4, 4],
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        files::delete(&private_key_file).await.unwrap();

        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_err());
    }

    async fn write_pem_and_read_private_key(pem: &str) -> Result<(), CryptErr> {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        files::write_bytes(
            &private_key_file,
            pem.as_bytes(),
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
        rsa::read_private_key(&private_key_file).await.map(|_| ())
    }

    #[tokio::test]
    async fn unsupported_pem_label() {
        // Valid PEM armor, but a label the dispatch does not accept.
        let pem = "-----BEGIN EC PRIVATE KEY-----\nAAAA\n-----END EC PRIVATE KEY-----\n";
        let result = write_pem_and_read_private_key(pem).await.unwrap_err();
        assert!(matches!(result, CryptErr::UnsupportedPEMLabelErr(_)));
    }

    #[tokio::test]
    async fn pkcs1_label_with_invalid_der() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n";
        let result = write_pem_and_read_private_key(pem).await.unwrap_err();
        assert!(matches!(result, CryptErr::ParsePrivateKeyErr(_)));
    }

    #[tokio::test]
    async fn pkcs8_label_with_invalid_der() {
        let pem = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
        let result = write_pem_and_read_private_key(pem).await.unwrap_err();
        assert!(matches!(result, CryptErr::ParsePrivateKeyErr(_)));
    }
}

pub mod read_public_key {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&private_key_file).await.unwrap();
        files::delete(&public_key_file).await.unwrap();

        rsa::gen_key_pair(
            rsa::KeySize::Rsa4096,
            &private_key_file,
            &public_key_file,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&public_key_file).await.unwrap();

        files::write_bytes(&public_key_file, &[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = filesys::File::new(public_key_path.clone());
        files::delete(&public_key_file).await.unwrap();

        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_err());
    }

    async fn write_pem_and_read_public_key(pem: &str) -> Result<(), CryptErr> {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));
        files::write_bytes(
            &public_key_file,
            pem.as_bytes(),
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
        rsa::read_public_key(&public_key_file).await.map(|_| ())
    }

    #[tokio::test]
    async fn unsupported_pem_label() {
        // SPKI armor is the only accepted public-key format.
        let pem = "-----BEGIN RSA PUBLIC KEY-----\nAAAA\n-----END RSA PUBLIC KEY-----\n";
        let result = write_pem_and_read_public_key(pem).await.unwrap_err();
        assert!(matches!(result, CryptErr::UnsupportedPEMLabelErr(_)));
    }

    #[tokio::test]
    async fn spki_label_with_invalid_der() {
        let pem = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n";
        let result = write_pem_and_read_public_key(pem).await.unwrap_err();
        assert!(matches!(result, CryptErr::ParsePublicKeyErr(_)));
    }
}

pub mod sign {
    use super::*;

    #[tokio::test]
    async fn round_trip_rs256() {
        let (_dir, private_key_file, public_key_file) = super::temp_key_pair().await;
        let data = b"hello world";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        assert!(rsa::verify_rs256(&public_key_file, data, &signature)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn round_trip_rs512() {
        let (_dir, private_key_file, public_key_file) = super::temp_key_pair().await;
        let data = b"hello world";
        let signature = rsa::sign_rs512(&private_key_file, data).await.unwrap();
        assert!(rsa::verify_rs512(&public_key_file, data, &signature)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_file = crypt_dir.file("private_key.pem");
        files::delete(&private_key_file).await.unwrap();

        let result = rsa::sign_rs256(&private_key_file, b"hello world").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let private_key_file = crypt_dir.file("private_key.pem");
        files::write_bytes(
            &private_key_file,
            &[4, 4],
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();

        let result = rsa::sign_rs256(&private_key_file, b"hello world").await;
        assert!(result.is_err());
    }
}

pub mod verify {
    use super::*;

    #[tokio::test]
    async fn wrong_data_returns_false() {
        let (_dir, private_key_file, public_key_file) = super::temp_key_pair().await;
        let signature = rsa::sign_rs256(&private_key_file, b"hello world")
            .await
            .unwrap();
        let is_valid = rsa::verify_rs256(&public_key_file, b"different data", &signature)
            .await
            .unwrap();
        assert!(!is_valid);
    }

    #[tokio::test]
    async fn wrong_key_pair_returns_false() {
        let dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let priv1 = dir.file("priv1.pem");
        let pub1 = dir.file("pub1.pem");
        rsa::gen_key_pair(rsa::KeySize::Rsa2048, &priv1, &pub1, Overwrite::Allow)
            .await
            .unwrap();
        let priv2 = dir.file("priv2.pem");
        let pub2 = dir.file("pub2.pem");
        rsa::gen_key_pair(rsa::KeySize::Rsa2048, &priv2, &pub2, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign_rs256(&priv1, data).await.unwrap();
        let is_valid = rsa::verify_rs256(&pub2, data, &signature).await.unwrap();
        assert!(!is_valid);
    }

    #[tokio::test]
    async fn empty_data() {
        let (_dir, private_key_file, public_key_file) = super::temp_key_pair().await;
        let data = b"";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        assert!(!signature.is_empty());
        let is_valid = rsa::verify_rs256(&public_key_file, data, &signature)
            .await
            .unwrap();
        assert!(is_valid);
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let public_key_file = crypt_dir.file("public_key.pem");
        files::delete(&public_key_file).await.unwrap();

        let result = rsa::verify_rs256(&public_key_file, b"hello world", &[4, 4]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = test_dirs::temp("crypt_rsa_test").unwrap();
        let public_key_file = crypt_dir.file("public_key.pem");
        files::write_bytes(&public_key_file, &[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();

        let result = rsa::verify_rs256(&public_key_file, b"hello world", &[4, 4]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rejects_other_algorithm() {
        let crypt = super::crypt_testdata();
        let spki = crypt.file("rsa2048_spki.pem");
        let message = files::read_bytes(&crypt.file("message.txt")).await.unwrap();
        let rs256_sig = files::read_bytes(&crypt.file("message.sig.rs256"))
            .await
            .unwrap();
        let rs512_sig = files::read_bytes(&crypt.file("message.sig.rs512"))
            .await
            .unwrap();

        assert!(!rsa::verify_rs256(&spki, &message, &rs512_sig)
            .await
            .unwrap());
        assert!(!rsa::verify_rs512(&spki, &message, &rs256_sig)
            .await
            .unwrap());
    }
}

/// Golden-fixture parity tests. The fixtures in `testdata/crypt/` were generated
/// once with the OpenSSL CLI (see the exec plan) and pin the on-disk formats every
/// provisioned device depends on: PKCS#1 and PKCS#8 private-key reads, deterministic
/// RSASSA-PKCS1-v1_5 signatures, and the SPKI fingerprint the backend uses as the
/// JWT `kid`. They must keep passing, byte-for-byte, across any crypto-stack change.
pub mod golden {
    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        testdata_dir()
            .subdir(PathBuf::from("crypt"))
            .path()
            .join(name)
    }

    fn fixture_file(name: &str) -> filesys::File {
        filesys::File::new(fixture_path(name))
    }

    async fn fixture_bytes(name: &str) -> Vec<u8> {
        files::read_bytes(&fixture_file(name))
            .await
            .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
    }

    #[tokio::test]
    async fn sign_rs256_matches_golden_signature_for_pkcs1_and_pkcs8() {
        let message = fixture_bytes("message.txt").await;
        let expected = fixture_bytes("message.sig.rs256").await;

        let from_pkcs1 = rsa::sign_rs256(&fixture_file("rsa2048_pkcs1.pem"), &message)
            .await
            .unwrap();
        assert_eq!(from_pkcs1, expected);

        // same key as PKCS#8: proves both private-key read paths load the same key
        let from_pkcs8 = rsa::sign_rs256(&fixture_file("rsa2048_pkcs8.pem"), &message)
            .await
            .unwrap();
        assert_eq!(from_pkcs8, expected);
    }

    #[tokio::test]
    async fn sign_rs512_matches_golden_signature_for_pkcs1_and_pkcs8() {
        let message = fixture_bytes("message.txt").await;
        let expected = fixture_bytes("message.sig.rs512").await;

        let from_pkcs1 = rsa::sign_rs512(&fixture_file("rsa2048_pkcs1.pem"), &message)
            .await
            .unwrap();
        assert_eq!(from_pkcs1, expected);

        // same key as PKCS#8: proves both private-key read paths load the same key
        let from_pkcs8 = rsa::sign_rs512(&fixture_file("rsa2048_pkcs8.pem"), &message)
            .await
            .unwrap();
        assert_eq!(from_pkcs8, expected);
    }

    #[tokio::test]
    async fn verify_rs256_accepts_golden_signature_and_rejects_tampered_message() {
        let message = fixture_bytes("message.txt").await;
        let signature = fixture_bytes("message.sig.rs256").await;
        let spki = fixture_file("rsa2048_spki.pem");

        assert!(rsa::verify_rs256(&spki, &message, &signature)
            .await
            .unwrap());

        let mut tampered = message.clone();
        tampered[0] ^= 0x01;
        assert!(!rsa::verify_rs256(&spki, &tampered, &signature)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn verify_rs512_accepts_golden_signature_and_rejects_tampered_message() {
        let message = fixture_bytes("message.txt").await;
        let signature = fixture_bytes("message.sig.rs512").await;
        let spki = fixture_file("rsa2048_spki.pem");

        assert!(rsa::verify_rs512(&spki, &message, &signature)
            .await
            .unwrap());

        let mut tampered = message.clone();
        tampered[0] ^= 0x01;
        assert!(!rsa::verify_rs512(&spki, &tampered, &signature)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn fingerprint_matches_golden_spki_digest() {
        let public_key = rsa::read_public_key(&fixture_file("rsa2048_spki.pem"))
            .await
            .unwrap();
        let fingerprint = rsa::fingerprint(&public_key).unwrap();

        let expected = String::from_utf8(fixture_bytes("fingerprint.txt").await).unwrap();
        assert_eq!(fingerprint, expected.trim_end());
    }
}
