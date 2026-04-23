// internal crates
use crate::mocks::http_client::{Call, CapturedRequest, MockClient};
use backend_api::models::{
    ActivateDeviceRequest, Device, IssueDeviceTokenRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};
use miru_agent::http::devices::{
    self, ActivateParams, CreateOrFetchDeviceParams, IssueActivationTokenParams, IssueTokenParams,
    UpdateParams,
};
use miru_agent::http::errors::{MockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::http::HTTPErr;

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
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
        assert_eq!(mock.call_count(Call::ActivateDevice), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ActivateDevice,
                method: reqwest::Method::POST,
                path: "/devices/dvc_1/activate".into(),
                url: "http://mock/devices/dvc_1/activate".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
                api_key: None,
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
        assert_eq!(mock.call_count(Call::IssueDeviceToken), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::IssueDeviceToken,
                method: reqwest::Method::POST,
                path: "/devices/dvc_1/issue_token".into(),
                url: "http://mock/devices/dvc_1/issue_token".into(),
                query: vec![],
                body: Some(expected_body),
                token: None,
                api_key: None,
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
                api_key: None,
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

pub mod create_or_fetch_device {
    use super::*;

    fn conflict_err() -> HTTPErr {
        HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::post("http://mock/devices", String::new())
                .meta()
                .unwrap(),
            status: reqwest::StatusCode::CONFLICT,
            error: None,
            trace: miru_agent::trace!(),
        })
    }

    fn server_err() -> HTTPErr {
        HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::post("http://mock/devices", String::new())
                .meta()
                .unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        })
    }

    #[tokio::test]
    async fn happy_path_returns_device_from_post() {
        let mock = MockClient::default();
        mock.set_create_or_fetch_device(|| {
            Ok(Device {
                id: "dvc_new".into(),
                ..Device::default()
            })
        });

        let result = devices::create_or_fetch_device(
            &mock,
            CreateOrFetchDeviceParams {
                name: "host-foo",
                api_key: "secret-key",
            },
        )
        .await
        .unwrap();

        assert_eq!(result.id, "dvc_new");
        assert_eq!(mock.call_count(Call::CreateDevice), 1);
        assert_eq!(mock.call_count(Call::FetchDeviceByName), 0);
        let expected_body =
            serde_json::to_string(&serde_json::json!({"name": "host-foo"})).unwrap();
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::CreateDevice,
                method: reqwest::Method::POST,
                path: "/devices".into(),
                url: "http://mock/devices".into(),
                query: vec![],
                body: Some(expected_body),
                token: None,
                api_key: Some("secret-key".into()),
            }]
        );
    }

    #[tokio::test]
    async fn conflict_falls_back_to_get_with_name_query() {
        let mock = MockClient::default();
        let counter = std::sync::atomic::AtomicUsize::new(0);
        mock.set_create_or_fetch_device(move || {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Err(conflict_err())
            } else {
                Ok(Device {
                    id: "dvc_existing".into(),
                    ..Device::default()
                })
            }
        });

        let result = devices::create_or_fetch_device(
            &mock,
            CreateOrFetchDeviceParams {
                name: "host-foo",
                api_key: "secret-key",
            },
        )
        .await
        .unwrap();

        assert_eq!(result.id, "dvc_existing");
        assert_eq!(mock.call_count(Call::CreateDevice), 1);
        assert_eq!(mock.call_count(Call::FetchDeviceByName), 1);

        let requests = mock.requests();
        let expected_body =
            serde_json::to_string(&serde_json::json!({"name": "host-foo"})).unwrap();
        assert_eq!(
            requests[0],
            CapturedRequest {
                call: Call::CreateDevice,
                method: reqwest::Method::POST,
                path: "/devices".into(),
                url: "http://mock/devices".into(),
                query: vec![],
                body: Some(expected_body),
                token: None,
                api_key: Some("secret-key".into()),
            }
        );
        assert_eq!(
            requests[1],
            CapturedRequest {
                call: Call::FetchDeviceByName,
                method: reqwest::Method::GET,
                path: "/devices".into(),
                url: "http://mock/devices".into(),
                query: vec![("name".into(), "host-foo".into())],
                body: None,
                token: None,
                api_key: Some("secret-key".into()),
            }
        );
    }

    #[tokio::test]
    async fn non_conflict_error_propagates() {
        let mock = MockClient::default();
        mock.set_create_or_fetch_device(|| Err(server_err()));

        let result = devices::create_or_fetch_device(
            &mock,
            CreateOrFetchDeviceParams {
                name: "host-foo",
                api_key: "secret-key",
            },
        )
        .await;

        match result {
            Err(HTTPErr::RequestFailed(rf)) => {
                assert_eq!(rf.status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
            }
            other => panic!("expected RequestFailed(500), got {other:?}"),
        }
        // Only the POST attempt — no GET fallback for non-409 errors.
        assert_eq!(mock.call_count(Call::CreateDevice), 1);
        assert_eq!(mock.call_count(Call::FetchDeviceByName), 0);
    }
}

pub mod issue_activation_token {
    use super::*;

    #[tokio::test]
    async fn happy_path_returns_token() {
        let mock = MockClient::default();
        mock.set_issue_activation_token(|| {
            Ok(TokenResponse {
                token: "test-jwt".into(),
                ..TokenResponse::default()
            })
        });

        let result = devices::issue_activation_token(
            &mock,
            IssueActivationTokenParams {
                id: "dvc_1",
                api_key: "secret-key",
                allow_reactivation: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.token, "test-jwt");
        assert_eq!(mock.call_count(Call::IssueActivationToken), 1);
        let expected_body =
            serde_json::to_string(&serde_json::json!({"allow_reactivation": false})).unwrap();
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::IssueActivationToken,
                method: reqwest::Method::POST,
                path: "/devices/dvc_1/activation_token".into(),
                url: "http://mock/devices/dvc_1/activation_token".into(),
                query: vec![],
                body: Some(expected_body),
                token: None,
                api_key: Some("secret-key".into()),
            }]
        );
    }

    #[tokio::test]
    async fn allow_reactivation_true_serializes_correctly() {
        let mock = MockClient::default();

        devices::issue_activation_token(
            &mock,
            IssueActivationTokenParams {
                id: "dvc_1",
                api_key: "secret-key",
                allow_reactivation: true,
            },
        )
        .await
        .unwrap();

        let requests = mock.requests();
        let expected_body =
            serde_json::to_string(&serde_json::json!({"allow_reactivation": true})).unwrap();
        assert_eq!(requests[0].body, Some(expected_body));
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_issue_activation_token(|| Err(mock_err()));

        let result = devices::issue_activation_token(
            &mock,
            IssueActivationTokenParams {
                id: "dvc_1",
                api_key: "secret-key",
                allow_reactivation: false,
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}
