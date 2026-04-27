// internal crates
use crate::mocks::http_client::{Call, CapturedRequest, MockClient};
use backend_api::models::{
    Device, ProvisionDeviceRequest, TokenResponse, UpdateDeviceFromAgentRequest,
};
use miru_agent::http::devices::{self, IssueTokenParams, ProvisionParams, UpdateParams};
use miru_agent::http::errors::MockErr;
use miru_agent::http::HTTPErr;

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

pub mod provision {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let payload = ProvisionDeviceRequest {
            public_key_pem: "test-pem".to_string(),
            agent_version: "v0.0.0".to_string(),
            ..ProvisionDeviceRequest::default()
        };
        let expected_body = serde_json::to_string(&payload).unwrap();

        let result = devices::provision(
            &mock,
            ProvisionParams {
                payload: &payload,
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, Device::default());
        assert_eq!(mock.call_count(Call::ProvisionDevice), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ProvisionDevice,
                method: reqwest::Method::POST,
                path: "/devices/provision".into(),
                url: "http://mock/devices/provision".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            provision_device_fn: Box::new(|| Err(mock_err())),
            ..MockClient::default()
        };

        let payload = ProvisionDeviceRequest::default();
        let result = devices::provision(
            &mock,
            ProvisionParams {
                payload: &payload,
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod issue_token {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let result = devices::issue_token(
            &mock,
            IssueTokenParams {
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, TokenResponse::default());
        assert_eq!(mock.call_count(Call::IssueDeviceToken), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::IssueDeviceToken,
                method: reqwest::Method::POST,
                path: "/devices/issue_token".into(),
                url: "http://mock/devices/issue_token".into(),
                query: vec![],
                body: Some(String::new()),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            issue_device_token_fn: Box::new(|| Err(mock_err())),
            ..MockClient::default()
        };

        let result = devices::issue_token(
            &mock,
            IssueTokenParams {
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

        let payload = UpdateDeviceFromAgentRequest {
            agent_version: Some("1.2.3".to_string()),
        };
        let expected_body = serde_json::to_string(&payload).unwrap();

        let result = devices::update(
            &mock,
            UpdateParams {
                id: "dvc_1",
                payload: &payload,
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, Device::default());
        assert_eq!(mock.call_count(Call::UpdateDevice), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::UpdateDevice,
                method: reqwest::Method::PATCH,
                path: "/devices/dvc_1".into(),
                url: "http://mock/devices/dvc_1".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            update_device_fn: std::sync::Mutex::new(Box::new(|| Err(mock_err()))),
            ..MockClient::default()
        };

        let payload = UpdateDeviceFromAgentRequest::default();
        let result = devices::update(
            &mock,
            UpdateParams {
                id: "dvc_1",
                payload: &payload,
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}
