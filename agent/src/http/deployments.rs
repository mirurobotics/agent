// internal crates
use crate::http::errors::HTTPErr;
use crate::http::expand::format_expand_query;
use crate::http::pagination::{Pagination, MAX_PAGINATE_LIMIT};
use crate::http::query::build_query_params;
use crate::http::request;
use crate::http::response;
use crate::http::ClientI;
use openapi_client::models::{
    Deployment, DeploymentActivityStatus, DeploymentList, DeploymentListExpansion,
    UpdateDeploymentRequest,
};

// ================================ PARAM STRUCTS ================================== //

pub struct ListParams<'a> {
    pub activity_status_filter: &'a [DeploymentActivityStatus],
    pub expansions: &'a [DeploymentListExpansion],
    pub pagination: &'a Pagination,
    pub token: &'a str,
}

pub struct ListAllParams<'a> {
    pub activity_status_filter: &'a [DeploymentActivityStatus],
    pub expansions: &'a [DeploymentListExpansion],
    pub token: &'a str,
}

pub struct GetParams<'a> {
    pub deployment_id: &'a str,
    pub expansions: &'a [DeploymentListExpansion],
    pub token: &'a str,
}

pub struct UpdateParams<'a> {
    pub deployment_id: &'a str,
    pub updates: &'a UpdateDeploymentRequest,
    pub expansions: &'a [DeploymentListExpansion],
    pub token: &'a str,
}

// ================================ HELPERS ========================================= //

fn format_activity_status_filter(statuses: &[DeploymentActivityStatus]) -> Option<String> {
    if statuses.is_empty() {
        return None;
    }
    let values: Vec<String> = statuses.iter().map(|s| s.to_string()).collect();
    Some(format!("activity_status={}", values.join(",")))
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn list(
    client: &impl ClientI,
    params: ListParams<'_>,
) -> Result<DeploymentList, HTTPErr> {
    let search_query = format_activity_status_filter(params.activity_status_filter);
    let expand_query = format_expand_query(params.expansions);
    let query_params = build_query_params(
        search_query.as_deref(),
        expand_query.as_deref(),
        params.pagination,
    );

    let url = format!("{}/deployments{}", client.base_url(), query_params);
    let request = request::Params::get(&url, client.default_timeout()).with_token(params.token);
    let meta = request.meta();
    let text = client.execute_cached(url.clone(), request).await?;
    response::parse_json(text, meta)
}

pub async fn list_all(
    client: &impl ClientI,
    params: ListAllParams<'_>,
) -> Result<Vec<Deployment>, HTTPErr> {
    let mut all_deployments = Vec::new();
    let mut pagination = Pagination {
        limit: MAX_PAGINATE_LIMIT,
        offset: 0,
    };

    loop {
        let page = list(
            client,
            ListParams {
                activity_status_filter: params.activity_status_filter,
                expansions: params.expansions,
                pagination: &pagination,
                token: params.token,
            },
        )
        .await?;
        all_deployments.extend(page.data);
        if !page.has_more {
            break;
        }
        pagination.offset += pagination.limit;
    }

    Ok(all_deployments)
}

pub async fn get(client: &impl ClientI, params: GetParams<'_>) -> Result<Deployment, HTTPErr> {
    let expand_query = format_expand_query(params.expansions);
    let query_params = if let Some(expand) = expand_query {
        format!("?{}", expand)
    } else {
        String::new()
    };

    let url = format!(
        "{}/deployments/{}{}",
        client.base_url(),
        params.deployment_id,
        query_params
    );
    let request = request::Params::get(&url, client.default_timeout()).with_token(params.token);
    let meta = request.meta();
    let text = client.execute_cached(url.clone(), request).await?;
    response::parse_json(text, meta)
}

pub async fn update(
    client: &impl ClientI,
    params: UpdateParams<'_>,
) -> Result<Deployment, HTTPErr> {
    let expand_query = format_expand_query(params.expansions);
    let query_params = if let Some(expand) = expand_query {
        format!("?{}", expand)
    } else {
        String::new()
    };

    let url = format!(
        "{}/deployments/{}{}",
        client.base_url(),
        params.deployment_id,
        query_params
    );
    let request = request::Params::patch(
        &url,
        request::marshal_json(params.updates)?,
        client.default_timeout(),
    )
    .with_token(params.token);
    let meta = request.meta();
    let text = client.execute(request).await?;
    response::parse_json(text, meta)
}
