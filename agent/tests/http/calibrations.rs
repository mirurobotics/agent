// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};

// internal crates
use crate::mocks::http_client::{self as mock, Call, CapturedRequest, MockClient};
use backend_api::models::{
    Calibration, CalibrationList, CreateCalibrationRequest, InstanceContent, InstanceFormat,
};
use miru_agent::errors::Error;
use miru_agent::http::calibrations::{
    self, CreateParams, GetContentParams, ListAllLatestParams, ListLatestParams, CONTENT_TIMEOUT,
};
use miru_agent::http::errors::MockErr;
use miru_agent::http::query::Page;
use miru_agent::http::{self, HTTPErr};

// external crates
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

fn report_id() -> Uuid {
    Uuid::from_u128(0x0b6f3c2e_6a51_4e0f_9a43_2f1d7c9e8b10)
}

fn payload() -> CreateCalibrationRequest {
    CreateCalibrationRequest {
        report_id: report_id(),
        deployment_id: Some("dpl_1".to_string()),
        component: "front-camera".to_string(),
        role: Some("left".to_string()),
        serial_number: Some("SN-123".to_string()),
        reported_by: Some("Ada Lovelace".to_string()),
        aspect: Some("intrinsics".to_string()),
        content: Box::new(InstanceContent {
            format: InstanceFormat::INSTANCE_FORMAT_JSON,
            data: r#"{"fx":1.0}"#.to_string(),
        }),
        reported_at: "2026-10-19T12:00:00Z".to_string(),
    }
}

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

fn calibration() -> Calibration {
    Calibration {
        id: "cal_1".to_string(),
        device_id: "dvc_1".to_string(),
        component_id: "cmp_1".to_string(),
        digest: "sha256:abc".to_string(),
        deployment_id: Some("dpl_1".to_string()),
        report_id: report_id(),
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
        assert_eq!(timeout.request.timeout, CONTENT_TIMEOUT);
        assert!(start.elapsed() >= CONTENT_TIMEOUT);
        assert!(err.is_network_conn_err());
    }
}

pub mod list_latest {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();
        mock.set_list_latest_calibrations(|| {
            Ok(CalibrationList {
                data: vec![calibration()],
                ..CalibrationList::default()
            })
        });

        let result = calibrations::list_latest(
            &mock,
            ListLatestParams {
                expansions: &[],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected = CalibrationList {
            data: vec![calibration()],
            ..CalibrationList::default()
        };
        assert_eq!(result, expected);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ListLatestCalibrations,
                method: reqwest::Method::GET,
                path: "/calibrations/latest".into(),
                url: "http://mock/calibrations/latest".into(),
                query: vec![("limit".into(), "10".into()), ("offset".into(), "0".into())],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn with_expansions() {
        let mock = MockClient::default();

        calibrations::list_latest(
            &mock,
            ListLatestParams {
                expansions: &["reported_by", "total_count"],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ListLatestCalibrations,
                method: reqwest::Method::GET,
                path: "/calibrations/latest".into(),
                url: "http://mock/calibrations/latest".into(),
                query: vec![
                    ("limit".into(), "10".into()),
                    ("offset".into(), "0".into()),
                    ("expand".into(), "reported_by".into()),
                    ("expand".into(), "total_count".into()),
                ],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_list_latest_calibrations(|| Err(mock_err()));

        let result = calibrations::list_latest(
            &mock,
            ListLatestParams {
                expansions: &[],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod list_all_latest {
    use super::*;

    async fn list_all_latest(mock: &MockClient) -> Result<Vec<Calibration>, HTTPErr> {
        calibrations::list_all_latest(
            mock,
            ListAllLatestParams {
                expansions: &[],
                token: "test-token",
            },
        )
        .await
    }

    fn calibration_with_id(id: &str) -> Calibration {
        Calibration {
            id: id.to_string(),
            ..calibration()
        }
    }

    #[tokio::test]
    async fn single_page() {
        let mock = MockClient::default();
        mock.set_list_latest_calibrations(|| {
            Ok(CalibrationList {
                data: vec![calibration()],
                ..CalibrationList::default()
            })
        });

        let result = list_all_latest(&mock).await.unwrap();

        assert_eq!(result, vec![calibration()]);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ListLatestCalibrations,
                method: reqwest::Method::GET,
                path: "/calibrations/latest".into(),
                url: "http://mock/calibrations/latest".into(),
                query: vec![
                    ("limit".into(), "100".into()),
                    ("offset".into(), "0".into()),
                ],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn multi_page_pagination() {
        let mock = MockClient::default();
        let call_num = AtomicUsize::new(0);
        mock.set_list_latest_calibrations(move || {
            let n = call_num.fetch_add(1, Ordering::SeqCst);
            Ok(CalibrationList {
                has_more: n == 0,
                data: vec![calibration_with_id(&format!("cal_{n}"))],
                ..CalibrationList::default()
            })
        });

        let result = list_all_latest(&mock).await.unwrap();

        assert_eq!(
            result,
            vec![calibration_with_id("cal_0"), calibration_with_id("cal_1")]
        );
        let base_req = CapturedRequest {
            call: Call::ListLatestCalibrations,
            method: reqwest::Method::GET,
            path: "/calibrations/latest".into(),
            url: "http://mock/calibrations/latest".into(),
            query: vec![
                ("limit".into(), "100".into()),
                ("offset".into(), "0".into()),
            ],
            body: None,
            token: Some("test-token".into()),
        };
        assert_eq!(
            mock.requests(),
            vec![
                base_req.clone(),
                CapturedRequest {
                    query: vec![
                        ("limit".into(), "100".into()),
                        ("offset".into(), "100".into()),
                    ],
                    ..base_req
                },
            ]
        );
    }

    #[tokio::test]
    async fn empty_result() {
        let mock = MockClient::default();

        let result = list_all_latest(&mock).await.unwrap();

        assert_eq!(result, Vec::<Calibration>::new());
        assert_eq!(mock.call_count(Call::ListLatestCalibrations), 1);
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_list_latest_calibrations(|| Err(mock_err()));

        let result = list_all_latest(&mock).await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod get_content {
    use super::*;

    async fn get_content(client: &impl http::ClientI) -> Result<String, HTTPErr> {
        calibrations::get_content(
            client,
            GetContentParams {
                id: "cal_1",
                token: "test-token",
            },
        )
        .await
    }

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();
        mock.set_get_calibration_content(|id| {
            assert_eq!(id, "cal_1");
            Ok("fx: 1.0\nfy: 1.0".to_string())
        });

        let result = get_content(&mock).await.unwrap();

        assert_eq!(result, "fx: 1.0\nfy: 1.0");
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::GetCalibrationContent,
                method: reqwest::Method::GET,
                path: "/calibrations/cal_1/content".into(),
                url: "http://mock/calibrations/cal_1/content".into(),
                query: vec![],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_get_calibration_content(|_| Err(mock_err()));

        let result = get_content(&mock).await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_is_network_error() {
        async fn handler() -> String {
            std::future::pending().await
        }
        let path = "/calibrations/cal_1/content";
        let server = mock::run_server(Router::new().route(path, get(handler))).await;
        let client = http::Client::new(&server.base_url).unwrap();

        let start = tokio::time::Instant::now();
        let err = get_content(&client).await.unwrap_err();

        let HTTPErr::TimeoutErr(timeout) = &err else {
            panic!("expected TimeoutErr, got {err:?}");
        };
        assert_eq!(timeout.request.timeout, CONTENT_TIMEOUT);
        assert!(start.elapsed() >= CONTENT_TIMEOUT);
        assert!(err.is_network_conn_err());
    }
}
