use crate::http::mock::{CapturedRequest, MockCall, MockClient};
use miru_agent::http::devices::{self, ActivateParams, IssueTokenParams, UpdateParams};
use miru_agent::http::errors::{HTTPErr, MockErr};
use openapi_client::models::{
    ActivateDeviceRequest, Device, IssueDeviceTokenRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_connection_error: false,
    })
}

pub mod activate {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();

        let payload = ActivateDeviceRequest {
            public_key_pem: "test-pem".to_string(),
            ..ActivateDeviceRequest::default()
        };
        let expected_body = serde_json::to_string(&payload).unwrap();

        let result = devices::activate(
            &mock,
            ActivateParams {
                id: "dvc_1",
                payload: &payload,
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, Device::default());
        assert_eq!(mock.call_count(MockCall::ActivateDevice), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::POST,
                url: "http://mock/devices/dvc_1/activate".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            activate_device_fn: Box::new(|| Err(mock_err())),
            ..MockClient::default()
        };

        let payload = ActivateDeviceRequest::default();
        let result = devices::activate(
            &mock,
            ActivateParams {
                id: "dvc_1",
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

        let payload = IssueDeviceTokenRequest::default();
        let expected_body = serde_json::to_string(&payload).unwrap();

        let result = devices::issue_token(
            &mock,
            IssueTokenParams {
                id: "dvc_1",
                payload: &payload,
            },
        )
        .await
        .unwrap();

        assert_eq!(result, TokenResponse::default());
        assert_eq!(mock.call_count(MockCall::IssueDeviceToken), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::POST,
                url: "http://mock/devices/dvc_1/issue_token".into(),
                query: vec![],
                body: Some(expected_body),
                token: None,
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            issue_device_token_fn: Box::new(|| Err(mock_err())),
            ..MockClient::default()
        };

        let payload = IssueDeviceTokenRequest::default();
        let result = devices::issue_token(
            &mock,
            IssueTokenParams {
                id: "dvc_1",
                payload: &payload,
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
        assert_eq!(mock.call_count(MockCall::UpdateDevice), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::PATCH,
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
            update_device_fn: Box::new(|| Err(mock_err())),
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
