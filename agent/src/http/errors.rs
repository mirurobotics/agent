use crate::errors::{Code, HTTPCode, Trace};
use crate::http::request;
use openapi_client::models::ErrorResponse;

#[derive(Debug, thiserror::Error)]
pub struct RequestFailed {
    pub request: request::Meta,
    pub status: reqwest::StatusCode,
    pub error: Option<ErrorResponse>,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for RequestFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let debug_msg = match &self.error {
            Some(error) => error.error.message.clone(),
            None => "unknown miru server error".to_string(),
        };
        write!(
            f,
            "request {} failed with status code {}: {}",
            self.request, self.status, debug_msg
        )
    }
}

impl crate::errors::Error for RequestFailed {
    fn code(&self) -> Code {
        match &self.error {
            Some(error) => Code::BackendError(error.error.code.clone()),
            None => Code::InternalServerError,
        }
    }

    fn http_status(&self) -> HTTPCode {
        self.status
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.error.as_ref().map(|error| {
            serde_json::to_value(&error.error.params)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()))
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub struct TimeoutErr {
    pub msg: String,
    pub request: request::Meta,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for TimeoutErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "request {} timed out after {} seconds",
            self.request,
            self.request.timeout.as_secs()
        )
    }
}

impl crate::errors::Error for TimeoutErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, PartialEq)]
pub enum ReqwestErrKind {
    Connection,
    DecodeBody,
    Other,
}

#[derive(Debug, thiserror::Error)]
pub struct ReqwestErr {
    pub kind: ReqwestErrKind,
    pub request: request::Meta,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for ReqwestErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ReqwestErrKind::Connection => write!(
                f,
                "request {} failed with network connection error: {}",
                self.request, self.source
            ),
            ReqwestErrKind::DecodeBody => write!(
                f,
                "request {} failed to decode response body: {}",
                self.request, self.source
            ),
            ReqwestErrKind::Other => write!(
                f,
                "request {} failed with reqwest crate error: {}",
                self.request, self.source
            ),
        }
    }
}

impl crate::errors::Error for ReqwestErr {
    fn is_network_connection_error(&self) -> bool {
        self.kind == ReqwestErrKind::Connection
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid header value: {source}")]
pub struct InvalidHeaderValueErr {
    pub msg: String,
    pub source: reqwest::header::InvalidHeaderValue,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidHeaderValueErr {}

#[derive(Debug, thiserror::Error)]
#[error("invalid request URL '{url}': {msg}")]
pub struct InvalidURLErr {
    pub url: String,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidURLErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to marshal JSON body: {source}")]
pub struct MarshalJSONErr {
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MarshalJSONErr {}

#[derive(Debug, thiserror::Error)]
#[error("request {request} failed to unmarshal JSON: {source}")]
pub struct UnmarshalJSONErr {
    pub request: request::Meta,
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnmarshalJSONErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to build request: {source}")]
pub struct BuildReqwestErr {
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for BuildReqwestErr {}

#[derive(Debug, thiserror::Error)]
#[error("Mock error (is network connection error: {is_network_connection_error})")]
pub struct MockErr {
    pub is_network_connection_error: bool,
}

impl crate::errors::Error for MockErr {
    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HTTPErr {
    #[error(transparent)]
    RequestFailed(RequestFailed),
    #[error(transparent)]
    TimeoutErr(TimeoutErr),
    #[error(transparent)]
    InvalidHeaderValueErr(InvalidHeaderValueErr),
    #[error(transparent)]
    InvalidURLErr(InvalidURLErr),
    #[error(transparent)]
    MarshalJSONErr(MarshalJSONErr),
    #[error(transparent)]
    UnmarshalJSONErr(UnmarshalJSONErr),
    #[error(transparent)]
    ReqwestErr(ReqwestErr),
    #[error(transparent)]
    BuildReqwestErr(BuildReqwestErr),
    #[error(transparent)]
    MockErr(MockErr),
}

crate::impl_error!(HTTPErr {
    RequestFailed,
    TimeoutErr,
    InvalidHeaderValueErr,
    InvalidURLErr,
    MarshalJSONErr,
    UnmarshalJSONErr,
    ReqwestErr,
    BuildReqwestErr,
    MockErr,
});

pub fn reqwest_err_to_http_client_err(
    e: reqwest::Error,
    meta: request::Meta,
    trace: Box<Trace>,
) -> HTTPErr {
    if e.is_timeout() {
        HTTPErr::TimeoutErr(TimeoutErr {
            msg: e.to_string(),
            request: meta,
            trace,
        })
    } else {
        let kind = if e.is_connect() {
            ReqwestErrKind::Connection
        } else if e.is_decode() {
            ReqwestErrKind::DecodeBody
        } else {
            ReqwestErrKind::Other
        };
        HTTPErr::ReqwestErr(ReqwestErr {
            kind,
            request: meta,
            source: e,
            trace,
        })
    }
}
