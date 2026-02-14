// standard library
use std::fmt;

// internal crates
use crate::errors::{Code, HTTPCode, Error, Trace};
use crate::filesys::errors::FileSysErr;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

#[derive(Debug)]
pub struct InvalidJWTErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl Error for InvalidJWTErr {
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

impl fmt::Display for InvalidJWTErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid JWT: {}", self.msg)
    }
}

#[derive(Debug)]
pub struct InvalidJWTPayloadFormatErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl Error for InvalidJWTPayloadFormatErr {
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

impl fmt::Display for InvalidJWTPayloadFormatErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid JWT payload format: {}", self.msg)
    }
}

#[derive(Debug)]
pub struct CryptFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl Error for CryptFileSysErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for CryptFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "File system error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct Base64DecodeErr {
    pub source: base64::DecodeError,
    pub trace: Box<Trace>,
}

impl Error for Base64DecodeErr {
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

impl fmt::Display for Base64DecodeErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Base64 decode error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ConvertBytesToStringErr {
    pub source: std::string::FromUtf8Error,
    pub trace: Box<Trace>,
}

impl Error for ConvertBytesToStringErr {
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

impl fmt::Display for ConvertBytesToStringErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Convert bytes to string error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ConvertPrivateKeyToPEMErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl Error for ConvertPrivateKeyToPEMErr {
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

impl fmt::Display for ConvertPrivateKeyToPEMErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Convert private key to PEM error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct GenerateRSAKeyPairErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl Error for GenerateRSAKeyPairErr {
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

impl fmt::Display for GenerateRSAKeyPairErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Generate RSA key pair error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ReadKeyErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl Error for ReadKeyErr {
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

impl fmt::Display for ReadKeyErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Read key error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct RSAToPKeyErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl Error for RSAToPKeyErr {
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

impl fmt::Display for RSAToPKeyErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RSA to PKey error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SignDataErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl Error for SignDataErr {
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

impl fmt::Display for SignDataErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sign data error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct VerifyDataErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl Error for VerifyDataErr {
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

impl fmt::Display for VerifyDataErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Verify data error: {}", self.source)
    }
}

#[derive(Debug)]
pub enum CryptErr {
    // crate errors
    InvalidJWTErr(Box<InvalidJWTErr>),
    InvalidJWTPayloadErr(Box<InvalidJWTPayloadFormatErr>),

    // internal crate errors
    FileSysErr(Box<CryptFileSysErr>),

    // external crate errors
    Base64DecodeErr(Box<Base64DecodeErr>),
    ConvertBytesToStringErr(Box<ConvertBytesToStringErr>),
    ConvertPrivateKeyToPEMErr(Box<ConvertPrivateKeyToPEMErr>),
    GenerateRSAKeyPairErr(Box<GenerateRSAKeyPairErr>),
    ReadKeyErr(Box<ReadKeyErr>),
    RSAToPKeyErr(Box<RSAToPKeyErr>),
    SignDataErr(Box<SignDataErr>),
    VerifyDataErr(Box<VerifyDataErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::InvalidJWTErr(e) => e.$method($($arg)?),
            Self::InvalidJWTPayloadErr(e) => e.$method($($arg)?),
            Self::FileSysErr(e) => e.$method($($arg)?),
            Self::Base64DecodeErr(e) => e.$method($($arg)?),
            Self::ConvertBytesToStringErr(e) => e.$method($($arg)?),
            Self::ConvertPrivateKeyToPEMErr(e) => e.$method($($arg)?),
            Self::GenerateRSAKeyPairErr(e) => e.$method($($arg)?),
            Self::ReadKeyErr(e) => e.$method($($arg)?),
            Self::RSAToPKeyErr(e) => e.$method($($arg)?),
            Self::SignDataErr(e) => e.$method($($arg)?),
            Self::VerifyDataErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for CryptErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for CryptErr {
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
