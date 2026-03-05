// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use super::errors::*;
use super::state::State;
use crate::errors::Error;
use crate::services::deployment as dpl_svc;
use crate::services::device as dvc_svc;
use crate::services::git_commit as git_cmt_svc;
use crate::services::release as rls_svc;
use crate::version;
use device_api::models as device_server;

// external crates
use axum::{
    extract::{Path, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
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
            let dpl = dpl_svc::get(&state.storage.deployments, deployment_id).await?;
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
            let release = rls_svc::get(&state.storage.releases, release_id).await?;
            Ok::<_, ServerErr>(device_server::Release::from(&release))
        },
        "Error getting release",
    )
    .await
}

pub async fn get_current_release(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    handle(
        async {
            let release =
                rls_svc::get_current(&state.storage.deployments, &state.storage.releases).await?;
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
            let gc = git_cmt_svc::get(&state.storage.git_commits, git_commit_id).await?;
            Ok::<_, ServerErr>(device_server::GitCommit::from(&gc))
        },
        "Error getting git commit",
    )
    .await
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
