// internal crates
use super::errors::HTTPErr;
use super::request;
use super::ClientI;
use backend_api::models::{
    ActivateDeviceRequest, Device, IssueDeviceTokenRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};

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
