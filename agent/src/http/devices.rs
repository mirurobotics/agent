// internal crates
use crate::http::{errors::HTTPErr, request, ClientI};
use backend_api::models::{
    Device, IssueDeviceTokenRequest, ProvisionDeviceRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};

// ================================ PARAM STRUCTS ================================== //

pub struct ProvisionParams<'a> {
    pub payload: &'a ProvisionDeviceRequest,
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

pub async fn provision(
    client: &impl ClientI,
    params: ProvisionParams<'_>,
) -> Result<Device, HTTPErr> {
    let url = format!("{}/devices/provision", client.base_url());
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
