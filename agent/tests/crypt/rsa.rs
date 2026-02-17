// internal crates
use miru_agent::crypt::errors::CryptErr;
use miru_agent::crypt::rsa;
use miru_agent::filesys::{dir::Dir, file::File, path::PathExt, Overwrite, WriteOptions};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod gen_key_pair {
    use super::*;

    // TEST CASES
    #[ignore]
    #[tokio::test]
    async fn sandbox() {
        let dir = Dir::new_home_dir().unwrap().subdir("Downloads");
        let private_key_path = dir.path().join("private_key.pem");
        let public_key_path = dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());

        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        let result =
            rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow).await;
        assert!(result.is_ok());

        assert!(private_key_file.exists());
        assert!(public_key_file.exists());
    }

    // TEST CASES
    #[tokio::test]
    async fn success_doesnt_exist_overwrite_true() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());

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
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
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
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        // public key file exists
        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
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
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny)
            .await
            .unwrap_err();

        assert!(private_key_file.exists());
    }

    #[tokio::test]
    async fn failure_existing_files_overwrite_false() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        // public key file exists
        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny)
            .await
            .unwrap_err();
        public_key_file.delete().await.unwrap();

        // private key file exists
        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Deny)
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn invalid_key_size() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());

        // Invalid key size
        let result = rsa::gen_key_pair(0, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap_err();
        assert!(matches!(result, CryptErr::GenerateRSAKeyPairErr { .. }));
    }
}

pub mod read_private_key {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
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
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        let result = rsa::read_private_key(&private_key_file).await;
        assert!(result.is_err());
    }
}

pub mod read_public_key {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
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
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        let result = rsa::read_public_key(&public_key_file).await;
        assert!(result.is_err());
    }
}

pub mod sign {
    use super::*;

    #[tokio::test]
    async fn success1() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign(&private_key_file, data).await.unwrap();
        assert!(!signature.is_empty());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        private_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        let data = b"hello world";
        let result = rsa::sign(&private_key_file, data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        private_key_file.delete().await.unwrap();

        let data = b"hello world";
        let result = rsa::sign(&private_key_file, data).await;
        assert!(result.is_err());
    }
}

pub mod verify {
    use super::*;

    #[tokio::test]
    async fn success() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let private_key_path = crypt_dir.path().join("private_key.pem");
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let private_key_file = File::new(private_key_path.clone());
        let public_key_file = File::new(public_key_path.clone());
        private_key_file.delete().await.unwrap();
        public_key_file.delete().await.unwrap();

        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let data = b"hello world";
        let signature = rsa::sign(&private_key_file, data).await.unwrap();
        let result = rsa::verify(&public_key_file, data, &signature).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn invalid_file() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        public_key_file
            .write_bytes(&[4, 4], WriteOptions::OVERWRITE)
            .await
            .unwrap();
        let data = b"hello world";
        let signature = vec![4, 4];
        let result = rsa::verify(&public_key_file, data, &signature).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_file() {
        let crypt_dir = Dir::create_temp_dir("crypt_rsa_test").await.unwrap();
        let public_key_path = crypt_dir.path().join("public_key.pem");

        let public_key_file = File::new(public_key_path.clone());
        public_key_file.delete().await.unwrap();

        let data = b"hello world";
        let signature = vec![4, 4];
        let result = rsa::verify(&public_key_file, data, &signature).await;
        assert!(result.is_err());
    }
}
