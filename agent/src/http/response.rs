// internal crates
use crate::http::{
    errors::{reqwest_err_to_http_client_err, HTTPErr, RequestFailed, UnmarshalJSONErr},
    request::Context,
};
use crate::trace;
use openapi_client::models::ErrorResponse;

// external crates
use serde::de::DeserializeOwned;

pub async fn handle(response: reqwest::Response, context: &Context) -> Result<String, HTTPErr> {
    let status = response.status();

    // check for an error response
    if !status.is_success() {
        let error_response = match response.text().await {
            Ok(text) => parse_json::<ErrorResponse>(text, context).ok(),
            Err(_) => None,
        };
        return Err(HTTPErr::RequestFailed(RequestFailed {
            request: context.clone(),
            status,
            error: error_response,
            trace: trace!(),
        }));
    }

    let text = response
        .text()
        .await
        .map_err(|e| reqwest_err_to_http_client_err(e, context, trace!()))?;
    Ok(text)
}

pub fn parse_json<T>(text: String, context: &Context) -> Result<T, HTTPErr>
where
    T: DeserializeOwned,
{
    serde_json::from_str::<T>(&text).map_err(|e| {
        HTTPErr::UnmarshalJSONErr(UnmarshalJSONErr {
            request: context.clone(),
            source: e,
            trace: trace!(),
        })
    })
}
