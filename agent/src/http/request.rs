// standard library
use std::fmt;

// internal crates
use crate::http::errors::{BuildReqwestErr, HTTPErr, InvalidHeaderValueErr, MarshalJSONErr};
use crate::telemetry::SystemInfo;
use crate::trace;
use crate::version;

// external crates
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Serialize;
use tokio::time::Duration;

#[derive(Clone, Debug)]
pub struct Params<'a> {
    pub method: reqwest::Method,
    pub url: &'a str,
    pub body: Option<String>,
    pub timeout: Duration,
    pub token: Option<&'a str>,
}

impl<'a> Params<'a> {
    pub fn get(url: &'a str, timeout: Duration) -> Self {
        Self {
            method: reqwest::Method::GET,
            url,
            body: None,
            timeout,
            token: None,
        }
    }

    pub fn post(url: &'a str, body: String, timeout: Duration) -> Self {
        Self {
            method: reqwest::Method::POST,
            url,
            body: Some(body),
            timeout,
            token: None,
        }
    }

    pub fn patch(url: &'a str, body: String, timeout: Duration) -> Self {
        Self {
            method: reqwest::Method::PATCH,
            url,
            body: Some(body),
            timeout,
            token: None,
        }
    }

    pub fn with_token(mut self, token: &'a str) -> Self {
        self.token = Some(token);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Context {
    pub url: String,
    pub method: reqwest::Method,
    pub timeout: Duration,
}

// Context is safe to send between threads since all fields are Send + Sync
impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} (timeout: {}ms)",
            self.method,
            self.url,
            self.timeout.as_millis()
        )
    }
}

#[derive(Debug)]
pub struct Headers {
    // build information
    pub agent_version: String,
    pub api_version: String,

    // host information
    pub host_name: String,
    pub arch: String,
    pub language: String,
    pub os: String,
}

impl Default for Headers {
    fn default() -> Self {
        Self {
            // build information
            agent_version: version::VERSION.to_string(),
            api_version: openapi_client::models::ApiVersion::API_VERSION.to_string(),

            // host information
            host_name: SystemInfo::host_name(),
            arch: SystemInfo::arch(),
            language: "rust".to_string(),
            os: SystemInfo::os(),
        }
    }
}

impl Headers {
    pub fn to_map(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&self.agent_version) {
            headers.insert("X-Miru-Agent-Version", value);
        }
        if let Ok(value) = HeaderValue::from_str(&self.api_version) {
            headers.insert("X-Miru-API-Version", value);
        }
        if let Ok(value) = HeaderValue::from_str(&self.host_name) {
            headers.insert("X-Host-Name", value);
        }
        if let Ok(value) = HeaderValue::from_str(&self.arch) {
            headers.insert("X-Arch", value);
        }
        if let Ok(value) = HeaderValue::from_str(&self.language) {
            headers.insert("X-Language", value);
        }
        if let Ok(value) = HeaderValue::from_str(&self.os) {
            headers.insert("X-OS", value);
        }
        headers
    }
}

pub fn marshal_json<T>(payload: &T) -> Result<String, HTTPErr>
where
    T: Serialize,
{
    serde_json::to_string(payload).map_err(|e| {
        HTTPErr::MarshalJSONErr(MarshalJSONErr {
            source: e,
            trace: trace!(),
        })
    })
}

fn add_token_to_headers(headers: &mut HeaderMap, token: &str) -> Result<(), HTTPErr> {
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| {
            HTTPErr::InvalidHeaderValueErr(InvalidHeaderValueErr {
                msg: e.to_string(),
                source: e,
                trace: trace!(),
            })
        })?,
    );
    Ok(())
}

pub fn build(
    client: &reqwest::Client,
    headers: &Headers,
    params: Params,
) -> Result<(reqwest::Request, Context), HTTPErr> {
    // request type (GET, POST, etc.)
    let mut request = client.request(params.method.clone(), params.url);

    // headers
    let mut header_map = headers.to_map();
    if let Some(token) = params.token {
        add_token_to_headers(&mut header_map, token)?;
    }
    request = request.headers(header_map);

    // body
    if let Some(body) = params.body {
        request = request.body(body);
    }

    // timeout
    request = request.timeout(params.timeout);

    // build
    let reqwest = request.build().map_err(|e| {
        HTTPErr::BuildReqwestErr(BuildReqwestErr {
            source: e,
            trace: trace!(),
        })
    })?;
    Ok((
        reqwest,
        Context {
            url: params.url.to_string(),
            method: params.method,
            timeout: params.timeout,
        },
    ))
}
