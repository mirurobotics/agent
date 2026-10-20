// standard crates
use std::time::Duration;

// internal crates
use crate::http::{errors::HTTPErr, path, request, ClientI};
use backend_api::models::{Calibration, CreateCalibrationRequest};

/// Calibration content can be up to 1 MiB, so creating one gets more time than
/// the default request timeout.
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(60);

// ================================ PARAM STRUCTS ================================== //

pub struct CreateParams<'a> {
    pub payload: &'a CreateCalibrationRequest,
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn create(
    client: &impl ClientI,
    params: CreateParams<'_>,
) -> Result<Calibration, HTTPErr> {
    let url = path::url(client.base_url(), &["calibrations"])?;
    let request = request::Params::post(&url, request::marshal_json(params.payload)?)
        .with_timeout(CREATE_TIMEOUT)
        .with_token(params.token);
    super::client::fetch(client, request).await
}
