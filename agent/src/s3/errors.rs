// internal crates
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys::{self, file::File};
use crate::s3::Object;

// external crates
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStreamError;

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
#[error("local I/O error during {operation} for object '{object}': {msg}")]
pub struct LocalIoErr {
    pub operation: String,
    pub object: String,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for LocalIoErr {}

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
    LocalIoErr(LocalIoErr),
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
    LocalIoErr,
    FileSysErr,
});

/// Whether an `SdkError` carries an HTTP 404 (checked against the raw response
/// status, so it works for any operation's service-error enum `E`).
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

/// Maps a local filesystem I/O error hit while streaming an object body
/// (opening the source, creating the destination, or copying) into
/// [`S3Err::LocalIoErr`].
pub fn map_body_io_err(operation: &str, obj: &Object, file: &File, err: std::io::Error) -> S3Err {
    S3Err::LocalIoErr(LocalIoErr {
        operation: operation.to_string(),
        object: obj.to_string(),
        msg: format!("filesystem I/O error at path '{file}': {err}"),
        trace: crate::trace!(),
    })
}

/// Maps a local `ByteStream` construction error (e.g. the file could not be
/// opened or read) into [`S3Err::LocalIoErr`].
pub fn map_bytestream_err(
    operation: &str,
    obj: &Object,
    file: &File,
    err: &ByteStreamError,
) -> S3Err {
    S3Err::LocalIoErr(LocalIoErr {
        operation: operation.to_string(),
        object: obj.to_string(),
        msg: format!("failed to open '{file}' for streaming: {err}"),
        trace: crate::trace!(),
    })
}

