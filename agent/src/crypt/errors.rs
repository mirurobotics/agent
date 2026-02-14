use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;

#[derive(Debug, thiserror::Error)]
#[error("Invalid JWT: {msg}")]
pub struct InvalidJWTErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidJWTErr {}

#[derive(Debug, thiserror::Error)]
#[error("Invalid JWT payload format: {msg}")]
pub struct InvalidJWTPayloadFormatErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidJWTPayloadFormatErr {}

#[derive(Debug, thiserror::Error)]
#[error("Base64 decode error: {source}")]
pub struct Base64DecodeErr {
    pub source: base64::DecodeError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for Base64DecodeErr {}

#[derive(Debug, thiserror::Error)]
#[error("Convert bytes to string error: {source}")]
pub struct ConvertBytesToStringErr {
    pub source: std::string::FromUtf8Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertBytesToStringErr {}

#[derive(Debug, thiserror::Error)]
#[error("Convert private key to PEM error: {source}")]
pub struct ConvertPrivateKeyToPEMErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertPrivateKeyToPEMErr {}

#[derive(Debug, thiserror::Error)]
#[error("Generate RSA key pair error: {source}")]
pub struct GenerateRSAKeyPairErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for GenerateRSAKeyPairErr {}

#[derive(Debug, thiserror::Error)]
#[error("Read key error: {source}")]
pub struct ReadKeyErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReadKeyErr {}

#[derive(Debug, thiserror::Error)]
#[error("RSA to PKey error: {source}")]
pub struct RSAToPKeyErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for RSAToPKeyErr {}

#[derive(Debug, thiserror::Error)]
#[error("Sign data error: {source}")]
pub struct SignDataErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SignDataErr {}

#[derive(Debug, thiserror::Error)]
#[error("Verify data error: {source}")]
pub struct VerifyDataErr {
    pub source: openssl::error::ErrorStack,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for VerifyDataErr {}

#[derive(Debug, thiserror::Error)]
pub enum CryptErr {
    #[error(transparent)]
    InvalidJWTErr(InvalidJWTErr),
    #[error(transparent)]
    InvalidJWTPayloadErr(InvalidJWTPayloadFormatErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    Base64DecodeErr(Base64DecodeErr),
    #[error(transparent)]
    ConvertBytesToStringErr(ConvertBytesToStringErr),
    #[error(transparent)]
    ConvertPrivateKeyToPEMErr(ConvertPrivateKeyToPEMErr),
    #[error(transparent)]
    GenerateRSAKeyPairErr(GenerateRSAKeyPairErr),
    #[error(transparent)]
    ReadKeyErr(ReadKeyErr),
    #[error(transparent)]
    RSAToPKeyErr(RSAToPKeyErr),
    #[error(transparent)]
    SignDataErr(SignDataErr),
    #[error(transparent)]
    VerifyDataErr(VerifyDataErr),
}

impl From<FileSysErr> for CryptErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(CryptErr {
    InvalidJWTErr,
    InvalidJWTPayloadErr,
    FileSysErr,
    Base64DecodeErr,
    ConvertBytesToStringErr,
    ConvertPrivateKeyToPEMErr,
    GenerateRSAKeyPairErr,
    ReadKeyErr,
    RSAToPKeyErr,
    SignDataErr,
    VerifyDataErr,
});
