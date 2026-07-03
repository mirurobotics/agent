// internal crates
use crate::errors::{Code, HTTPCode, Trace};

// external crates
use aws_sdk_s3::error::SdkError;

#[derive(Debug, thiserror::Error)]
#[error("object not found: {key}")]
pub struct ObjectNotFoundErr {
    pub key: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ObjectNotFoundErr {
    fn code(&self) -> Code {
        Code::ResourceNotFound
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::NOT_FOUND
    }
}

#[derive(Debug, thiserror::Error)]
#[error("connection error for object '{key}': {msg}")]
pub struct ConnectionErr {
    pub key: String,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConnectionErr {
    fn is_network_conn_err(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
pub struct RequestFailedErr {
    pub operation: String,
    pub key: Option<String>,
    pub status: Option<u16>,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for RequestFailedErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = self.key.as_deref().unwrap_or("<none>");
        let status = self
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        write!(
            f,
            "S3 {} request for object '{}' failed with status {}: {}",
            self.operation, key, status, self.msg
        )
    }
}

impl crate::errors::Error for RequestFailedErr {}

#[derive(Debug, thiserror::Error)]
#[error("invalid response from S3 {operation} request: {msg}")]
pub struct InvalidResponseErr {
    pub operation: String,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidResponseErr {}

#[derive(Debug, thiserror::Error)]
pub enum S3Err {
    #[error(transparent)]
    ObjectNotFoundErr(ObjectNotFoundErr),
    #[error(transparent)]
    ConnectionErr(ConnectionErr),
    #[error(transparent)]
    RequestFailedErr(RequestFailedErr),
    #[error(transparent)]
    InvalidResponseErr(InvalidResponseErr),
}

crate::impl_error!(S3Err {
    ObjectNotFoundErr,
    ConnectionErr,
    RequestFailedErr,
    InvalidResponseErr,
});

/// Maps the operation-agnostic `SdkError` variants into an `S3Err`.
///
/// Each S3 operation has its own service-error enum `E`, so a not-found case
/// (`NoSuchKey`, HEAD 404, ...) must be classified by the caller before
/// delegating here. This helper only covers the variants that need no
/// operation-specific knowledge: dispatch/timeout (network), response and
/// construction failures, and the service-error fallback.
pub fn map_sdk_err_common<E>(
    operation: &str,
    key: Option<String>,
    err: SdkError<E>,
) -> S3Err
where
    E: std::error::Error + 'static,
{
    match err {
        SdkError::TimeoutError(e) => S3Err::ConnectionErr(ConnectionErr {
            key: key.unwrap_or_default(),
            msg: format!("request timed out: {e:?}"),
            trace: crate::trace!(),
        }),
        SdkError::DispatchFailure(e) => S3Err::ConnectionErr(ConnectionErr {
            key: key.unwrap_or_default(),
            msg: format!("failed to dispatch request: {e:?}"),
            trace: crate::trace!(),
        }),
        SdkError::ResponseError(e) => {
            let status = e.raw().status().as_u16();
            S3Err::RequestFailedErr(RequestFailedErr {
                operation: operation.to_string(),
                key,
                status: Some(status),
                msg: "response could not be parsed".to_string(),
                trace: crate::trace!(),
            })
        }
        SdkError::ConstructionFailure(e) => S3Err::RequestFailedErr(RequestFailedErr {
            operation: operation.to_string(),
            key,
            status: None,
            msg: format!("failed to construct request: {e:?}"),
            trace: crate::trace!(),
        }),
        SdkError::ServiceError(e) => {
            let status = e.raw().status().as_u16();
            let source = e.into_err();
            S3Err::RequestFailedErr(RequestFailedErr {
                operation: operation.to_string(),
                key,
                status: Some(status),
                msg: source.to_string(),
                trace: crate::trace!(),
            })
        }
        // `SdkError` is `#[non_exhaustive]`; treat any future variant as a
        // generic request failure rather than panicking.
        other => S3Err::RequestFailedErr(RequestFailedErr {
            operation: operation.to_string(),
            key,
            status: None,
            msg: format!("request failed: {other}"),
            trace: crate::trace!(),
        }),
    }
}