/// Maps an error reading the object body off the wire (connection reset, TLS
/// drop, truncated body) into a retryable [`S3Err::ConnectionErr`].
pub fn map_body_read_err(operation: &str, key: &str, err: &ByteStreamError) -> S3Err {
    S3Err::ConnectionErr(ConnectionErr {
        key: key.to_string(),
        msg: format!("failed to read object body during {operation}: {err}"),
        trace: crate::trace!(),
    })
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

    mod body_mappers {
        use super::*;
        use crate::filesys::file::File;

        fn obj() -> Object {
            Object {
                bucket: "bucket".to_string(),
                key: "key".to_string(),
            }
        }

        #[test]
        fn map_body_io_err_maps_to_local_io_err() {
            // A failure writing bytes to the local destination is a terminal
            // local I/O error, never a network condition.
            let err = std::io::Error::other("no space left on device");
            let mapped = map_body_io_err("get_object", &obj(), &File::new("/data/out.bin"), err);
            assert!(matches!(mapped, S3Err::LocalIoErr(_)));
            assert!(!mapped.is_network_conn_err());
            assert_eq!(mapped.http_status().as_u16(), 500);
            let msg = mapped.to_string();
            assert!(msg.contains("s3://bucket/key"));
            assert!(msg.contains("/data/out.bin"));
            assert!(msg.contains("no space left on device"));
        }

        #[test]
        fn map_bytestream_err_maps_to_local_io_err() {
            // A failure opening the local source file for streaming is also a
            // terminal local I/O error.
            let err = ByteStreamError::from(std::io::Error::other("permission denied"));
            let mapped = map_bytestream_err("put_object", &obj(), &File::new("/data/in.bin"), &err);
            assert!(matches!(mapped, S3Err::LocalIoErr(_)));
            assert!(!mapped.is_network_conn_err());
            let msg = mapped.to_string();
            assert!(msg.contains("s3://bucket/key"));
            assert!(msg.contains("/data/in.bin"));
        }

        #[test]
        fn map_body_read_err_maps_to_connection_err() {
            // A failure reading the object body off the wire is a retryable
            // transport error. `ByteStreamError` has no public constructor, but
            // its public `From<std::io::Error>` impl builds a representative one.
            let err = ByteStreamError::from(std::io::Error::other("connection reset"));
            let mapped = map_body_read_err("get_object", "blobs/data.bin", &err);
            assert!(matches!(mapped, S3Err::ConnectionErr(_)));
            assert!(mapped.is_network_conn_err());
            assert!(mapped.to_string().contains("connection error"));
        }
    }

    /// Direct assertions on the leaf error types' trait behavior. These do not
    /// go through the SDK; they pin the `crate::errors::Error` contract each
    /// variant promises (code / http_status / is_network_conn_err / Display).
    mod error_types {
        use super::*;
        use crate::errors::Code;
        use crate::filesys::file::File;
        use crate::filesys::files;

        #[test]
        fn object_not_found_maps_to_resource_not_found() {
            let err = S3Err::ObjectNotFoundErr(ObjectNotFoundErr {
                key: "k".to_string(),
                trace: crate::trace!(),
            });
            assert!(matches!(err.code(), Code::ResourceNotFound));
            assert_eq!(err.http_status().as_u16(), 404);
            assert!(!err.is_network_conn_err());
            assert!(err.to_string().contains("object not found"));
        }

        #[test]
        fn connection_err_is_network_conn_err() {
            let err = S3Err::ConnectionErr(ConnectionErr {
                key: "k".to_string(),
                msg: "boom".to_string(),
                trace: crate::trace!(),
            });
            assert!(err.is_network_conn_err());
            assert!(matches!(err.code(), Code::InternalServerError));
            assert!(err.to_string().contains("connection error"));
        }

        #[test]
        fn request_failed_err_defaults_to_internal_server_error() {
            let err = S3Err::RequestFailedErr(RequestFailedErr {
                operation: "get_object".to_string(),
                object: None,
                status: None,
                msg: "nope".to_string(),
                trace: crate::trace!(),
            });
            assert!(matches!(err.code(), Code::InternalServerError));
            assert_eq!(err.http_status().as_u16(), 500);
            assert!(!err.is_network_conn_err());
            // Display with no object / no status hits the fallback formatting.
            let msg = err.to_string();
            assert!(msg.contains("<none>"));
            assert!(msg.contains("unknown"));
        }

        #[test]
        fn invalid_response_err_defaults_to_internal_server_error() {
            let err = S3Err::InvalidResponseErr(InvalidResponseErr {
                operation: "get_object".to_string(),
                msg: "bad body".to_string(),
                trace: crate::trace!(),
            });
            assert!(matches!(err.code(), Code::InternalServerError));
            assert_eq!(err.http_status().as_u16(), 500);
            assert!(err.to_string().contains("invalid response"));
        }

        #[test]
        fn local_io_err_defaults_to_internal_server_error() {
            let err = S3Err::LocalIoErr(LocalIoErr {
                operation: "get_object".to_string(),
                object: "s3://bucket/key".to_string(),
                msg: "no such file or directory".to_string(),
                trace: crate::trace!(),
            });
            assert!(matches!(err.code(), Code::InternalServerError));
            assert_eq!(err.http_status().as_u16(), 500);
            assert!(!err.is_network_conn_err());
            let msg = err.to_string();
            assert!(msg.contains("s3://bucket/key"));
            assert!(msg.contains("no such file or directory"));
        }

        #[test]
        fn request_failed_err_display_includes_object_and_status() {
            // The non-`<none>`/`unknown` branch: both object and status are
            // `Some`, so Display echoes the s3:// URI and the numeric status.
            let err = S3Err::RequestFailedErr(RequestFailedErr {
                operation: "put_object".to_string(),
                object: Some("s3://bucket/key".to_string()),
                status: Some(403),
                msg: "denied".to_string(),
                trace: crate::trace!(),
            });
            let msg = err.to_string();
            assert!(msg.contains("s3://bucket/key"));
            assert!(msg.contains("403"));
        }

        // A `FileSysErr` produced by a real (failing) filesystem op converts
        // into an `S3Err` via `From<filesys::FileSysErr>`, and the resulting
        // variant delegates its trait behavior (code / http_status / Display)
        // to the underlying error.
        #[tokio::test]
        async fn filesys_err_delegates_to_underlying_error() {
            let fs_err: filesys::FileSysErr =
                files::read_bytes(&File::new("/nonexistent/definitely/not/here.bin"))
                    .await
                    .unwrap_err();
            // Capture the underlying values to assert delegation. `Code` is not
            // `PartialEq`, so compare its `Debug` form.
            let want_code = format!("{:?}", fs_err.code());
            let want_status = fs_err.http_status().as_u16();
            let want_display = fs_err.to_string();

            // Explicitly exercise `From<filesys::FileSysErr> for S3Err`.
            let err: S3Err = fs_err.into();

            assert!(matches!(err, S3Err::FileSysErr(_)));
            assert_eq!(format!("{:?}", err.code()), want_code);
            assert_eq!(err.http_status().as_u16(), want_status);
            assert!(!err.is_network_conn_err());
            assert_eq!(err.to_string(), want_display);
        }
    }
}
