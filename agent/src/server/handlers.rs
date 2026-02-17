use std::sync::Arc;

// internal crates
use crate::errors::Error;
use crate::models::device::DeviceStatus;
use crate::server::errors::*;
use crate::server::state::ServerState;
use crate::services::device::{get, sync};
use crate::version;
use openapi_server::models as openapi;

// external
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use tracing::error;

// ================================= AGENT INFO ==================================== //
pub async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(openapi::HealthResponse {
            status: "ok".to_string(),
        }),
    )
}

pub async fn version() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(openapi::VersionResponse {
            version: version::VERSION.to_string(),
            commit: version::COMMIT.to_string(),
        }),
    )
}

// ================================= DEVICE ======================================== //
pub async fn get_device(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let service = async move {
        let device = get::get_device(&state.device_file).await?;
        Ok::<openapi_server::models::Device, ServerErr>(openapi_server::models::Device {
            object: openapi_server::models::device::Object::Device,
            id: device.id.clone(),
            name: device.name.clone(),
            status: DeviceStatus::to_sdk(&device.status),
            last_synced_at: device.last_synced_at.to_rfc3339(),
            last_connected_at: device.last_connected_at.to_rfc3339(),
            last_disconnected_at: device.last_disconnected_at.to_rfc3339(),
        })
    };

    match service.await {
        Ok(device) => (StatusCode::OK, Json(json!(device))),
        Err(e) => {
            error!("Error getting device: {e:?}");
            (e.http_status(), Json(json!(to_error_response(e))))
        }
    }
}

pub async fn sync_device(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let service = async move { sync::sync_device(state.syncer.as_ref()).await };

    match service.await {
        Ok(device) => (StatusCode::OK, Json(json!(device))),
        Err(e) => {
            error!("Error syncing device: {e:?}");
            (e.http_status(), Json(json!(to_error_response(e))))
        }
    }
}

// ================================ UTILITIES ====================================== //
fn to_error_response(e: impl Error) -> openapi::ErrorResponse {
    let params = e
        .params()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    openapi::ErrorResponse {
        error: Box::new(openapi::Error {
            code: e.code().as_str().to_string(),
            params,
            message: e.to_string(),
        }),
    }
}
