// internal crates
use crate::http::{
    errors::HTTPErr,
    query::{Page, QueryParams, MAX_PAGE_LIMIT},
    request, ClientI,
};
use backend_api::models::{
    Deployment, DeploymentActivityStatus, DeploymentList, UpdateDeploymentRequest,
};

// ================================ PARAM STRUCTS ================================== //

pub struct ListParams<'a> {
    pub activity_status: &'a [DeploymentActivityStatus],
    pub expansions: &'a [&'a str],
    pub pagination: &'a Page,
    pub token: &'a str,
}

pub struct ListAllParams<'a> {
    pub activity_status: &'a [DeploymentActivityStatus],
    pub expansions: &'a [&'a str],
    pub token: &'a str,
}

pub struct UpdateParams<'a> {
    pub id: &'a str,
    pub updates: &'a UpdateDeploymentRequest,
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn list(
    client: &impl ClientI,
    params: ListParams<'_>,
) -> Result<DeploymentList, HTTPErr> {
    let mut qp = QueryParams::new().paginate(params.pagination);
    if !params.activity_status.is_empty() {
        let values: Vec<String> = params
            .activity_status
            .iter()
            .map(|s| s.to_string())
            .collect();
        qp = qp.add("activity_status", &values.join("|"));
    }
    qp = qp.expand(params.expansions);

    let url = format!("{}/deployments", client.base_url());
    let request = request::Params::get(&url)
        .with_query(qp)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn list_all(
    client: &impl ClientI,
    params: ListAllParams<'_>,
) -> Result<Vec<Deployment>, HTTPErr> {
    let mut all_deployments = Vec::new();
    let mut pagination = Page {
        limit: MAX_PAGE_LIMIT,
        offset: 0,
    };

    loop {
        let page = list(
            client,
            ListParams {
                activity_status: params.activity_status,
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

pub async fn update(
    client: &impl ClientI,
    params: UpdateParams<'_>,
) -> Result<Deployment, HTTPErr> {
    let url = format!("{}/deployments/{}", client.base_url(), params.id,);
    let request = request::Params::patch(&url, request::marshal_json(params.updates)?)
        .with_token(params.token);
    super::client::fetch(client, request).await
}
