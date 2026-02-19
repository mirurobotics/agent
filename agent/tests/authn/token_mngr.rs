// standard library
use std::sync::Arc;

// internal crates
use crate::http::mock::MockClient;
use miru_agent::authn::{
    errors::AuthnErr,
    token::Token,
    token_mngr::{TokenFile, TokenManager, TokenManagerExt},
};
use miru_agent::crypt::rsa;
use miru_agent::filesys::{dir::Dir, Overwrite, WriteOptions};
use miru_agent::http;
use miru_agent::http::errors::{HTTPErr, MockErr};
use openapi_client::models::TokenResponse;

// external crates
use chrono::{Duration, Utc};
use tokio::task::JoinHandle;

/// Setup a TokenManager with a dummy private key (for tests that don't reach RSA signing).
async fn setup(mock_client: MockClient) -> (Dir, TokenManager, JoinHandle<()>) {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
        .await
        .unwrap();
    let private_key_file = dir.file("private_key.pem");
    private_key_file
        .write_string("private_key", WriteOptions::default())
        .await
        .unwrap();
    let (token_mngr, worker_handle) = TokenManager::spawn(
        32,
        "device_id".to_string(),
        Arc::new(mock_client),
        token_file,
        private_key_file,
    )
    .unwrap();
    (dir, token_mngr, worker_handle)
}

/// Setup a TokenManager with a real RSA key pair (for tests that exercise token refresh/signing).
async fn setup_with_rsa(mock_client: MockClient) -> (Dir, TokenManager, JoinHandle<()>) {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
        .await
        .unwrap();
    let private_key_file = dir.file("private_key.pem");
    let public_key_file = dir.file("public_key.pem");
    rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow)
        .await
        .unwrap();
    let (token_mngr, worker_handle) = TokenManager::spawn(
        32,
        "device_id".to_string(),
        Arc::new(mock_client),
        token_file,
        private_key_file,
    )
    .unwrap();
    (dir, token_mngr, worker_handle)
}

pub mod spawn {
    use super::*;

    #[tokio::test]
    async fn token_file_does_not_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();
        token_file.file.delete().await.unwrap();
        let private_key_file = dir.file("private_key.pem");
        private_key_file
            .write_string("private_key", WriteOptions::default())
            .await
            .unwrap();

        let http_client = http::Client::new("doesntmatter").unwrap();
        let result = TokenManager::spawn(
            32,
            "device_id".to_string(),
            Arc::new(http_client),
            token_file,
            private_key_file,
        )
        .unwrap_err();
        assert!(matches!(result, AuthnErr::FileSysErr(_)));
    }

    #[tokio::test]
    async fn private_key_file_does_not_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
            .await
            .unwrap();

        let http_client = http::Client::new("doesntmatter").unwrap();
        let result = TokenManager::spawn(
            32,
            "device_id".to_string(),
            Arc::new(http_client),
            token_file,
            dir.file("private_key.pem"),
        )
        .unwrap_err();
        assert!(matches!(result, AuthnErr::FileSysErr(_)));
    }

    #[tokio::test]
    async fn success() {
        let (_dir, token_mngr, worker_handle) = setup_with_rsa(MockClient::default()).await;
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn shutdown() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn after_shutdown() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();

        let result = token_mngr.shutdown().await;
        assert!(matches!(result, Err(AuthnErr::SendActorMessageErr(_))));
    }
}

pub mod get_token {
    use super::*;

    #[tokio::test]
    async fn success() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        let token = token_mngr.get_token().await.unwrap();
        assert_eq!(token.as_ref(), &Token::default());
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn after_shutdown() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();

        let result = token_mngr.get_token().await;
        assert!(matches!(result, Err(AuthnErr::SendActorMessageErr(_))));
    }
}

pub mod refresh_token {
    use super::*;

    #[tokio::test]
    async fn after_shutdown() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();

