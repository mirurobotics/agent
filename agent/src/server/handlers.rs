use std::sync::Arc;

// internal crates
use crate::errors::Error;
use crate::server::errors::*;
use crate::server::state::ServerState;
use crate::services::deployment as dpl_svc;
use crate::services::device as dvc_svc;
use crate::services::git_commit as git_cmt_svc;
use crate::services::release as rls_svc;
use crate::version;
use openapi_server::models as openapi;

// external
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
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
pub async fn get_device(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    handle(
        async move {
            let device = dvc_svc::get(&state.storage.device).await?;
            Ok::<_, ServerErr>(openapi::Device::from(&device))
        },
        "Error getting device",
    )
    .await
}

pub async fn sync_device(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    handle(
        async move { dvc_svc::sync(state.syncer.as_ref()).await },
        "Error syncing device",
    )
    .await
}

// ================================ DEPLOYMENTS ==================================== //
pub async fn get_deployment(
    State(state): State<Arc<ServerState>>,
    Path(deployment_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let dpl = dpl_svc::get(&state.storage.deployments, deployment_id).await?;
            Ok::<_, ServerErr>(openapi::Deployment::from(&dpl))
        },
        "Error getting deployment",
    )
    .await
}

pub async fn get_current_deployment(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    handle(
        async {
            let dpl = dpl_svc::get_current(&state.storage.deployments).await?;
            Ok::<_, ServerErr>(openapi::Deployment::from(&dpl))
        },
        "Error getting current deployment",
    )
    .await
}

// ================================= RELEASES ====================================== //
pub async fn get_release(
    State(state): State<Arc<ServerState>>,
    Path(release_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let release = rls_svc::get(&state.storage.releases, release_id).await?;
            Ok::<_, ServerErr>(openapi::Release::from(&release))
        },
        "Error getting release",
    )
    .await
}

pub async fn get_current_release(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    handle(
        async {
            let release =
                rls_svc::get_current(&state.storage.deployments, &state.storage.releases).await?;
            Ok::<_, ServerErr>(openapi::Release::from(&release))
        },
        "Error getting current release",
    )
    .await
}

// ================================ GIT COMMITS ==================================== //
pub async fn get_git_commit(
    State(state): State<Arc<ServerState>>,
    Path(git_commit_id): Path<String>,
) -> impl IntoResponse {
    handle(
        async {
            let gc = git_cmt_svc::get(&state.storage.git_commits, git_commit_id).await?;
            Ok::<_, ServerErr>(openapi::GitCommit::from(&gc))
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
