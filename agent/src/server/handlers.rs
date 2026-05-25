// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::errors::Error;
use crate::server::{errors::*, state::State};
use crate::services::{
    config_instance as ci_svc, deployment as dpl_svc, device as dvc_svc,
    git_commit as git_cmt_svc, release as rls_svc, HttpBackend,
};
use crate::version;
use device_api::models as device_server;

// external crates
use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
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
            let release = rls_svc::get(&state.storage.releases, &backend, release_id).await?;
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

// ============================= CONFIG INSTANCES ================================= //
#[derive(Deserialize)]
pub struct ExpandQuery {
    pub expand: Option<String>,
}

pub async fn get_config_instance(
    AxumState(state): AxumState<Arc<State>>,
    Path(config_instance_id): Path<String>,
    Query(query): Query<ExpandQuery>,
) -> impl IntoResponse {
    handle(
        async move {
            let expand_content = query
                .expand
                .as_deref()
                .map(|v| v == "content")
                .unwrap_or(false);
            let response = ci_svc::get_response(
                &state.storage.cfg_insts.meta,
                &state.storage.cfg_insts.content,
                config_instance_id,
                expand_content,
            )
            .await?;
            Ok::<_, ServerErr>(response)
        },
        "Error getting config instance",
    )
    .await
}

pub async fn get_config_instance_content(
    AxumState(state): AxumState<Arc<State>>,
    Path(config_instance_id): Path<String>,
) -> impl IntoResponse {
    let result = ci_svc::get_raw_content(
        &state.storage.cfg_insts.meta,
        &state.storage.cfg_insts.content,
        config_instance_id,
    )
    .await;

    match result {
        Ok((ci, content)) => {
            let format = ci_svc::infer_format(&ci.filepath);
            let content_type = match format {
                "yaml" => "application/x-yaml",
                _ => "application/json",
            };
            let filename = ci
                .filepath
                .rsplit('/')
                .next()
                .unwrap_or(&ci.filepath)
                .to_string();
            let disposition = format!("attachment; filename=\"{filename}\"");

            (
                StatusCode::OK,
                [
                    (
                        axum::http::header::CONTENT_TYPE,
                        content_type.to_string(),
                    ),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        disposition,
                    ),
                ],
                content,
            )
                .into_response()
        }
        Err(e) => {
            let e: ServerErr = e.into();
            error!("Error getting config instance content: {e:?}");
            let error_response = to_error_response(&e);
            (e.http_status(), Json(json!(error_response))).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct PrefixQuery {
    pub prefix: Option<String>,
}

pub async fn get_config_instance_parameter(
    AxumState(state): AxumState<Arc<State>>,
    Path((config_instance_id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    handle(
        async move {
            let response = ci_svc::get_parameter(
                &state.storage.cfg_insts.meta,
                &state.storage.cfg_insts.content,
                config_instance_id,
                key,
            )
            .await?;
            Ok::<_, ServerErr>(response)
        },
        "Error getting config instance parameter",
    )
    .await
}

pub async fn list_config_instance_parameters(
    AxumState(state): AxumState<Arc<State>>,
    Path(config_instance_id): Path<String>,
    Query(query): Query<PrefixQuery>,
) -> impl IntoResponse {
    handle(
        async move {
            let response = ci_svc::list_parameters(
                &state.storage.cfg_insts.meta,
                &state.storage.cfg_insts.content,
                config_instance_id,
                query.prefix,
            )
            .await?;
            Ok::<_, ServerErr>(response)
        },
        "Error listing config instance parameters",
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
            (e.http_status(), Json(json!(to_error_response(&e))))
        }
    }
}

fn to_error_response(e: &(impl Error + ?Sized)) -> device_server::ErrorResponse {
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
