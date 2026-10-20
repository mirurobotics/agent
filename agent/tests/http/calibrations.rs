// standard crates
use std::collections::HashMap;

// internal crates
use crate::mocks::http_client::{self as mock, Call, CapturedRequest, MockClient};
use backend_api::models::{
    Calibration, CreateCalibrationRequest, Error as ApiError, ErrorResponse, InstanceContent,
    InstanceFormat,
};
use miru_agent::errors::Error;
use miru_agent::http::calibrations::{self, CreateParams, CREATE_TIMEOUT};
use miru_agent::http::errors::MockErr;
use miru_agent::http::{self, HTTPErr};

// external crates
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};

fn payload() -> CreateCalibrationRequest {
    CreateCalibrationRequest {
        deployment_id: Some("dpl_1".to_string()),
        component_slug: "front-camera".to_string(),
        role: Some("left".to_string()),
        serial_number: Some("SN-123".to_string()),
        aspect: Some("intrinsics".to_string()),
        content: Box::new(InstanceContent {
            format: InstanceFormat::INSTANCE_FORMAT_JSON,
            data: r#"{"fx":1.0}"#.to_string(),
        }),
        reported_at: "2026-10-19T12:00:00Z".to_string(),
    }
}

fn calibration() -> Calibration {
    Calibration {
        id: "cal_1".to_string(),
        device_id: "dvc_1".to_string(),
        component_id: "cmp_1".to_string(),
        digest: "sha256:abc".to_string(),
        deployment_id: Some("dpl_1".to_string()),
        created_at: "2026-10-19T12:00:01Z".to_string(),
        ..Calibration::default()
    }
}

async fn create(client: &impl http::ClientI) -> Result<Calibration, HTTPErr> {
    let payload = payload();
    calibrations::create(
        client,
        CreateParams {
            payload: &payload,
            token: "test-token",
        },
    )
    .await
}

pub mod create {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient {
            create_calibration_fn: std::sync::Mutex::new(Box::new(|| Ok(calibration()))),
            ..MockClient::default()
        };

        let result = create(&mock).await.unwrap();

        assert_eq!(result, calibration());
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::CreateCalibration,
                method: reqwest::Method::POST,
                path: "/calibrations".into(),
                url: "http://mock/calibrations".into(),
                query: vec![],
                body: Some(serde_json::to_string(&payload()).unwrap()),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn body_carries_reported_at() {
        let mock = MockClient::default();
        create(&mock).await.unwrap();

        let body = mock.requests()[0].body.clone().unwrap();
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["reported_at"], "2026-10-19T12:00:00Z");
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient {
            create_calibration_fn: std::sync::Mutex::new(Box::new(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            })),
            ..MockClient::default()
        };

        let err = create(&mock).await.unwrap_err();
        assert!(matches!(err, HTTPErr::MockErr(_)));
        assert!(err.is_network_conn_err());
    }

    #[tokio::test]
    async fn server_200_returns_calibration() {
        // The agent's requests carry no Content-Type, so parse the body directly.
        async fn handler(body: String) -> Json<Calibration> {
            let req: CreateCalibrationRequest = serde_json::from_str(&body).unwrap();
            assert_eq!(req, payload());
            Json(calibration())
        }
        let server = mock::run_server(Router::new().route("/calibrations", post(handler))).await;
        let client = http::Client::new(&server.base_url).unwrap();

        assert_eq!(create(&client).await.unwrap(), calibration());
    }

    #[tokio::test]
    async fn server_422_passes_through() {
        async fn handler() -> (StatusCode, Json<ErrorResponse>) {
            let params = HashMap::from([(
                "field".to_string(),
                serde_json::Value::String("component_slug".to_string()),
            )]);
            let body = ErrorResponse::new(ApiError::new(
                "invalid_request".to_string(),
                params,
                "invalid component slug".to_string(),
            ));
            (StatusCode::UNPROCESSABLE_ENTITY, Json(body))
        }
        let server = mock::run_server(Router::new().route("/calibrations", post(handler))).await;
        let client = http::Client::new(&server.base_url).unwrap();

        let err = create(&client).await.unwrap_err();

        let HTTPErr::RequestFailed(failed) = &err else {
            panic!("expected RequestFailed, got {err:?}");
        };
        assert_eq!(failed.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(failed.error.as_ref().unwrap().error.code, "invalid_request");
        assert!(err.is_terminal());
        assert!(!err.is_network_conn_err());
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_is_network_error() {
        async fn handler() -> Json<Calibration> {
            std::future::pending().await
        }
        let server = mock::run_server(Router::new().route("/calibrations", post(handler))).await;
        let client = http::Client::new(&server.base_url).unwrap();

        let start = tokio::time::Instant::now();
        let err = create(&client).await.unwrap_err();

        let HTTPErr::TimeoutErr(timeout) = &err else {
            panic!("expected TimeoutErr, got {err:?}");
        };
        assert_eq!(timeout.request.timeout, CREATE_TIMEOUT);
        assert!(start.elapsed() >= CREATE_TIMEOUT);
        assert!(err.is_network_conn_err());
    }
}
