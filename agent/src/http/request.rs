// standard library
use std::fmt;

// internal crates
use crate::http::errors::{
    BuildReqwestErr, HTTPErr, InvalidHeaderValueErr, InvalidURLErr, MarshalJSONErr,
};
use crate::http::query::QueryParams;
use crate::telemetry::SystemInfo;
use crate::trace;
use crate::version;
// external crates
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Serialize;
use tokio::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub struct Params<'a> {
    pub method: reqwest::Method,
    pub url: &'a str,
    pub query: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout: Duration,
    pub token: Option<&'a str>,
}

impl<'a> Params<'a> {
    pub fn meta(&self) -> Result<Meta, HTTPErr> {
        Ok(Meta {
            url: self.url_with_query()?,
            method: self.method.clone(),
            timeout: self.timeout,
        })
    }

    pub fn url_with_query(&self) -> Result<String, HTTPErr> {
        if self.query.is_empty() {
            return Ok(self.url.to_string());
        }

        let pairs: Vec<(&str, &str)> = self
            .query
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        reqwest::Url::parse_with_params(self.url, &pairs)
            .map(|url| url.to_string())
            .map_err(|source| {
                HTTPErr::InvalidURLErr(InvalidURLErr {
                    url: self.url.to_string(),
                    msg: source.to_string(),
                    trace: trace!(),
                })
            })
    }

    pub fn get(url: &'a str) -> Self {
        Self {
            method: reqwest::Method::GET,
            url,
            query: Vec::new(),
            body: None,
            timeout: DEFAULT_TIMEOUT,
            token: None,
        }
    }

    pub fn post(url: &'a str, body: String) -> Self {
        Self {
            method: reqwest::Method::POST,
            url,
            query: Vec::new(),
            body: Some(body),
            timeout: DEFAULT_TIMEOUT,
            token: None,
        }
    }

    pub fn patch(url: &'a str, body: String) -> Self {
        Self {
            method: reqwest::Method::PATCH,
            url,
            query: Vec::new(),
            body: Some(body),
            timeout: DEFAULT_TIMEOUT,
            token: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_token(mut self, token: &'a str) -> Self {
        self.token = Some(token);
        self
    }

    pub fn with_query(mut self, qp: QueryParams) -> Self {
        self.query = qp.into_pairs();
        self
    }
}

pub struct Request {
    pub meta: Meta,
    pub reqwest: reqwest::Request,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub url: String,
    pub method: reqwest::Method,
    pub timeout: Duration,
}

impl fmt::Display for Meta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} (timeout: {}ms)",
            self.method,
            self.url,
            self.timeout.as_millis(),
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
    pub fn to_map(&self) -> Result<HeaderMap, HTTPErr> {
        let mut headers = HeaderMap::new();
        insert_header(&mut headers, "X-Miru-Agent-Version", &self.agent_version)?;
        insert_header(&mut headers, "X-Miru-API-Version", &self.api_version)?;
        insert_header(&mut headers, "X-Host-Name", &self.host_name)?;
        insert_header(&mut headers, "X-Arch", &self.arch)?;
        insert_header(&mut headers, "X-Language", &self.language)?;
        insert_header(&mut headers, "X-OS", &self.os)?;
        Ok(headers)
    }
}

fn insert_header(headers: &mut HeaderMap, key: &'static str, value: &str) -> Result<(), HTTPErr> {
    match HeaderValue::from_str(value) {
        Ok(value) => {
            headers.insert(key, value);
            Ok(())
        }
        Err(source) => Err(HTTPErr::InvalidHeaderValueErr(InvalidHeaderValueErr {
            msg: format!("failed to set header {key}: {source}"),
            source,
            trace: trace!(),
        })),
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

pub fn build(
    client: &reqwest::Client,
    headers: &Headers,
    params: Params,
) -> Result<Request, HTTPErr> {
    let meta = params.meta()?;
    let mut request = client.request(params.method.clone(), params.url);

    // query params
    if !params.query.is_empty() {
        request = request.query(&params.query);
    }
    // headers
    let mut header_map = headers.to_map()?;
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
    Ok(Request { meta, reqwest })
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
