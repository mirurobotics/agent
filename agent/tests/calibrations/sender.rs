//! Mapping a buffered report to Miru's `POST /calibrations` and sending it.

// standard crates
use std::sync::Arc;

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::mocks::stub_token_manager::StubTokenManager;
use crate::test_utils::calibrations::{received_at, report};
use backend_api::models::{Calibration, InstanceContent, InstanceFormat as BackendFormat};
use miru_agent::authn::{errors::MockError as AuthnMockErr, AuthnErr};
use miru_agent::calibrations::{sender::new_request, CalibrationsErr, HttpSender, Report, Sender};
use miru_agent::errors::Error;
use miru_agent::http::errors::{HTTPErr, MockErr, RequestFailed, UnmarshalJSONErr};
use miru_agent::http::request::Params;
use miru_agent::models::InstanceFormat;

// external crates
use serde_json::Value;

fn full_report() -> Report {
    let mut report = report("arm");
    report.metadata.role = Some("left".to_string());
    report.metadata.aspect = Some("pose".to_string());
    report.metadata.serial_number = Some("SN-1".to_string());
    report.metadata.deployment_id = Some("dpl_1".to_string());
    report
}

fn request_failed(status: u16) -> HTTPErr {
    HTTPErr::RequestFailed(RequestFailed {
        request: Params::get("http://mock/calibrations").meta().unwrap(),
        status: reqwest::StatusCode::from_u16(status).unwrap(),
        error: None,
        trace: miru_agent::trace!(),
    })
}

fn sender(
    client: &Arc<MockClient>,
    tokens: StubTokenManager,
) -> HttpSender<MockClient, StubTokenManager> {
    HttpSender::new(client.clone(), Arc::new(tokens))
}

pub mod new_request {
    use super::*;

    #[test]
    fn maps_the_metadata() {
        let request = new_request(full_report());

        assert_eq!(request.component, "arm");
        assert_eq!(
            (request.role, request.aspect),
            (Some("left".to_string()), Some("pose".to_string()))
        );
        assert_eq!(
            (request.serial_number, request.deployment_id),
            (Some("SN-1".to_string()), Some("dpl_1".to_string()))
        );
        assert_eq!(request.reported_at, received_at().to_rfc3339());
        assert_eq!(
            *request.content,
            InstanceContent {
                format: BackendFormat::INSTANCE_FORMAT_JSON,
                data: r#"{"offset":[1,2,3]}"#.to_string(),
            }
        );
    }

    #[test]
    fn omits_absent_fields() {
        let request = new_request(report("arm"));

        let value = serde_json::to_value(&request).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["component", "content", "reported_at"]);
    }

    #[test]
    fn sends_the_data_as_given_in_its_format() {
        let mut report = report("arm");
        report.metadata.format = InstanceFormat::Yaml;
        report.data = "offset: [1, 2, 3]\n".to_string();

        let request = new_request(report);

        assert_eq!(
            *request.content,
            InstanceContent {
                format: BackendFormat::INSTANCE_FORMAT_YAML,
                data: "offset: [1, 2, 3]\n".to_string(),
            }
        );
    }
}

pub mod send {
    use super::*;

    #[tokio::test]
    async fn posts_one_request_with_the_token() {
        let client = Arc::new(MockClient::default());

        sender(&client, StubTokenManager::ok("tok"))
            .send(report("arm"))
            .await
            .unwrap();

        let requests = client.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].call, Call::CreateCalibration);
        assert_eq!(requests[0].token.as_deref(), Some("tok"));
        let body: Value = serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["reported_at"], received_at().to_rfc3339());
        assert_eq!(body["component"], "arm");
    }

    #[tokio::test]
    async fn passes_a_422_through_as_terminal() {
        let client = Arc::new(MockClient::default());
        client.set_create_calibration(|| Err(request_failed(422)));

        let err = sender(&client, StubTokenManager::ok("tok"))
            .send(report("arm"))
            .await
            .unwrap_err();

        assert!(matches!(err, CalibrationsErr::HTTPErr(_)), "{err:?}");
        assert!(err.is_terminal());
        assert_eq!(client.call_count(Call::CreateCalibration), 1);
    }

    #[tokio::test]
    async fn an_accepted_report_with_an_unparseable_response_is_sent() {
        // a response body is parsed only after a 2xx, so Miru stored the
        // report; resending it would store it again
        let client = Arc::new(MockClient::default());
        client.set_create_calibration(|| {
            Err(HTTPErr::UnmarshalJSONErr(UnmarshalJSONErr {
                request: Params::get("http://mock/calibrations").meta().unwrap(),
                source: serde_json::from_str::<Calibration>("{}").unwrap_err(),
                trace: miru_agent::trace!(),
            }))
        });

        sender(&client, StubTokenManager::ok("tok"))
            .send(report("arm"))
            .await
            .unwrap();

        assert_eq!(client.call_count(Call::CreateCalibration), 1);
    }

    #[tokio::test]
    async fn reports_a_network_error() {
        let client = Arc::new(MockClient::default());
        client.set_create_calibration(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        });

        let err = sender(&client, StubTokenManager::ok("tok"))
            .send(report("arm"))
            .await
            .unwrap_err();

        assert!(err.is_network_conn_err());
        assert!(!err.is_terminal());
        assert_eq!(client.call_count(Call::CreateCalibration), 1);
    }

    #[tokio::test]
    async fn returns_a_token_error() {
        let client = Arc::new(MockClient::default());
        let tokens = StubTokenManager::err(AuthnErr::MockError(AuthnMockErr {
            is_network_conn_err: true,
            trace: miru_agent::trace!(),
        }));

        let err = sender(&client, tokens)
            .send(report("arm"))
            .await
            .unwrap_err();

        assert!(matches!(err, CalibrationsErr::TokenErr(_)), "{err:?}");
        assert!(err.is_network_conn_err());
        assert!(!err.is_terminal());
        assert!(
            err.to_string().starts_with("failed to get a token"),
            "{err}"
        );
        assert!(client.requests().is_empty());
    }

    #[tokio::test]
    async fn a_terminal_token_error_is_retryable() {
        // a token request Miru rejects says nothing about the report, which
        // must be retried rather than parked
        let client = Arc::new(MockClient::default());
        let tokens = StubTokenManager::err(AuthnErr::HTTPErr(request_failed(403)));

        let err = sender(&client, tokens)
            .send(report("arm"))
            .await
            .unwrap_err();

        assert!(!err.is_terminal());
        assert!(!err.is_network_conn_err());
        assert!(client.requests().is_empty());
    }
}
