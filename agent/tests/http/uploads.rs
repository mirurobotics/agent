// internal crates
use crate::mocks::http_client::{Call, CapturedRequest, MockClient};
use backend_api::models::{
    CreateUploadRequest, MultipartUpload, UpdateUploadRequest, Upload, UploadCredentials,
    UploadWithCredentials,
};
use miru_agent::http::errors::MockErr;
use miru_agent::http::uploads::{
    self, ConfirmParams, CreateParams, UpdateParams, VendCredentialsParams,
};
use miru_agent::http::HTTPErr;

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

pub mod create {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let payload = CreateUploadRequest {
            file_rule_id: "file_rule_1".to_string(),
            digest: "sha256:abc123".to_string(),
            size: 42,
            ..CreateUploadRequest::default()
        };
        let expected_body = serde_json::to_string(&payload).unwrap();

        let result = uploads::create(
            &mock,
            CreateParams {
                payload: &payload,
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, UploadWithCredentials::default());
        assert_eq!(mock.call_count(Call::CreateUpload), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::CreateUpload,
                method: reqwest::Method::POST,
                path: "/uploads".into(),
                url: "http://mock/uploads".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            create_upload_fn: std::sync::Mutex::new(Box::new(|| Err(mock_err()))),
            ..MockClient::default()
        };

        let payload = CreateUploadRequest::default();
        let result = uploads::create(
            &mock,
            CreateParams {
                payload: &payload,
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod update {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let payload = UpdateUploadRequest::new(MultipartUpload {
            id: "mpu_1".to_string(),
            part_size: 8 * 1024 * 1024,
        });
        let expected_body = serde_json::to_string(&payload).unwrap();

        let result = uploads::update(
            &mock,
            UpdateParams {
                id: "upl_1",
                payload: &payload,
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, Upload::default());
        assert_eq!(mock.call_count(Call::UpdateUpload), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::UpdateUpload,
                method: reqwest::Method::PATCH,
                path: "/uploads/upl_1".into(),
                url: "http://mock/uploads/upl_1".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            update_upload_fn: std::sync::Mutex::new(Box::new(|| Err(mock_err()))),
            ..MockClient::default()
        };

        let payload = UpdateUploadRequest::default();
        let result = uploads::update(
            &mock,
            UpdateParams {
                id: "upl_1",
                payload: &payload,
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod vend_credentials {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let result = uploads::vend_credentials(
            &mock,
            VendCredentialsParams {
                id: "upl_1",
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, UploadCredentials::default());
        assert_eq!(mock.call_count(Call::VendUploadCredentials), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::VendUploadCredentials,
                method: reqwest::Method::POST,
                path: "/uploads/upl_1/credentials".into(),
                url: "http://mock/uploads/upl_1/credentials".into(),
                query: vec![],
                body: Some(String::new()),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            vend_upload_credentials_fn: std::sync::Mutex::new(Box::new(|| Err(mock_err()))),
            ..MockClient::default()
        };

        let result = uploads::vend_credentials(
            &mock,
            VendCredentialsParams {
                id: "upl_1",
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod confirm {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let result = uploads::confirm(
            &mock,
            ConfirmParams {
                id: "upl_1",
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, Upload::default());
        assert_eq!(mock.call_count(Call::ConfirmUpload), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ConfirmUpload,
                method: reqwest::Method::POST,
                path: "/uploads/upl_1/confirm".into(),
                url: "http://mock/uploads/upl_1/confirm".into(),
                query: vec![],
                body: Some(String::new()),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            confirm_upload_fn: std::sync::Mutex::new(Box::new(|| Err(mock_err()))),
            ..MockClient::default()
        };

        let result = uploads::confirm(
            &mock,
            ConfirmParams {
                id: "upl_1",
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}
