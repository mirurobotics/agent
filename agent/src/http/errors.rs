// standard library
use std::fmt;

// internal crates
use crate::errors::Trace;
use crate::errors::{Code, HTTPCode, MiruError};
use crate::http::backend::BackendErrorCodes;
use crate::http::client::RequestContext;
use openapi_client::models::ErrorResponse;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

#[derive(Debug)]
pub struct RequestFailed {
    pub request: RequestContext,
    pub status: reqwest::StatusCode,
    pub error: Option<ErrorResponse>,
    pub trace: Box<Trace>,
}

impl MiruError for RequestFailed {
    fn code(&self) -> Code {
        match &self.error {
            Some(error) => Code::BackendError(error.error.code.clone()),
            None => Code::InternalServerError,
        }
    }

    fn http_status(&self) -> HTTPCode {
        self.status
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        match &self.error {
            Some(error) => error.error.params.clone(),
            None => None,
        }
    }
}

impl fmt::Display for RequestFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let debug_msg = match &self.error {
            Some(error) => error.error.debug_message.clone(),
            None => "unknown miru server error".to_string(),
        };
        write!(
            f,
            "Request {} failed with status code {}: {}",
            self.request, self.status, debug_msg
        )
    }
}

#[derive(Debug)]
pub struct TimeoutErr {
    pub msg: String,
    pub request: RequestContext,
    pub trace: Box<Trace>,
}

impl MiruError for TimeoutErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        true
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for TimeoutErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Request {} timed out after {} seconds",
            self.request,
            self.request.timeout.as_secs()
        )
    }
}

#[derive(Debug)]
pub struct CacheErr {
    pub msg: String,
    pub code: Code,
    pub http_status: HTTPCode,
    pub is_network_connection_error: bool,
    pub params: Option<serde_json::Value>,
    pub trace: Box<Trace>,
}

impl MiruError for CacheErr {
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

impl fmt::Display for CacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP request cache error: {}", self.msg)
    }
}

#[derive(Debug)]
pub struct ConnectionErr {
    pub request: RequestContext,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl MiruError for ConnectionErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        true
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ConnectionErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Request {} failed with network connection error: {}",
            self.request, self.source
        )
    }
}

#[derive(Debug)]
pub struct DecodeRespBodyErr {
    pub request: RequestContext,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl MiruError for DecodeRespBodyErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DecodeRespBodyErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Request {} failed to decode response body: {}",
            self.request, self.source
        )
    }
}

#[derive(Debug)]
pub struct InvalidHeaderValueErr {
    pub msg: String,
    pub source: reqwest::header::InvalidHeaderValue,
    pub trace: Box<Trace>,
}

impl MiruError for InvalidHeaderValueErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for InvalidHeaderValueErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid header value: {}", self.source)
    }
}

#[derive(Debug)]
pub struct MarshalJSONErr {
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl MiruError for MarshalJSONErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for MarshalJSONErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to marshal JSON body: {}", self.source)
    }
}

#[derive(Debug)]
pub struct UnmarshalJSONErr {
    pub request: RequestContext,
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl MiruError for UnmarshalJSONErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnmarshalJSONErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Request {} failed to unmarshal JSON: {}",
            self.request, self.source
        )
    }
}

#[derive(Debug)]
pub struct BuildReqwestErr {
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl MiruError for BuildReqwestErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for BuildReqwestErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to build request: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ReqwestErr {
    pub request: RequestContext,
    pub source: reqwest::Error,
    pub trace: Box<Trace>,
}

impl MiruError for ReqwestErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ReqwestErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Request {} failed with reqwest crate error: {}",
            self.request, self.source
        )
    }
}

#[derive(Debug)]
pub struct MockErr {
    pub is_network_connection_error: bool,
}

impl MiruError for MockErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for MockErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Mock error (is network connection error: {})",
            self.is_network_connection_error
        )
    }
}

#[derive(Debug)]
pub enum HTTPErr {
    // HTTP errors
    RequestFailed(Box<RequestFailed>),
    TimeoutErr(Box<TimeoutErr>),
    CacheErr(Box<CacheErr>),

    // external crate errors
    ConnectionErr(Box<ConnectionErr>),
    DecodeRespBodyErr(Box<DecodeRespBodyErr>),
    InvalidHeaderValueErr(Box<InvalidHeaderValueErr>),
    MarshalJSONErr(Box<MarshalJSONErr>),
    UnmarshalJSONErr(Box<UnmarshalJSONErr>),
    ReqwestErr(Box<ReqwestErr>),
    BuildReqwestErr(Box<BuildReqwestErr>),

    // mock errors (not for production use)
    MockErr(Box<MockErr>),
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

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::RequestFailed(e) => e.$method($($arg)?),
            Self::TimeoutErr(e) => e.$method($($arg)?),
            Self::CacheErr(e) => e.$method($($arg)?),
            Self::ConnectionErr(e) => e.$method($($arg)?),
            Self::DecodeRespBodyErr(e) => e.$method($($arg)?),
            Self::InvalidHeaderValueErr(e) => e.$method($($arg)?),
            Self::MarshalJSONErr(e) => e.$method($($arg)?),
            Self::UnmarshalJSONErr(e) => e.$method($($arg)?),
            Self::ReqwestErr(e) => e.$method($($arg)?),
            Self::BuildReqwestErr(e) => e.$method($($arg)?),
            Self::MockErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for HTTPErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl MiruError for HTTPErr {
    fn code(&self) -> Code {
        forward_error_method!(self, code)
    }

    fn http_status(&self) -> HTTPCode {
        forward_error_method!(self, http_status)
    }

    fn is_network_connection_error(&self) -> bool {
        forward_error_method!(self, is_network_connection_error)
    }

    fn params(&self) -> Option<serde_json::Value> {
        forward_error_method!(self, params)
    }
}

pub fn reqwest_err_to_http_client_err(
    e: reqwest::Error,
    context: &RequestContext,
    trace: Box<Trace>,
) -> HTTPErr {
    if e.is_connect() {
        HTTPErr::ConnectionErr(Box::new(ConnectionErr {
            request: context.clone(),
            source: e,
            trace,
        }))
    } else if e.is_decode() {
        HTTPErr::DecodeRespBodyErr(Box::new(DecodeRespBodyErr {
            request: context.clone(),
            source: e,
            trace,
        }))
    } else if e.is_timeout() {
        HTTPErr::TimeoutErr(Box::new(TimeoutErr {
            msg: e.to_string(),
            request: context.clone(),
            trace,
        }))
    } else {
        HTTPErr::ReqwestErr(Box::new(ReqwestErr {
            request: context.clone(),
            source: e,
            trace,
        }))
    }
}
