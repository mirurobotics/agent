// standard crates
use std::os::unix::fs::PermissionsExt;

// internal crates
use miru_agent::crypt::{rsa, CryptErr};
use miru_agent::filesys::{self, Overwrite, PathExt, WriteOptions};

pub mod pub_key_to_jwk {
    use super::*;
    use miru_agent::crypt::rsa::pub_key_to_jwk;

    #[tokio::test]
    async fn success_deterministic_for_known_key() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let public_key = rsa::read_public_key(&public_key_file).await.unwrap();
        let jwk_a = pub_key_to_jwk(&public_key);
        let jwk_b = pub_key_to_jwk(&public_key);

        // deterministic: two calls yield the same struct
        assert_eq!(jwk_a, jwk_b);

        // shape: kty is RSA, n and e are non-empty url-safe-no-pad strings
        assert_eq!(jwk_a.kty, "RSA");
        assert!(!jwk_a.n.is_empty());
        assert!(!jwk_a.e.is_empty());
        let url_safe = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        assert!(jwk_a.n.chars().all(url_safe));
        assert!(jwk_a.e.chars().all(url_safe));
    }

    #[tokio::test]
    async fn serialized_json_field_order() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let public_key = rsa::read_public_key(&public_key_file).await.unwrap();
        let jwk = pub_key_to_jwk(&public_key);
        let serialized = serde_json::to_string(&jwk).unwrap();

        assert!(serialized.contains(r#""kty":"RSA""#));
        assert!(serialized.contains(r#""n":"#));
        assert!(serialized.contains(r#""e":"#));
    }
}

pub mod gen_key_pair {
    use super::*;

    #[tokio::test]
    async fn success_doesnt_exist_overwrite_true() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());

        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        let result =
            rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow).await;
        assert!(result.is_ok());

        assert!(private_key_file.exists());
        assert!(public_key_file.exists());
    }

    #[tokio::test]
    async fn success_doesnt_exist_overwrite_false() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        let result =
            rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny).await;
        assert!(result.is_ok());

        assert!(private_key_file.exists());
        assert!(public_key_file.exists());
    }

    #[tokio::test]
    async fn success_existing_files_overwrite_true() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        // public key file exists
        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        assert!(public_key_file.exists());

        // private key file exists
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();
        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny)
            .await
            .unwrap_err();

        assert!(private_key_file.exists());
    }

    #[tokio::test]
    async fn failure_existing_files_overwrite_false() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        // public key file exists
        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny)
            .await
            .unwrap_err();
        public_key_file.delete().await.unwrap();

        // private key file exists
        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny)
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn invalid_key_size() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());

        // Invalid key size
        let result = rsa::gen_key_pair(0, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap_err();
        assert!(matches!(result, CryptErr::GenerateRSAKeyPairErr { .. }));
    }

    #[tokio::test]
    async fn file_permissions() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let private_perms = private_key_file.permissions().await.unwrap();
        let public_perms = public_key_file.permissions().await.unwrap();
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
}

pub mod read_private_key {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_err());
    }
}

pub mod read_public_key {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = filesys::File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = filesys::File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_err());
    }
}

pub mod sign_rs256 {
    use super::*;

    #[tokio::test]
    async fn success1() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        assert!(!signature.is_empty());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let data = b"hello world";
        let result = rsa::sign_rs256(&private_key_file, data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        let data = b"hello world";
        let result = rsa::sign_rs256(&private_key_file, data).await;
        assert!(result.is_err());
    }
}

pub mod verify {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        let result = rsa::verify(&public_key_file, data, &signature).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn wrong_data_returns_false() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = filesys::File::new(private_key_path.clone());
        let public_key_file = filesys::File::new(public_key_path.clone());

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        // verify with different data — should return Ok(false)
        let is_valid = rsa::verify(&public_key_file, b"different data", &signature)
            .await
            .unwrap();
        assert!(!is_valid);
    }

    #[tokio::test]
    async fn wrong_key_pair_returns_false() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();

        // generate two key pairs
        let priv1 = filesys::File::new(crypt_dir.path().join("priv1.pem"));
        let pub1 = filesys::File::new(crypt_dir.path().join("pub1.pem"));
        rsa::gen_key_pair(2048, &priv1, &pub1, Overwrite::Allow)
            .await
            .unwrap();

        let priv2 = filesys::File::new(crypt_dir.path().join("priv2.pem"));
        let pub2 = filesys::File::new(crypt_dir.path().join("pub2.pem"));
        rsa::gen_key_pair(2048, &priv2, &pub2, Overwrite::Allow)
            .await
            .unwrap();

        // sign with key pair 1, verify with key pair 2's public key
        let data = b"hello world";
        let signature = rsa::sign_rs256(&priv1, data).await.unwrap();
        let is_valid = rsa::verify(&pub2, data, &signature).await.unwrap();
        assert!(!is_valid);
    }

    #[tokio::test]
    async fn empty_data() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"";
        let signature = rsa::sign_rs256(&private_key_file, data).await.unwrap();
        assert!(!signature.is_empty());
        let is_valid = rsa::verify(&public_key_file, data, &signature)
            .await
            .unwrap();
        assert!(is_valid);
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = filesys::File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let data = b"hello world";
        let signature = vec![4, 4];
        let result = rsa::verify(&public_key_file, data, &signature).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = filesys::File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        let data = b"hello world";
        let signature = vec![4, 4];
        let result = rsa::verify(&public_key_file, data, &signature).await;
        assert!(result.is_err());
    }
}

pub mod sign_rs512 {
    use super::*;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::sign::Verifier;

    #[tokio::test]
    async fn success() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign_rs512(&private_key_file, data).await.unwrap();
        // RS512 with a 2048-bit key produces a 256-byte signature
        assert!(!signature.is_empty());
        assert!(signature.len() > 200);
    }

    #[tokio::test]
    async fn verifies_with_sha512() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        let public_key_file = filesys::File::new(crypt_dir.path().join("public_key.pem"));

        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"jwt signing input";
        let signature = rsa::sign_rs512(&private_key_file, data).await.unwrap();

        // Verify directly via openssl with SHA-512: should be valid
        let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
        let pkey = PKey::from_rsa(public_key_rsa).unwrap();
        let mut verifier_512 = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
        verifier_512.update(data).unwrap();
        assert!(verifier_512.verify(&signature).unwrap());

        // Verifying with SHA-256 should fail (sentinel that the digest is
        // genuinely SHA-512)
        let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
        let pkey = PKey::from_rsa(public_key_rsa).unwrap();
        let mut verifier_256 = Verifier::new(MessageDigest::sha256(), &pkey).unwrap();
        verifier_256.update(data).unwrap();
        assert!(!verifier_256.verify(&signature).unwrap());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        private_key_file.delete().await.unwrap();

        let result = rsa::sign_rs512(&private_key_file, b"hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = filesys::Dir::create_temp_dir("crypt_rsa_test")
            .await
            .unwrap();
        let private_key_file = filesys::File::new(crypt_dir.path().join("private_key.pem"));
        private_key_file.delete().await.unwrap();

        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let result = rsa::sign_rs512(&private_key_file, b"hello").await;
        assert!(result.is_err());
    }
}
