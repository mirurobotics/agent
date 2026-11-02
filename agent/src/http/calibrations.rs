// standard crates
use std::time::Duration;

// internal crates
use crate::http::{errors::HTTPErr, path, query::QueryParams, request, ClientI};
use backend_api::models::{Calibration, CalibrationList, CreateCalibrationRequest};

/// Calibration content can be up to 1 MiB, so creating one gets more time than
/// the default request timeout.
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(60);

// ================================ PARAM STRUCTS ================================== //

pub struct CreateParams<'a> {
    pub payload: &'a CreateCalibrationRequest,
    pub token: &'a str,
}

pub struct ListParams<'a> {
    pub query: &'a [(&'static str, String)],
    pub token: &'a str,
}

pub struct GetParams<'a> {
    pub id: &'a str,
    pub query: &'a [(&'static str, String)],
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

pub async fn list(
    client: &impl ClientI,
    params: ListParams<'_>,
) -> Result<CalibrationList, HTTPErr> {
    let url = path::url(client.base_url(), &["calibrations"])?;
    let request = request::Params::get(&url)
        .with_query(to_query_params(params.query))
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn get(client: &impl ClientI, params: GetParams<'_>) -> Result<Calibration, HTTPErr> {
    let url = path::url(client.base_url(), &["calibrations", params.id])?;
    let request = request::Params::get(&url)
        .with_query(to_query_params(params.query))
        .with_token(params.token);
    super::client::fetch(client, request).await
}

fn to_query_params(query: &[(&'static str, String)]) -> QueryParams {
    query
        .iter()
        .fold(QueryParams::new(), |qp, (key, value)| qp.add(*key, value))
}
