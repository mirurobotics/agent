// internal crates
use crate::http::{errors::HTTPErr, request, ClientI};
use backend_api::models::{CreateUploadRequest, Upload, UploadCredentials, UploadWithCredentials};

// ================================ PARAM STRUCTS ================================== //

pub struct CreateParams<'a> {
    pub payload: &'a CreateUploadRequest,
    pub token: &'a str,
}

pub struct VendCredentialsParams<'a> {
    pub id: &'a str,
    pub token: &'a str,
}

pub struct ConfirmParams<'a> {
    pub id: &'a str,
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //
pub async fn create(
    client: &impl ClientI,
    params: CreateParams<'_>,
) -> Result<UploadWithCredentials, HTTPErr> {
    let url = format!("{}/uploads", client.base_url());
    let request = request::Params::post(&url, request::marshal_json(params.payload)?)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn vend_credentials(
    client: &impl ClientI,
    params: VendCredentialsParams<'_>,
) -> Result<UploadCredentials, HTTPErr> {
    let url = format!("{}/uploads/{}/credentials", client.base_url(), params.id);
    let request = request::Params::post(&url, String::new()).with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn confirm(client: &impl ClientI, params: ConfirmParams<'_>) -> Result<Upload, HTTPErr> {
    let url = format!("{}/uploads/{}/confirm", client.base_url(), params.id);
    let request = request::Params::post(&url, String::new()).with_token(params.token);
    super::client::fetch(client, request).await
}
