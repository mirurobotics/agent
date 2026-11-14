// standard crates
use std::time::Duration;

// internal crates
use crate::http::{
    errors::HTTPErr,
    path,
    query::{Page, QueryParams, MAX_PAGE_LIMIT},
    request, ClientI,
};
use backend_api::models::{Calibration, CalibrationList, CreateCalibrationRequest};

/// Calibration content can be up to 1 MiB, so a request that carries it gets
/// more time than the default request timeout.
pub const CONTENT_TIMEOUT: Duration = Duration::from_secs(60);

// ================================ PARAM STRUCTS ================================== //

pub struct CreateParams<'a> {
    pub payload: &'a CreateCalibrationRequest,
    pub token: &'a str,
}

pub struct ListLatestParams<'a> {
    pub expansions: &'a [&'a str],
    pub pagination: &'a Page,
    pub token: &'a str,
}

pub struct ListAllLatestParams<'a> {
    pub expansions: &'a [&'a str],
    pub token: &'a str,
}

pub struct GetContentParams<'a> {
    pub id: &'a str,
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn create(
    client: &impl ClientI,
    params: CreateParams<'_>,
) -> Result<Calibration, HTTPErr> {
    let url = path::url(client.base_url(), &["calibrations"])?;
    let request = request::Params::post(&url, request::marshal_json(params.payload)?)
        .with_timeout(CONTENT_TIMEOUT)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn list_latest(
    client: &impl ClientI,
    params: ListLatestParams<'_>,
) -> Result<CalibrationList, HTTPErr> {
    let qp = QueryParams::new()
        .paginate(params.pagination)
        .expand(params.expansions);
    let url = format!("{}/calibrations/latest", client.base_url());
    let request = request::Params::get(&url)
        .with_query(qp)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn list_all_latest(
    client: &impl ClientI,
    params: ListAllLatestParams<'_>,
) -> Result<Vec<Calibration>, HTTPErr> {
    let mut all_calibrations = Vec::new();
    let mut pagination = Page {
        limit: MAX_PAGE_LIMIT,
        offset: 0,
    };

    loop {
        let page = list_latest(
            client,
            ListLatestParams {
                expansions: params.expansions,
                pagination: &pagination,
                token: params.token,
            },
        )
        .await?;
        all_calibrations.extend(page.data);
        if !page.has_more {
            break;
        }
        pagination.offset += pagination.limit;
    }

    Ok(all_calibrations)
}

pub async fn get_content(
    client: &impl ClientI,
    params: GetContentParams<'_>,
) -> Result<String, HTTPErr> {
    let url = path::url(client.base_url(), &["calibrations", params.id, "content"])?;
    let request = request::Params::get(&url)
        .with_timeout(CONTENT_TIMEOUT)
        .with_token(params.token);
    let (text, _meta) = client.execute(request).await?;
    Ok(text)
}
