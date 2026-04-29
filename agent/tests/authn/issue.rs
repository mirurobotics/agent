// internal crates
use crate::mocks::http_client::{Call, MockClient};
use backend_api::models::TokenResponse;
use miru_agent::authn::errors::AuthnErr;
use miru_agent::authn::issue::{encode_part, issue_token, mint_jwt};
use miru_agent::authn::Token;
use miru_agent::crypt::{base64, rsa};
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::MockErr;
use miru_agent::http::HTTPErr;

// external crates
use chrono::{Duration, Utc};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Verifier;
use serde::ser::Error as _;
use serde::{Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

/// Generate a real RSA key pair in a temp dir and return the file handles.
async fn generate_keys() -> (filesys::Dir, filesys::File, filesys::File) {
    let dir = filesys::Dir::create_temp_dir("authn_issue_test")
        .await
        .unwrap();
    let private_key_file = dir.file("private_key.pem");
    let public_key_file = dir.file("public_key.pem");
    rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
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
        let dir = filesys::Dir::create_temp_dir("authn_issue_test")
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        public_key_file.delete().await.unwrap();
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
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();

        assert_eq!(3, jwt.split('.').count());
    }

    #[tokio::test]
    async fn header_decodes_to_rs512_with_jwk() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let header_bytes = base64::decode_bytes_url_safe_no_pad(parts[0]).unwrap();
        let header: Value = serde_json::from_slice(&header_bytes).unwrap();

        assert_eq!("RS512", header["alg"]);
        assert_eq!("JWT", header["typ"]);
        assert_eq!("RSA", header["jwk"]["kty"]);
        assert!(!header["jwk"]["n"].as_str().unwrap().is_empty());
        assert!(!header["jwk"]["e"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn payload_decodes_with_jti_iat_exp() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let before = Utc::now().timestamp();
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
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
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature = base64::decode_bytes_url_safe_no_pad(parts[2]).unwrap();

        let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
        let pkey = PKey::from_rsa(public_key_rsa).unwrap();
        let mut verifier = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
        verifier.update(signing_input.as_bytes()).unwrap();
        assert!(verifier.verify(&signature).unwrap());

        let mut tampered = signing_input.clone().into_bytes();
        tampered[0] ^= 0x01;
        let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
        let pkey = PKey::from_rsa(public_key_rsa).unwrap();
        let mut verifier = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
        verifier.update(&tampered).unwrap();
        assert!(!verifier.verify(&signature).unwrap());
    }

    #[tokio::test]
    async fn generates_unique_jti_across_calls() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt_a = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let jwt_b = mint_jwt(&private_key_file, &public_key_file).await.unwrap();

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
        let dir = filesys::Dir::create_temp_dir("authn_issue_test")
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        public_key_file.delete().await.unwrap();

        let result = mint_jwt(&private_key_file, &public_key_file).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn returns_err_when_private_key_file_is_missing() {
        let dir = filesys::Dir::create_temp_dir("authn_issue_test")
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        private_key_file.delete().await.unwrap();

        let result = mint_jwt(&private_key_file, &public_key_file).await;

        assert!(result.is_err());
    }
}