        let result = token_mngr.refresh_token().await;
        assert!(matches!(result, Err(AuthnErr::SendActorMessageErr(_))));
    }

    #[tokio::test]
    async fn invalid_timestamp() {
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(|| {
                Ok(TokenResponse {
                    token: "token".to_string(),
                    expires_at: "not-a-valid-timestamp".to_string(),
                })
            }),
            ..Default::default()
        };
        let (_dir, token_mngr, worker_handle) = setup_with_rsa(mock_client).await;

        let result = token_mngr.refresh_token().await.unwrap_err();
        assert!(matches!(result, AuthnErr::TimestampConversionErr(_)));
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_private_key() {
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(|| {
                Ok(TokenResponse {
                    token: "token".to_string(),
                    expires_at: Utc::now().to_rfc3339(),
                })
            }),
            ..Default::default()
        };
        // uses dummy key — signing will fail
        let (_dir, token_mngr, worker_handle) = setup(mock_client).await;

        let result = token_mngr.refresh_token().await.unwrap_err();
        assert!(matches!(result, AuthnErr::CryptErr(_)));
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn http_client_error() {
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_connection_error: false,
                }))
            }),
            ..Default::default()
        };
        let (_dir, token_mngr, worker_handle) = setup_with_rsa(mock_client).await;

        let result = token_mngr.refresh_token().await.unwrap_err();
        assert!(matches!(result, AuthnErr::HTTPErr(_)));
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn success() {
        let expires_at = Utc::now() + Duration::days(1);
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(move || {
                Ok(TokenResponse {
                    token: "token".to_string(),
                    expires_at: expires_at.to_rfc3339(),
                })
            }),
            ..Default::default()
        };
        let (_dir, token_mngr, worker_handle) = setup_with_rsa(mock_client).await;

        token_mngr.refresh_token().await.unwrap();
        let token = token_mngr.get_token().await.unwrap();
        assert_eq!(token.token, "token");
        assert_eq!(token.expires_at, expires_at);
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn success_token_file_deleted() {
        let expires_at = Utc::now() + Duration::days(1);
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(move || {
                Ok(TokenResponse {
                    token: "token".to_string(),
                    expires_at: expires_at.to_rfc3339(),
                })
            }),
            ..Default::default()
        };
        let (dir, token_mngr, worker_handle) = setup_with_rsa(mock_client).await;

        // delete the token file — refresh should still work (cached in memory)
        dir.file("token.json").delete().await.unwrap();

        token_mngr.refresh_token().await.unwrap();
        let token = token_mngr.get_token().await.unwrap();
        assert_eq!(token.token, "token");
        assert_eq!(token.expires_at, expires_at);
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn sequential_refreshes_update_token() {
        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let call_count_clone = call_count.clone();
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(move || {
                let n = call_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(TokenResponse {
                    token: format!("token-{n}"),
                    expires_at: (Utc::now() + Duration::days(1)).to_rfc3339(),
                })
            }),
            ..Default::default()
        };
        let (_dir, token_mngr, worker_handle) = setup_with_rsa(mock_client).await;

        token_mngr.refresh_token().await.unwrap();
        let token = token_mngr.get_token().await.unwrap();
        assert_eq!(token.token, "token-0");

        token_mngr.refresh_token().await.unwrap();
        let token = token_mngr.get_token().await.unwrap();
        assert_eq!(token.token, "token-1");
        token_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }
}

pub mod arc_delegation {
    use super::*;

    #[tokio::test]
    async fn get_token_via_arc() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        let arc_mngr = Arc::new(token_mngr);
        let token = arc_mngr.get_token().await.unwrap();
        assert_eq!(token.as_ref(), &Token::default());
        arc_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn refresh_token_via_arc() {
        let expires_at = Utc::now() + Duration::days(1);
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(move || {
                Ok(TokenResponse {
                    token: "arc-token".to_string(),
                    expires_at: expires_at.to_rfc3339(),
                })
            }),
            ..Default::default()
        };
        let (_dir, token_mngr, worker_handle) = setup_with_rsa(mock_client).await;
        let arc_mngr = Arc::new(token_mngr);
        arc_mngr.refresh_token().await.unwrap();
        let token = arc_mngr.get_token().await.unwrap();
        assert_eq!(token.token, "arc-token");
        arc_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_via_arc() {
        let (_dir, token_mngr, worker_handle) = setup(MockClient::default()).await;
        let arc_mngr = Arc::new(token_mngr);
        arc_mngr.shutdown().await.unwrap();
        worker_handle.await.unwrap();
    }
}
