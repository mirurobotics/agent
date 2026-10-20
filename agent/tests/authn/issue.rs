// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models::TokenResponse;
use miru_agent::authn::errors::AuthnErr;
use miru_agent::authn::issue::{encode_part, issue_token, mint_jwt};
use miru_agent::authn::Token;
use miru_agent::crypt::{base64, rsa};
use miru_agent::filesys::{self, files, Overwrite};
use miru_agent::http::errors::MockErr;
use miru_agent::http::HTTPErr;

// external crates
use chrono::{Duration, Utc};
use serde::ser::Error as _;
use serde::{Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

/// Generate a real RSA key pair in a temp dir and return the file handles.
async fn generate_keys() -> (test_dirs::TempDir, filesys::File, filesys::File) {
    let dir = test_dirs::temp("authn_issue_test").unwrap();
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

mod encode_part {
    use super::*;

    /// A `Serialize` impl that always fails — used to exercise the
    /// `encode_part` error mapping path.
    struct AlwaysFails;
    impl Serialize for AlwaysFails {
        fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("intentional failure"))
        }
    }

    #[test]
    fn maps_serialize_failure_to_serde_err() {
        let result = encode_part(&AlwaysFails);
        assert!(matches!(result, Err(AuthnErr::SerdeErr(_))));
    }
}

mod issue_token {
    use super::*;

    #[tokio::test]
    async fn happy_path_returns_token_and_records_one_call() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let expires_at = Utc::now() + Duration::days(1);
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(move || {
                Ok(TokenResponse {
                    token: "issued-token".to_string(),
                    expires_at: expires_at.to_rfc3339(),
                })
            }),
            ..Default::default()
        };

        let token: Token = issue_token(&mock_client, &private_key_file, &public_key_file)
            .await
            .unwrap();

        assert_eq!(token.token, "issued-token");
        assert!((token.expires_at - expires_at).num_seconds().abs() <= 1);
        assert_eq!(mock_client.call_count(Call::IssueDeviceToken), 1);

        let requests = mock_client.requests();
        let captured = requests
            .iter()
            .find(|r| r.call == Call::IssueDeviceToken)
            .unwrap();
        let bearer = captured.token.as_ref().unwrap();
        assert_eq!(bearer.split('.').count(), 3);
    }

    #[tokio::test]
    async fn invalid_rfc3339_returns_timestamp_conversion_err() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(|| {
                Ok(TokenResponse {
                    token: "token".to_string(),
                    expires_at: "not-a-timestamp".to_string(),
                })
            }),
            ..Default::default()
        };

        let result = issue_token(&mock_client, &private_key_file, &public_key_file).await;

        assert!(matches!(result, Err(AuthnErr::TimestampConversionErr(_))));
        assert_eq!(mock_client.call_count(Call::IssueDeviceToken), 1);
    }

    #[tokio::test]
    async fn bubbles_http_err_from_backend() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let mock_client = MockClient {
            issue_device_token_fn: Box::new(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            }),
            ..Default::default()
        };

        let result = issue_token(&mock_client, &private_key_file, &public_key_file).await;

        assert!(matches!(result, Err(AuthnErr::HTTPErr(_))));
        assert_eq!(mock_client.call_count(Call::IssueDeviceToken), 1);
    }

    #[tokio::test]
    async fn bubbles_filesys_err_when_public_key_missing() {
        let dir = test_dirs::temp("authn_issue_test").unwrap();
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
        files::delete(&public_key_file).await.unwrap();
        let mock_client = MockClient::default();

        let result = issue_token(&mock_client, &private_key_file, &public_key_file).await;

        assert!(result.is_err());
        assert_eq!(mock_client.call_count(Call::IssueDeviceToken), 0);
    }
}

mod mint_jwt {
    use super::*;

