use std::sync::Arc;

// internal crates
use crate::errors::MiruError;
use crate::models::device::DeviceStatus;
use crate::server::errors::*;
use crate::server::state::ServerState;
use crate::services::device::{get, sync};
use crate::trace;
use crate::utils::version_info;
use openapi_server::models::{Error, ErrorResponse, HealthResponse, VersionResponse};

// external
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use tracing::error;

// ================================= AGENT INFO ==================================== //
pub async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
        }),
    )
}

pub async fn version() -> impl IntoResponse {
    let version_info = version_info();
    (
        StatusCode::OK,
        Json(VersionResponse {
            version: version_info.version,
            commit: version_info.commit,
        }),
    )
}

// ================================= DEVICE ======================================== //
pub async fn get_device(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let service = async move {
        let device = get::get_device(&state.device_file).await.map_err(|e| {
            ServerErr::ServiceErr(Box::new(ServerServiceErr {
                source: e,
                trace: trace!(),
            }))
        })?;
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
    let service = async move {
        sync::sync_device(state.syncer.as_ref()).await.map_err(|e| {
            ServerErr::ServiceErr(Box::new(ServerServiceErr {
                source: e,
                trace: trace!(),
            }))
        })
    };

    match service.await {
        Ok(device) => (StatusCode::OK, Json(json!(device))),
        Err(e) => {
            error!("Error syncing device: {e:?}");
            (e.http_status(), Json(json!(to_error_response(e))))
        }
    }
}

// ================================ UTILITIES ====================================== //
fn to_error_response(e: impl MiruError) -> ErrorResponse {
    let params = e
        .params()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    ErrorResponse {
        error: Box::new(Error {
            code: e.code().as_str().to_string(),
            params,
            message: e.to_string(),
        }),
    }
}
