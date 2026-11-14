// standard crates
use std::sync::Arc;

// internal crates
use crate::server::{
    state::State,
    v02::{handlers, sse},
};

// external crates
use axum::{
    routing::{get, post},
    Router,
};

pub fn table() -> Router<Arc<State>> {
    let api_version = device_api_v02::models::ApiVersion::API_VERSION.to_string();
    Router::new()
        .merge(agent_info(&api_version))
        .merge(device(&api_version))
        .merge(deployments(&api_version))
        .merge(releases(&api_version))
        .merge(git_commits(&api_version))
        .merge(file_rules(&api_version))
        .merge(calibrations(&api_version))
        .merge(events(&api_version))
}

fn agent_info(api_version: &str) -> Router<Arc<State>> {
    Router::new()
        .route(
            format!("/{api_version}/health").as_str(),
            get(handlers::health),
        )
        .route(
            format!("/{api_version}/version").as_str(),
            get(handlers::version),
        )
}

fn device(api_version: &str) -> Router<Arc<State>> {
    Router::new()
        .route(
            format!("/{api_version}/device").as_str(),
            get(handlers::get_device),
        )
        .route(
            format!("/{api_version}/device/sync").as_str(),
            post(handlers::sync_device),
        )
}

fn deployments(api_version: &str) -> Router<Arc<State>> {
    // /current before /{id} so "current" isn't captured as a deployment_id
    Router::new()
        .route(
            format!("/{api_version}/deployments/current").as_str(),
            get(handlers::get_current_deployment),
        )
        .route(
            format!("/{api_version}/deployments/{{deployment_id}}").as_str(),
            get(handlers::get_deployment),
        )
}

fn releases(api_version: &str) -> Router<Arc<State>> {
    // /current before /{id} so "current" isn't captured as a release_id
    Router::new()
        .route(
            format!("/{api_version}/releases/current").as_str(),
            get(handlers::get_current_release),
        )
        .route(
            format!("/{api_version}/releases/{{release_id}}").as_str(),
            get(handlers::get_release),
        )
}

fn git_commits(api_version: &str) -> Router<Arc<State>> {
    Router::new().route(
        format!("/{api_version}/git_commits/{{git_commit_id}}").as_str(),
        get(handlers::get_git_commit),
    )
}

fn file_rules(api_version: &str) -> Router<Arc<State>> {
    Router::new().route(
        format!("/{api_version}/file_rules/{{file_rule_id}}").as_str(),
        get(handlers::get_file_rule),
    )
}

fn calibrations(api_version: &str) -> Router<Arc<State>> {
    Router::new()
        .route(
            format!("/{api_version}/calibrations").as_str(),
            post(handlers::create_calibration),
        )
        .route(
            format!("/{api_version}/calibrations/latest").as_str(),
            get(handlers::list_latest_calibrations),
        )
        .route(
            format!("/{api_version}/calibrations/latest/{{report_id}}/content").as_str(),
            get(handlers::get_latest_calibration_content),
        )
}

fn events(api_version: &str) -> Router<Arc<State>> {
    Router::new().route(format!("/{api_version}/events").as_str(), get(sse::events))
}
