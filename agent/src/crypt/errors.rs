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
#[error("Convert private key to DER error: {source}")]
pub struct ConvertPrivateKeyToDERErr {
    pub source: aws_lc_rs::error::Unspecified,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertPrivateKeyToDERErr {}

#[derive(Debug, thiserror::Error)]
#[error("Convert private key to PEM error: {source}")]
pub struct ConvertPrivateKeyToPEMErr {
    pub source: pem_rfc7468::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertPrivateKeyToPEMErr {}

#[derive(Debug, thiserror::Error)]
#[error("Convert public key to PEM error: {source}")]
pub struct ConvertPublicKeyToPEMErr {
    pub source: pem_rfc7468::Error,
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
#[error("Generate RSA key pair error: {source}")]
pub struct GenerateRSAKeyPairErr {
    pub source: aws_lc_rs::error::Unspecified,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for GenerateRSAKeyPairErr {}

#[derive(Debug, thiserror::Error)]
#[error("Decode PEM error: {source}")]
pub struct DecodePEMErr {
    pub source: pem_rfc7468::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DecodePEMErr {}

#[derive(Debug, thiserror::Error)]
#[error("Unsupported PEM label: {label}")]
pub struct UnsupportedPEMLabelErr {
    pub label: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnsupportedPEMLabelErr {}

#[derive(Debug, thiserror::Error)]
#[error("Parse private key error: {source}")]
pub struct ParsePrivateKeyErr {
    pub source: aws_lc_rs::error::KeyRejected,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ParsePrivateKeyErr {}

#[derive(Debug, thiserror::Error)]
#[error("Parse public key error: {source}")]
pub struct ParsePublicKeyErr {
    pub source: aws_lc_rs::error::KeyRejected,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ParsePublicKeyErr {}

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
    ConvertPrivateKeyToDERErr(ConvertPrivateKeyToDERErr),
    #[error(transparent)]
    ConvertPrivateKeyToPEMErr(ConvertPrivateKeyToPEMErr),
    #[error(transparent)]
    ConvertPublicKeyToPEMErr(ConvertPublicKeyToPEMErr),
    #[error(transparent)]
    ConvertPublicKeyToDERErr(ConvertPublicKeyToDERErr),
    #[error(transparent)]
    GenerateRSAKeyPairErr(GenerateRSAKeyPairErr),
    #[error(transparent)]
    DecodePEMErr(DecodePEMErr),
    #[error(transparent)]
    UnsupportedPEMLabelErr(UnsupportedPEMLabelErr),
    #[error(transparent)]
    ParsePrivateKeyErr(ParsePrivateKeyErr),
    #[error(transparent)]
    ParsePublicKeyErr(ParsePublicKeyErr),
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
    ConvertPrivateKeyToDERErr,
    ConvertPrivateKeyToPEMErr,
    ConvertPublicKeyToPEMErr,
    ConvertPublicKeyToDERErr,
    GenerateRSAKeyPairErr,
    DecodePEMErr,
    UnsupportedPEMLabelErr,
    ParsePrivateKeyErr,
    ParsePublicKeyErr,
    SignDataErr,
});
