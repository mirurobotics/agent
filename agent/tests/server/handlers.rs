// internal crates
use miru_agent::server::handlers;
use miru_agent::version::{COMMIT, VERSION};

// external crates
use axum::body;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use openapi_server::models::{HealthResponse, VersionResponse};

pub mod health {
    use super::*;

    #[tokio::test]
    async fn returns_ok_with_status() {
        let response = handlers::health().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = body::to_bytes(response.into_body(), 1024).await.unwrap();
        let actual: HealthResponse = serde_json::from_slice(&bytes).unwrap();
        let expected = HealthResponse {
            status: "ok".to_string(),
        };
        assert_eq!(actual, expected);
    }
}

pub mod version {
    use super::*;

    #[tokio::test]
    async fn returns_ok_with_version_and_commit() {
        let response = handlers::version().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = body::to_bytes(response.into_body(), 1024).await.unwrap();
        let actual: VersionResponse = serde_json::from_slice(&bytes).unwrap();
        let expected = VersionResponse {
            version: VERSION.to_string(),
            commit: COMMIT.to_string(),
        };
        assert_eq!(actual, expected);
    }
}
