// standard library
use std::fmt;

// internal crates
use crate::errors::Trace;
use crate::errors::{Code, HTTPCode, Error};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

#[derive(Debug)]
pub struct AuthenticationErr {
    pub source: rumqttc::ConnectionError,
    pub trace: Box<Trace>,
}

impl Error for AuthenticationErr {
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

impl fmt::Display for AuthenticationErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to authenticate to MQTT broker: {}", self.source)
    }
}

#[derive(Debug)]
pub struct NetworkConnectionErr {
    pub source: rumqttc::ConnectionError,
    pub trace: Box<Trace>,
}

impl Error for NetworkConnectionErr {
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

impl fmt::Display for NetworkConnectionErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to connect to MQTT broker: {}", self.source)
    }
}

#[derive(Debug)]
pub struct PollErr {
    pub source: rumqttc::ConnectionError,
    pub trace: Box<Trace>,
}

impl Error for PollErr {
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

impl fmt::Display for PollErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to poll event loop: {}", self.source)
    }
}

#[derive(Debug)]
pub struct TimeoutErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl Error for TimeoutErr {
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
        write!(f, "Request timed out: {}", self.msg)
    }
}

#[derive(Debug)]
pub struct PublishErr {
    pub source: rumqttc::ClientError,
    pub trace: Box<Trace>,
}

impl Error for PublishErr {
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

impl fmt::Display for PublishErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to publish message: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SerdeErr {
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl Error for SerdeErr {
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

impl fmt::Display for SerdeErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Serialization error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct MockErr {
    pub is_authentication_error: bool,
    pub is_network_connection_error: bool,
}

impl Error for MockErr {
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
            "Mock MQTT error (is authentication error: {}, is network connection error: {})",
            self.is_authentication_error, self.is_network_connection_error
        )
    }
}

#[derive(Debug)]
pub enum MQTTError {
    AuthenticationErr(Box<AuthenticationErr>),
    NetworkConnectionErr(Box<NetworkConnectionErr>),
    TimeoutErr(Box<TimeoutErr>),
    PollErr(Box<PollErr>),
    PublishErr(Box<PublishErr>),
    SerdeErr(Box<SerdeErr>),

    MockErr(Box<MockErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            MQTTError::AuthenticationErr(e) => e.$method($($arg)?),
            MQTTError::NetworkConnectionErr(e) => e.$method($($arg)?),
            MQTTError::TimeoutErr(e) => e.$method($($arg)?),
            MQTTError::PollErr(e) => e.$method($($arg)?),
            MQTTError::PublishErr(e) => e.$method($($arg)?),
            MQTTError::SerdeErr(e) => e.$method($($arg)?),
            MQTTError::MockErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for MQTTError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for MQTTError {
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

impl MQTTError {
    pub fn is_authentication_error(&self) -> bool {
        match self {
            MQTTError::AuthenticationErr(_) => true,
            MQTTError::MockErr(e) => e.is_authentication_error,
            _ => false,
        }
    }
}
