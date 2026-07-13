// internal crates
use crate::http::{errors::HTTPErr, request, ClientI};
use backend_api::models::{CreateUploadRequest, Upload, UploadCredentials, UploadWithCredentials};

// ================================ PARAM STRUCTS ================================== //

pub struct CreateParams<'a> {
    pub payload: &'a CreateUploadRequest,
    pub token: &'a str,
}

pub struct CredentialsParams<'a> {
    pub upload_id: &'a str,
    pub token: &'a str,
}

pub struct ConfirmParams<'a> {
    pub upload_id: &'a str,
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //

/// Create an upload: the backend verifies the device, creates the ledger entry,
/// authorizes the object key, dedups by digest, and vends short-lived downscoped
/// cloud credentials. The device then uploads via the native SDK and confirms.
pub async fn create(
    client: &impl ClientI,
    params: CreateParams<'_>,
) -> Result<UploadWithCredentials, HTTPErr> {
    let url = format!("{}/uploads", client.base_url());
    let request = request::Params::post(&url, request::marshal_json(params.payload)?)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

/// Re-vend downscoped credentials for an in-progress upload, e.g. after the
/// previous credentials expired mid-transfer.
pub async fn vend_credentials(
    client: &impl ClientI,
    params: CredentialsParams<'_>,
) -> Result<UploadCredentials, HTTPErr> {
    let url = format!(
        "{}/uploads/{}/credentials",
        client.base_url(),
        params.upload_id
    );
    let request = request::Params::post(&url, String::new()).with_token(params.token);
    super::client::fetch(client, request).await
}

/// Confirm that the device durably wrote the upload to the destination bucket.
pub async fn confirm(client: &impl ClientI, params: ConfirmParams<'_>) -> Result<Upload, HTTPErr> {
    let url = format!("{}/uploads/{}/confirm", client.base_url(), params.upload_id);
    let request = request::Params::post(&url, String::new()).with_token(params.token);
    super::client::fetch(client, request).await
}
