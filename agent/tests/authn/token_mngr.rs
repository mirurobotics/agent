// standard library
use std::sync::Arc;

// internal crates
use crate::http::mock::MockClient;
use miru_agent::authn::{
    errors::AuthnErr,
    token::Token,
    token_mngr::{SingleThreadTokenManager, TokenFile, TokenManager, TokenManagerExt, Worker},
};
use miru_agent::crypt::rsa;
use miru_agent::filesys::{dir::Dir, file::File, Overwrite, WriteOptions};
use miru_agent::http;
use miru_agent::http::errors::{HTTPErr, MockErr};
use openapi_client::models::TokenResponse;

// external crates
use chrono::{Duration, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub fn spawn(
    buffer_size: usize,
    device_id: String,
    http_client: Arc<MockClient>,
    token_file: TokenFile,
    private_key_file: File,
) -> Result<(TokenManager, JoinHandle<()>), AuthnErr> {
    let (sender, receiver) = mpsc::channel(buffer_size);
    let worker = Worker::new(
        SingleThreadTokenManager::new(device_id, http_client, token_file, private_key_file)?,
        receiver,
    );
    let worker_handle = tokio::spawn(worker.run());
    Ok((TokenManager::new(sender), worker_handle))
}

pub mod spawn {
    use super::*;

    #[tokio::test]
    async fn token_file_does_not_exist() {
        // create and delete the token file
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        token_file.file.delete().await.unwrap();

        // spawn the token manager
        let http_client = http::Client::new("doesntmatter").unwrap();
        let result = TokenManager::spawn(
            32,
            "device_id".to_string(),
            Arc::new(http_client),
            token_file,
            File::new("private_key.pem"),
        )
        .unwrap_err();
        assert!(matches!(result, AuthnErr::FileSysErr(_)));
    }

    #[tokio::test]
    async fn private_key_file_does_not_exist() {
        // create the token file
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        token_file.file.delete().await.unwrap();

        // create and delete the private key file
        let private_key_file = dir.file("private_key.pem");
        private_key_file.delete().await.unwrap();

        // spawn the token manager
        let http_client = http::Client::new("doesntmatter").unwrap();
        let result = TokenManager::spawn(
            32,
            "device_id".to_string(),
            Arc::new(http_client),
            token_file,
            File::new("private_key.pem"),
        )
        .unwrap_err();

        assert!(matches!(result, AuthnErr::FileSysErr(_)));
    }

    #[tokio::test]
    async fn success() {
        // create the token file
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let http_client = http::Client::new("doesntmatter").unwrap();
        TokenManager::spawn(
            32,
            "device_id".to_string(),
            Arc::new(http_client),
            token_file,
            private_key_file,
        )
        .unwrap();
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn shutdown() {
        // create the token file
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        let mock_http_client = MockClient::default();
        let (token_mngr, worker_handle) = spawn(
            32,
            "device_id".to_string(),
            Arc::new(mock_http_client),
            token_file,
            private_key_file,
        )
        .unwrap();
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }
}

pub mod get_token {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        private_key_file
            .write_string("private_key", WriteOptions::default())
            .await
            .unwrap();

        let mock_http_client = MockClient::default();

        let (token_mngr, _) = spawn(
            32,
            "device_id".to_string(),
            Arc::new(mock_http_client),
            token_file,
            private_key_file,
        )
        .unwrap();

        let token = token_mngr.get_token().await.unwrap();
        assert_eq!(token.as_ref(), &Token::default());
    }
}

pub mod refresh_token {
    use super::*;

    #[tokio::test]
    async fn invalid_private_key() {
        // prepare the arguments
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        private_key_file
            .write_string("private_key", WriteOptions::default())
            .await
            .unwrap();

        // prepare the mock http client
        let expected = TokenResponse {
            token: "token".to_string(),
            expires_at: Utc::now().to_rfc3339(),
        };
        let mock_http_client = MockClient {
            issue_device_token_fn: Box::new(move || Ok(expected.clone())),
            ..Default::default()
        };

        // spawn the token manager
        let (token_mngr, _) = spawn(
            32,
            "device_id".to_string(),
            Arc::new(mock_http_client),
            token_file,
            private_key_file,
        )
        .unwrap();

        // refresh the token
        let result = token_mngr.refresh_token().await.unwrap_err();
        assert!(matches!(result, AuthnErr::CryptErr(_)));
    }

    #[tokio::test]
    async fn http_client_error() {
        // prepare the arguments
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        // prepare the mock http client
        let mock_http_client = MockClient {
            issue_device_token_fn: Box::new(move || {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_connection_error: false,
                }))
            }),
            ..Default::default()
        };

        // spawn the token manager
        let (token_mngr, _) = spawn(
            32,
            "device_id".to_string(),
            Arc::new(mock_http_client),
            token_file,
            private_key_file,
        )
        .unwrap();

        // refresh the token
        let result = token_mngr.refresh_token().await.unwrap_err();
        assert!(matches!(result, AuthnErr::HTTPErr(_)));
    }

    #[tokio::test]
    async fn success() {
        // prepare the arguments
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        // prepare the mock http client
        let expires_at = Utc::now() + Duration::days(1);
        let resp = TokenResponse {
            token: "token".to_string(),
            expires_at: expires_at.to_rfc3339(),
        };
        let resp_clone = resp.clone();
        let mock_http_client = MockClient {
            issue_device_token_fn: Box::new(move || Ok(resp_clone.clone())),
            ..Default::default()
        };

        // spawn the token manager
        let (token_mngr, _) = spawn(
            32,
            "device_id".to_string(),
            Arc::new(mock_http_client),
            token_file,
            private_key_file,
        )
        .unwrap();

        // refresh the token
        token_mngr.refresh_token().await.unwrap();
        let token = token_mngr.get_token().await.unwrap();
        let expected = Token {
            token: resp.token,
            expires_at,
        };
        assert_eq!(token.as_ref(), &expected);
    }

    #[tokio::test]
    async fn success_token_file_deleted() {
        // prepare the arguments
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = dir.file("token.json");
        let cached_token_file = TokenFile::new_with_default(token_file.clone(), Token::default())
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();

        // prepare the mock http client
        let expires_at = Utc::now() + Duration::days(1);
        let resp = TokenResponse {
            token: "token".to_string(),
            expires_at: expires_at.to_rfc3339(),
        };
        let resp_clone = resp.clone();
        let mock_http_client = MockClient {
            issue_device_token_fn: Box::new(move || Ok(resp_clone.clone())),
            ..Default::default()
        };

        // spawn the token manager
        let (token_mngr, _) = spawn(
            32,
            "device_id".to_string(),
            Arc::new(mock_http_client),
            cached_token_file,
            private_key_file,
        )
        .unwrap();

        // delete the token file just because it should still work
        token_file.delete().await.unwrap();

        // refresh the token
        token_mngr.refresh_token().await.unwrap();
        let token = token_mngr.get_token().await.unwrap();
        let expected = Token {
            token: resp.token,
            expires_at,
        };
        assert_eq!(token.as_ref(), &expected);
    }
}
