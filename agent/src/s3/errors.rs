// internal crates
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys;

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
    pub object: Option<String>,
    pub status: Option<u16>,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for RequestFailedErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let object = self.object.as_deref().unwrap_or("<none>");
        let status = self
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        write!(
            f,
            "S3 {} request for object '{}' failed with status {}: {}",
            self.operation, object, status, self.msg
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
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
}

impl From<filesys::FileSysErr> for S3Err {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(S3Err {
    ObjectNotFoundErr,
    ConnectionErr,
    RequestFailedErr,
    InvalidResponseErr,
    FileSysErr,
});

/// Whether an `SdkError` carries an HTTP 404 (checked against the raw response
/// status, so it works for any operation's service-error enum `E`).
///
/// This is detection only. What a 404 *means* is operation-specific and cannot
/// be decided here: some callers turn it into an error (`get` →
/// `ObjectNotFoundErr`), while others treat it as a non-error success value
/// (`exists` → `Ok(false)`). Callers must therefore check this and
/// short-circuit before delegating to [`map_sdk_err`].
pub fn is_not_found<E>(err: &SdkError<E>) -> bool {
    err.raw_response()
        .map(|r| r.status().as_u16() == 404)
        .unwrap_or(false)
}

/// Maps the operation-agnostic `SdkError` variants into an `S3Err`.
///
/// Each S3 operation has its own service-error enum `E`. A not-found case can be
/// *detected* generically with [`is_not_found`], but it must be *interpreted* and
/// short-circuited by the caller before delegating here, because some callers
/// treat a 404 as a success value (`Ok(false)`) that this `-> S3Err` signature
/// cannot express. This helper only covers the variants that need no
/// operation-specific knowledge: dispatch/timeout (network), response and
/// construction failures, and the service-error fallback.
pub fn map_sdk_err<E>(operation: &str, key: Option<String>, err: SdkError<E>) -> S3Err
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
                object: key,
                status: Some(status),
                msg: "response could not be parsed".to_string(),
                trace: crate::trace!(),
            })
        }
        SdkError::ConstructionFailure(e) => S3Err::RequestFailedErr(RequestFailedErr {
            operation: operation.to_string(),
            object: key,
            status: None,
            msg: format!("failed to construct request: {e:?}"),
            trace: crate::trace!(),
        }),
        SdkError::ServiceError(e) => {
            let status = e.raw().status().as_u16();
            let source = e.into_err();
            S3Err::RequestFailedErr(RequestFailedErr {
                operation: operation.to_string(),
                object: key,
                status: Some(status),
                msg: source.to_string(),
                trace: crate::trace!(),
            })
        }
        // `SdkError` is `#[non_exhaustive]`; treat any future variant as a
        // generic request failure rather than panicking.
        other => S3Err::RequestFailedErr(RequestFailedErr {
            operation: operation.to_string(),
            object: key,
            status: None,
            msg: format!("request failed: {other}"),
            trace: crate::trace!(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Error as _;
    use aws_sdk_s3::config::http::HttpResponse;
    use aws_sdk_s3::error::ConnectorError;
    use aws_smithy_types::body::SdkBody;

    // A stand-in service error `E` for the mapper's generic parameter. The
    // timeout/construction/dispatch branches never inspect it.
    #[derive(Debug, thiserror::Error)]
    #[error("dummy service error")]
    struct DummyErr;

    // Builds a raw smithy HTTP response carrying `status` with an empty body, as
    // the SDK attaches to `ResponseError`/`ServiceError` variants. Goes through
    // the `http` crate so we reuse the `TryFrom<http::Response>` conversion the
    // SDK itself uses.
    fn raw_response(status: u16) -> HttpResponse {
        let http = http::Response::builder()
            .status(status)
            .body(SdkBody::empty())
            .unwrap();
        HttpResponse::try_from(http).unwrap()
    }

    #[test]
    fn timeout_maps_to_connection_err() {
        let err: SdkError<DummyErr> =
            SdkError::timeout_error(Box::<dyn std::error::Error + Send + Sync>::from("slow"));
        let mapped = map_sdk_err("get_object", Some("k".to_string()), err);
        assert!(matches!(mapped, S3Err::ConnectionErr(_)));
        assert!(mapped.is_network_conn_err());
    }

    #[test]
    fn dispatch_failure_maps_to_connection_err() {
        let err: SdkError<DummyErr> =
            SdkError::dispatch_failure(ConnectorError::io(Box::<
                dyn std::error::Error + Send + Sync,
            >::from("socket hangup")));
        let mapped = map_sdk_err("get_object", Some("k".to_string()), err);
        assert!(matches!(mapped, S3Err::ConnectionErr(_)));
        assert!(mapped.is_network_conn_err());
    }

    #[test]
    fn response_error_maps_to_request_failed_with_status() {
        let err: SdkError<DummyErr> = SdkError::response_error(
            Box::<dyn std::error::Error + Send + Sync>::from("unparseable"),
            raw_response(500),
        );
        let mapped = map_sdk_err("get_object", Some("k".to_string()), err);
        match mapped {
            S3Err::RequestFailedErr(e) => assert_eq!(e.status, Some(500)),
            other => panic!("expected RequestFailedErr, got {other:?}"),
        }
    }

    #[test]
    fn service_error_maps_to_request_failed_with_status() {
        let err: SdkError<DummyErr> = SdkError::service_error(DummyErr, raw_response(403));
        let mapped = map_sdk_err("head_object", Some("k".to_string()), err);
        match mapped {
            S3Err::RequestFailedErr(e) => assert_eq!(e.status, Some(403)),
            other => panic!("expected RequestFailedErr, got {other:?}"),
        }
    }

    #[test]
    fn construction_failure_maps_to_request_failed_without_status() {
        let err: SdkError<DummyErr> =
            SdkError::construction_failure(Box::<dyn std::error::Error + Send + Sync>::from("bad"));
        let mapped = map_sdk_err("put_object", None, err);
        match mapped {
            S3Err::RequestFailedErr(e) => assert!(e.status.is_none()),
            other => panic!("expected RequestFailedErr, got {other:?}"),
        }
    }

    // note: the `other` non-exhaustive fallback arm is genuinely uncoverable in a
    // test: `SdkError` is `#[non_exhaustive]`, so a hypothetical future variant
    // cannot be instantiated from outside the smithy crate. There is no public
    // constructor for it, so the arm exists only to keep the match exhaustive
    // against future SDK versions.

    #[test]
    fn is_not_found_detects_404_and_ignores_no_raw_response() {
        // 404 raw response -> true.
        let not_found: SdkError<DummyErr> = SdkError::response_error(
            Box::<dyn std::error::Error + Send + Sync>::from("nf"),
            raw_response(404),
        );
        assert!(is_not_found(&not_found));

        // Timeout carries no raw response -> false.
        let timeout: SdkError<DummyErr> =
            SdkError::timeout_error(Box::<dyn std::error::Error + Send + Sync>::from("slow"));
        assert!(!is_not_found(&timeout));
    }
}
