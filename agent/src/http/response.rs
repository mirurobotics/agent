// internal crates
use crate::http::{
    errors::{reqwest_err_to_http_client_err, HTTPErr, RequestFailed, UnmarshalJSONErr},
    request,
};
use crate::trace;
use openapi_client::models::ErrorResponse;

// external crates
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub struct Response {
    pub reqwest: reqwest::Response,
    pub meta: request::Meta,
}

pub async fn handle(resp: Response) -> Result<String, HTTPErr> {
    let status = resp.reqwest.status();
    let meta = resp.meta;

    if !status.is_success() {
        let error_response = match resp.reqwest.text().await {
            Ok(text) => parse_json::<ErrorResponse>(text, meta.clone()).ok(),
            Err(_) => None,
        };
        Err(HTTPErr::RequestFailed(RequestFailed {
            request: meta,
            status,
            error: error_response,
            trace: trace!(),
        }))
    } else {
        match resp.reqwest.text().await {
            Ok(text) => Ok(text),
            Err(e) => Err(reqwest_err_to_http_client_err(e, meta, trace!())),
        }
    }
}

pub fn parse_json<T>(text: String, meta: request::Meta) -> Result<T, HTTPErr>
where
    T: DeserializeOwned,
{
    serde_json::from_str::<T>(&text).map_err(|e| {
        HTTPErr::UnmarshalJSONErr(UnmarshalJSONErr {
            request: meta,
            source: e,
            trace: trace!(),
        })
    })
}