    #[tokio::test]
    async fn has_three_parts() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file, Utc::now())
            .await
            .unwrap();

        assert_eq!(3, jwt.split('.').count());
    }

    #[tokio::test]
    async fn header_decodes_to_rs512_with_kid() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file, Utc::now())
            .await
            .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let header_bytes = base64::decode_bytes_url_safe_no_pad(parts[0]).unwrap();
        let header: Value = serde_json::from_slice(&header_bytes).unwrap();

        assert_eq!("RS512", header["alg"]);
        assert_eq!("JWT", header["typ"]);

        // kid is the SHA-256 of DER PKIX SPKI bytes as 64-char lowercase hex.
        // The backend uses this to look up the enrolled device.
        let kid = header["kid"].as_str().unwrap();
        let public_key = rsa::read_public_key(&public_key_file).await.unwrap();
        let expected_kid = rsa::fingerprint(&public_key).unwrap();
        assert_eq!(kid, expected_kid);
        assert_eq!(kid.len(), 64);
    }

    #[tokio::test]
    async fn payload_decodes_with_jti_iat_exp() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let before = Utc::now().timestamp();
        let jwt = mint_jwt(&private_key_file, &public_key_file, Utc::now())
            .await
            .unwrap();
        let after = Utc::now().timestamp();
        let parts: Vec<&str> = jwt.split('.').collect();
        let payload_bytes = base64::decode_bytes_url_safe_no_pad(parts[1]).unwrap();
        let payload: Value = serde_json::from_slice(&payload_bytes).unwrap();

        let jti = payload["jti"].as_str().unwrap();
        assert!(!jti.is_empty());
        Uuid::parse_str(jti).unwrap();

        let iat = payload["iat"].as_i64().unwrap();
        assert!(
            iat >= before && iat <= after,
            "iat={iat} outside [{before}, {after}]"
        );

        let exp = payload["exp"].as_i64().unwrap();
        assert_eq!(iat + 120, exp);
    }

    #[tokio::test]
    async fn signature_verifies_with_public_key() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file, Utc::now())
            .await
            .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature = base64::decode_bytes_url_safe_no_pad(parts[2]).unwrap();

        assert!(
            rsa::verify_rs512(&public_key_file, signing_input.as_bytes(), &signature,)
                .await
                .unwrap()
        );

        let mut tampered = signing_input.into_bytes();
        tampered[0] ^= 0x01;
        assert!(!rsa::verify_rs512(&public_key_file, &tampered, &signature)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn generates_unique_jti_across_calls() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt_a = mint_jwt(&private_key_file, &public_key_file, Utc::now())
            .await
            .unwrap();
        let jwt_b = mint_jwt(&private_key_file, &public_key_file, Utc::now())
            .await
            .unwrap();

        let parts_a: Vec<&str> = jwt_a.split('.').collect();
        let parts_b: Vec<&str> = jwt_b.split('.').collect();
        let payload_a: Value =
            serde_json::from_slice(&base64::decode_bytes_url_safe_no_pad(parts_a[1]).unwrap())
                .unwrap();
        let payload_b: Value =
            serde_json::from_slice(&base64::decode_bytes_url_safe_no_pad(parts_b[1]).unwrap())
                .unwrap();

        assert_ne!(payload_a["jti"], payload_b["jti"]);
    }

    #[tokio::test]
    async fn returns_err_when_public_key_file_is_missing() {
        let dir = test_dirs::temp("authn_issue_test").unwrap();
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
        files::delete(&public_key_file).await.unwrap();

        let result = mint_jwt(&private_key_file, &public_key_file, Utc::now()).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn returns_err_when_private_key_file_is_missing() {
        let dir = test_dirs::temp("authn_issue_test").unwrap();
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
        files::delete(&private_key_file).await.unwrap();

        let result = mint_jwt(&private_key_file, &public_key_file, Utc::now()).await;

        assert!(result.is_err());
    }
}

mod wrong_system_clock {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::mocks::http_client as mock;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::{Json, Router};
    use chrono::DateTime;
    use miru_agent::clock::Clock;
    use miru_agent::http;

    /// A backend that, like the real one, rejects a JWT whose `exp` has passed
    /// by its own clock.
    fn backend(calls: Arc<AtomicUsize>) -> Router {
        backend_with(calls, false)
    }

    fn backend_with(calls: Arc<AtomicUsize>, reject_all: bool) -> Router {
        Router::new().route(
            "/devices/token",
            post(move |headers: HeaderMap| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let bearer = headers["authorization"].to_str().unwrap();
                    let jwt = bearer.trim_start_matches("Bearer ");
                    let payload = jwt.split('.').nth(1).unwrap();
                    let payload: Value = serde_json::from_slice(
                        &base64::decode_bytes_url_safe_no_pad(payload).unwrap(),
                    )
                    .unwrap();
                    if reject_all || payload["exp"].as_i64().unwrap() < Utc::now().timestamp() {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Ok(Json(TokenResponse {
                        token: "issued-token".to_string(),
                        expires_at: (Utc::now() + Duration::days(1)).to_rfc3339(),
                    }))
                }
            }),
        )
    }

    fn client_with_system_clock(base_url: &str, system: DateTime<Utc>) -> http::Client {
        let clock =
            Clock::from_sources(Arc::new(move || system), Arc::new(std::time::Instant::now));
        http::Client::with_clock(base_url, clock).unwrap()
    }

    #[tokio::test]
    async fn system_clock_at_1970_retries_once_with_server_time() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let server = mock::run_server(backend(calls.clone())).await;
        let client = client_with_system_clock(&server.base_url, DateTime::UNIX_EPOCH);

        let token = issue_token(&client, &private_key_file, &public_key_file)
            .await
            .unwrap();

        assert_eq!(token.token, "issued-token");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_second_rejection_is_returned_without_further_retries() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let server = mock::run_server(backend_with(calls.clone(), true)).await;
        let client = client_with_system_clock(&server.base_url, DateTime::UNIX_EPOCH);

        let result = issue_token(&client, &private_key_file, &public_key_file).await;

        assert!(matches!(result, Err(AuthnErr::HTTPErr(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn known_server_time_signs_correctly_the_first_time() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let server = mock::run_server(backend(calls.clone())).await;
        let client = client_with_system_clock(&server.base_url, DateTime::UNIX_EPOCH);
        client.clock().observe_server_time(Utc::now());

        issue_token(&client, &private_key_file, &public_key_file)
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_rejection_with_server_time_already_known_is_not_retried() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let server = mock::run_server(backend(calls.clone())).await;
        let client = client_with_system_clock(&server.base_url, DateTime::UNIX_EPOCH);
        // a stale reading: server time is known but a day behind
        client
            .clock()
            .observe_server_time(Utc::now() - Duration::days(1));

        let result = issue_token(&client, &private_key_file, &public_key_file).await;

        assert!(matches!(result, Err(AuthnErr::HTTPErr(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mint_jwt_signs_with_the_given_time() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let now = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let jwt = mint_jwt(&private_key_file, &public_key_file, now)
            .await
            .unwrap();
        let payload = jwt.split('.').nth(1).unwrap();
        let payload: Value =
            serde_json::from_slice(&base64::decode_bytes_url_safe_no_pad(payload).unwrap())
                .unwrap();
        assert_eq!(payload["iat"].as_i64(), Some(now.timestamp()));
        assert_eq!(payload["exp"].as_i64(), Some(now.timestamp() + 120));
    }
}
