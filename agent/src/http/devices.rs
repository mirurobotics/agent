// internal crates
use crate::http::{errors::HTTPErr, query::QueryParams, request, ClientI};
use backend_api::models::{
    ActivateDeviceRequest, Device, IssueDeviceTokenRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};

// external crates
use serde::Serialize;

// ================================ LOCAL REQUEST TYPES ============================ //

// TODO(provision): move to libs/backend-api once OpenAPI spec is regenerated.
#[derive(Debug, Serialize)]
pub struct CreateDeviceRequest {
    pub name: String,
}

// TODO(provision): move to libs/backend-api once OpenAPI spec is regenerated.
#[derive(Debug, Serialize)]
pub struct IssueActivationTokenRequest {
    pub allow_reactivation: bool,
}

// ================================ PARAM STRUCTS ================================== //

pub struct ActivateParams<'a> {
    pub id: &'a str,
    pub payload: &'a ActivateDeviceRequest,
    pub token: &'a str,
}

pub struct IssueTokenParams<'a> {
    pub id: &'a str,
    pub payload: &'a IssueDeviceTokenRequest,
}

pub struct UpdateParams<'a> {
    pub id: &'a str,
    pub payload: &'a UpdateDeviceFromAgentRequest,
    pub token: &'a str,
}

pub struct CreateOrFetchDeviceParams<'a> {
    pub name: &'a str,
    pub api_key: &'a str,
}

pub struct IssueActivationTokenParams<'a> {
    pub id: &'a str,
    pub api_key: &'a str,
    pub allow_reactivation: bool,
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn activate(
    client: &impl ClientI,
    params: ActivateParams<'_>,
) -> Result<Device, HTTPErr> {
    let url = format!("{}/devices/{}/activate", client.base_url(), params.id);
    let request = request::Params::post(&url, request::marshal_json(params.payload)?)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn issue_token(
    client: &impl ClientI,
    params: IssueTokenParams<'_>,
) -> Result<TokenResponse, HTTPErr> {
    let url = format!("{}/devices/{}/issue_token", client.base_url(), params.id);
    let request = request::Params::post(&url, request::marshal_json(params.payload)?);
    super::client::fetch(client, request).await
}

pub async fn update(client: &impl ClientI, params: UpdateParams<'_>) -> Result<Device, HTTPErr> {
    let url = format!("{}/devices/{}", client.base_url(), params.id);
    let request = request::Params::patch(&url, request::marshal_json(params.payload)?)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn create_or_fetch_device(
    client: &impl ClientI,
    params: CreateOrFetchDeviceParams<'_>,
) -> Result<Device, HTTPErr> {
    // `client.base_url()` is `{backend_host}/v1` (constructed in run_provision),
    // so the absolute URL is `{backend_host}/v1/devices`.
    let url = format!("{}/devices", client.base_url());
    let body = request::marshal_json(&CreateDeviceRequest {
        name: params.name.to_string(),
    })?;
    let post = request::Params::post(&url, body).with_api_key(params.api_key);
    match super::client::fetch::<Device>(client, post).await {
        Ok(device) => Ok(device),
        Err(HTTPErr::RequestFailed(rf)) if rf.status == reqwest::StatusCode::CONFLICT => {
            let get = request::Params::get(&url)
                .with_api_key(params.api_key)
                .with_query(QueryParams::new().add("name", params.name));
            super::client::fetch(client, get).await
        }
        Err(e) => Err(e),
    }
}

pub async fn issue_activation_token(
    client: &impl ClientI,
    params: IssueActivationTokenParams<'_>,
) -> Result<TokenResponse, HTTPErr> {
    let url = format!(
        "{}/devices/{}/activation_token",
        client.base_url(),
        params.id
    );
    let body = request::marshal_json(&IssueActivationTokenRequest {
        allow_reactivation: params.allow_reactivation,
    })?;
    let request = request::Params::post(&url, body).with_api_key(params.api_key);
    super::client::fetch(client, request).await
}
