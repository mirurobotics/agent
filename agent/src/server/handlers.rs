// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::errors::Error;
use crate::models::InstanceFormat;
use crate::server::{errors::*, routes, state::State};
use crate::services::{
    calibration as calibration_svc, deployment as dpl_svc, device as dvc_svc,
    file_rule as file_rule_svc, git_commit as git_cmt_svc, release as rls_svc, HttpBackend,
};
use crate::trace;
use crate::version;
use device_api::models as device_server;

// external crates
use axum::{
    extract::{rejection::JsonRejection, Path, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::error;

// ================================= AGENT INFO ==================================== //
pub async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(device_server::HealthResponse {
            status: "ok".to_string(),
        }),
    )
}

pub async fn version() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(device_server::VersionResponse {
            version: version::VERSION.to_string(),
            git_commit: version::COMMIT.to_string(),
            api_version: version::api_version(),
            api_git_commit: version::api_git_commit(),
            api_release_version: version::api_release_version(),
            rust_version: version::RUST_VERSION.to_string(),
            build_date: version::BUILD_DATE.to_string(),
            os: version::OS.to_string(),
            arch: version::ARCH.to_string(),
        }),
    )
}

// ================================= DEVICE ======================================== //
pub async fn get_device(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    handle(
        async move {
            let device = dvc_svc::get(&state.storage.device).await?;
            Ok::<_, ServerErr>(device_server::Device::from(&device))
        },
        "Error getting device",
    )
    .await
}

pub async fn sync_device(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    handle(
        async move { dvc_svc::sync(state.syncer.as_ref()).await },
        "Error syncing device",
    )
    .await
}

// ================================ DEPLOYMENTS ==================================== //
pub async fn get_deployment(
    AxumState(state): AxumState<Arc<State>>,
    Path(deployment_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let backend = HttpBackend::new(state.http_client.as_ref(), state.token_mngr.as_ref());
            let dpl = dpl_svc::get(&state.storage.deployments, &backend, deployment_id).await?;
            Ok::<_, ServerErr>(device_server::Deployment::from(&dpl))
        },
        "Error getting deployment",
    )
    .await
}

pub async fn get_current_deployment(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    handle(
        async {
            let dpl = dpl_svc::get_current(&state.storage.deployments).await?;
            Ok::<_, ServerErr>(device_server::Deployment::from(&dpl))
        },
        "Error getting current deployment",
    )
    .await
}

// ================================= RELEASES ====================================== //
pub async fn get_release(
    AxumState(state): AxumState<Arc<State>>,
    Path(release_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let backend = HttpBackend::new(state.http_client.as_ref(), state.token_mngr.as_ref());
            let release = rls_svc::get(
                &state.storage.releases,
                &state.storage.file_rules,
                &backend,
                release_id,
            )
            .await?;
            Ok::<_, ServerErr>(device_server::Release::from(&release))
        },
        "Error getting release",
    )
    .await
}

pub async fn get_current_release(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    handle(
        async {
            let backend = HttpBackend::new(state.http_client.as_ref(), state.token_mngr.as_ref());
            let release = rls_svc::get_current(
                &state.storage.deployments,
                &state.storage.releases,
                &state.storage.file_rules,
                &backend,
            )
            .await?;
            Ok::<_, ServerErr>(device_server::Release::from(&release))
        },
        "Error getting current release",
    )
    .await
}

// ================================ GIT COMMITS ==================================== //
pub async fn get_git_commit(
    AxumState(state): AxumState<Arc<State>>,
    Path(git_commit_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let backend = HttpBackend::new(state.http_client.as_ref(), state.token_mngr.as_ref());
            let gc = git_cmt_svc::get(&state.storage.git_commits, &backend, git_commit_id).await?;
            Ok::<_, ServerErr>(device_server::GitCommit::from(&gc))
        },
        "Error getting git commit",
    )
    .await
}

// ================================ FILE RULES ===================================== //
pub async fn get_file_rule(
    AxumState(state): AxumState<Arc<State>>,
    Path(file_rule_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let backend = HttpBackend::new(state.http_client.as_ref(), state.token_mngr.as_ref());
            let rule =
                file_rule_svc::get(&state.storage.file_rules, &backend, file_rule_id).await?;
            Ok::<_, ServerErr>(device_server::BaseFileRule::from(&rule))
        },
        "Error getting file rule",
    )
    .await
}

// =============================== CALIBRATIONS ==================================== //
pub async fn create_calibration(
    AxumState(state): AxumState<Arc<State>>,
    request: Result<Json<device_server::CreateCalibrationRequest>, JsonRejection>,
) -> impl IntoResponse {
    handle(
        async {
            let params = to_create_params(request)?;
            let created = calibration_svc::create(
                &state.calibration_uploader,
                &state.storage.deployments,
                params,
                Utc::now(),
            )
            .await?;
            Ok::<_, ServerErr>(device_server::CreateCalibrationResponse::from(&created))
        },
        "Error creating calibration",
    )
    .await
}

/// Checks the request's shape only; Miru judges the rest.
fn to_create_params(
    request: Result<Json<device_server::CreateCalibrationRequest>, JsonRejection>,
) -> Result<calibration_svc::CreateParams, ServerErr> {
    let Json(request) = request.map_err(reject_body)?;
    let content = *request.content;
    let Some(format) = InstanceFormat::from_device_api(content.format) else {
        return Err(ServerErr::InvalidRequestErr(InvalidRequestErr {
            msg: "content.format is not a known format".to_string(),
            trace: trace!(),
        }));
    };
    Ok(calibration_svc::CreateParams {
        component: request.component_slug,
        role: request.role,
        aspect: request.aspect,
        serial_number: request.serial_number,
        format,
        data: content.data,
    })
}

/// A body over the limit is 413; any other unreadable body is 422.
fn reject_body(rejection: JsonRejection) -> ServerErr {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ServerErr::CalibrationRequestTooLargeErr(CalibrationRequestTooLargeErr {
            max_request_bytes: routes::MAX_REQUEST_BYTES,
            trace: trace!(),
        });
    }
    ServerErr::InvalidRequestErr(InvalidRequestErr {
        msg: rejection.body_text(),
        trace: trace!(),
    })
}

// ================================ UTILITIES ====================================== //
async fn handle<F, T, E>(service: F, err_msg: &str) -> (StatusCode, Json<Value>)
where
    F: Future<Output = Result<T, E>>,
    T: Serialize,
    E: Into<ServerErr>,
{
    match service.await {
        Ok(val) => (StatusCode::OK, Json(json!(val))),
        Err(e) => {
            let e: ServerErr = e.into();
            error!("{err_msg}: {e:?}");
            (e.http_status(), Json(json!(to_error_response(e))))
        }
    }
}

fn to_error_response(e: impl Error) -> device_server::ErrorResponse {
    let params = e
        .params()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    device_server::ErrorResponse {
        error: Box::new(device_server::Error {
            code: e.code().as_str().to_string(),
            params,
            message: e.to_string(),
        }),
    }
}
