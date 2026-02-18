// internal crates
use crate::http::{
    errors::{reqwest_err_to_http_client_err, HTTPErr, RequestFailed, UnmarshalJSONErr},
    request,
};
use crate::trace;
use openapi_client::models::ErrorResponse;

// external crates
use serde::de::DeserializeOwned;

pub async fn handle(response: reqwest::Response, meta: request::Meta) -> Result<String, HTTPErr> {
    let status = response.status();

    if !status.is_success() {
        let error_response = match response.text().await {
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
        let text = response
            .text()
            .await
            .map_err(|e| reqwest_err_to_http_client_err(e, meta, trace!()))?;
        Ok(text)
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
