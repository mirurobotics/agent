// internal crates
use crate::errors::Trace;
use crate::filesys;

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
#[error("Convert private key to PEM error: {msg}")]
pub struct ConvertPrivateKeyToPEMErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertPrivateKeyToPEMErr {}

#[derive(Debug, thiserror::Error)]
#[error("Convert public key to PEM error: {msg}")]
pub struct ConvertPublicKeyToPEMErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertPublicKeyToPEMErr {}

#[derive(Debug, thiserror::Error)]
#[error("Convert public key to DER error: {source}")]
pub struct ConvertPublicKeyToDERErr {
    pub source: aws_lc_rs::error::Unspecified,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertPublicKeyToDERErr {}

#[derive(Debug, thiserror::Error)]
#[error("Generate RSA key pair error: {msg}")]
pub struct GenerateRSAKeyPairErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for GenerateRSAKeyPairErr {}

#[derive(Debug, thiserror::Error)]
#[error("Read key error: {msg}")]
pub struct ReadKeyErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReadKeyErr {}

#[derive(Debug, thiserror::Error)]
#[error("Sign data error: {source}")]
pub struct SignDataErr {
    pub source: aws_lc_rs::error::Unspecified,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SignDataErr {}

#[derive(Debug, thiserror::Error)]
pub enum CryptErr {
    #[error(transparent)]
    InvalidJWTErr(InvalidJWTErr),
    #[error(transparent)]
    InvalidJWTPayloadErr(InvalidJWTPayloadFormatErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    Base64DecodeErr(Base64DecodeErr),
    #[error(transparent)]
    ConvertBytesToStringErr(ConvertBytesToStringErr),
    #[error(transparent)]
    ConvertPrivateKeyToPEMErr(ConvertPrivateKeyToPEMErr),
    #[error(transparent)]
    ConvertPublicKeyToPEMErr(ConvertPublicKeyToPEMErr),
    #[error(transparent)]
    ConvertPublicKeyToDERErr(ConvertPublicKeyToDERErr),
    #[error(transparent)]
    GenerateRSAKeyPairErr(GenerateRSAKeyPairErr),
    #[error(transparent)]
    ReadKeyErr(ReadKeyErr),
    #[error(transparent)]
    SignDataErr(SignDataErr),
}

impl From<filesys::FileSysErr> for CryptErr {
    fn from(e: filesys::FileSysErr) -> Self {
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
    ConvertPublicKeyToPEMErr,
    ConvertPublicKeyToDERErr,
    GenerateRSAKeyPairErr,
    ReadKeyErr,
    SignDataErr,
});
