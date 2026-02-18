// internal crates
use crate::http::errors::HTTPErr;
use crate::http::request::{self, Params};
use crate::http::response;
use crate::http::ClientI;
use openapi_client::models::{
    ActivateDeviceRequest, Device, IssueDeviceTokenRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};

// ================================ PARAM STRUCTS ================================== //

pub struct ActivateParams<'a> {
    pub device_id: &'a str,
    pub payload: &'a ActivateDeviceRequest,
    pub token: &'a str,
}

pub struct IssueTokenParams<'a> {
    pub device_id: &'a str,
    pub payload: &'a IssueDeviceTokenRequest,
}

pub struct UpdateParams<'a> {
    pub device_id: &'a str,
    pub payload: &'a UpdateDeviceFromAgentRequest,
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn activate(
    client: &impl ClientI,
    params: ActivateParams<'_>,
) -> Result<Device, HTTPErr> {
    let url = format!(
        "{}/devices/{}/activate",
        client.base_url(),
        params.device_id
    );
    let (text, context) = client
        .execute(
            Params::post(
                &url,
                request::marshal_json(params.payload)?,
                client.default_timeout(),
            )
            .with_token(params.token),
        )
        .await?;
    response::parse_json(text, &context)
}

pub async fn issue_token(
    client: &impl ClientI,
    params: IssueTokenParams<'_>,
) -> Result<TokenResponse, HTTPErr> {
    let url = format!(
        "{}/devices/{}/issue_token",
        client.base_url(),
        params.device_id
    );
    let (text, context) = client
        .execute(Params::post(
            &url,
            request::marshal_json(params.payload)?,
            client.default_timeout(),
        ))
        .await?;
    response::parse_json(text, &context)
}

pub async fn update(client: &impl ClientI, params: UpdateParams<'_>) -> Result<Device, HTTPErr> {
    let url = format!("{}/devices/{}", client.base_url(), params.device_id);
    let (text, context) = client
        .execute(
            Params::patch(
                &url,
                request::marshal_json(params.payload)?,
                client.default_timeout(),
            )
            .with_token(params.token),
        )
        .await?;
    response::parse_json(text, &context)
}
