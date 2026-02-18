use crate::errors::{Code, HTTPCode, Trace};
use crate::http::backend::BackendErrorCodes;
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

#[derive(Debug, thiserror::Error)]
#[error("HTTP request cache error: {msg}")]
pub struct CacheErr {
    pub msg: String,
    pub code: Code,
    pub http_status: HTTPCode,
    pub is_network_connection_error: bool,
    pub params: Option<serde_json::Value>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CacheErr {
    fn code(&self) -> Code {
        self.code.clone()
    }

    fn http_status(&self) -> HTTPCode {
        self.http_status
    }

    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.params.clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("request {request} failed with network connection error: {source}")]
pub struct ConnectionErr {
    pub request: request::Meta,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConnectionErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("request {request} failed to decode response body: {source}")]
pub struct DecodeRespBodyErr {
    pub request: request::Meta,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DecodeRespBodyErr {}

#[derive(Debug, thiserror::Error)]
#[error("invalid header value: {source}")]
pub struct InvalidHeaderValueErr {
    pub msg: String,
    pub source: reqwest::header::InvalidHeaderValue,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidHeaderValueErr {}

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
#[error("request {request} failed with reqwest crate error: {source}")]
pub struct ReqwestErr {
    pub request: request::Meta,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReqwestErr {}

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
    CacheErr(CacheErr),
    #[error(transparent)]
    ConnectionErr(ConnectionErr),
    #[error(transparent)]
    DecodeRespBodyErr(DecodeRespBodyErr),
    #[error(transparent)]
    InvalidHeaderValueErr(InvalidHeaderValueErr),
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

impl HTTPErr {
    pub fn is_invalid_token_error(&self) -> bool {
        match self {
            HTTPErr::RequestFailed(e) => match &e.error {
                Some(error) => error.error.code == BackendErrorCodes::InvalidJWTAuth.as_str(),
                None => false,
            },
            _ => false,
        }
    }
}

crate::impl_error!(HTTPErr {
    RequestFailed,
    TimeoutErr,
    CacheErr,
    ConnectionErr,
    DecodeRespBodyErr,
    InvalidHeaderValueErr,
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
    if e.is_connect() {
        HTTPErr::ConnectionErr(ConnectionErr {
            request: meta,
            source: e,
            trace,
        })
    } else if e.is_decode() {
        HTTPErr::DecodeRespBodyErr(DecodeRespBodyErr {
            request: meta,
            source: e,
            trace,
        })
    } else if e.is_timeout() {
        HTTPErr::TimeoutErr(TimeoutErr {
            msg: e.to_string(),
            request: meta,
            trace,
        })
    } else {
        HTTPErr::ReqwestErr(ReqwestErr {
            request: meta,
            source: e,
            trace,
        })
    }
}
