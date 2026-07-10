// internal crates
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys::{self, file::File};
use crate::gcs::Object;

// external crates
use google_cloud_gax::error::Error as GaxError;

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
    // s3 names this field `object`; matched here for parity. Carries the
    // `gs://bucket/key` URI (or the bare key) of the target object, when known.
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
            "GCS {} request for object '{}' failed with status {}: {}",
            self.operation, object, status, self.msg
        )
    }
}

impl crate::errors::Error for RequestFailedErr {}

#[derive(Debug, thiserror::Error)]
#[error("invalid response from GCS {operation} request: {msg}")]
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
pub enum GcsErr {
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
    // s3's `S3Err::NoSuchUploadErr` has no GCS analog: GCS's `write_object` folds
    // the multipart/resumable decision inside the SDK, so there is no user-visible
    // upload-id surface to report a missing upload against (see `gcs/mod.rs`).
}

impl From<filesys::FileSysErr> for GcsErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(GcsErr {
    ObjectNotFoundErr,
    ConnectionErr,
    RequestFailedErr,
    InvalidResponseErr,
    LocalIoErr,
    FileSysErr,
});

/// Returns whether a GCS SDK error represents a missing object (gRPC
/// `NOT_FOUND` or HTTP 404). The four ops classify not-found themselves before
/// delegating to [`map_gcs_err`], mirroring how s3 classifies `NoSuchKey`/404.
pub fn is_not_found(err: &GaxError) -> bool {
    err.status()
        .map(|s| s.code == google_cloud_gax::error::rpc::Code::NotFound)
        .unwrap_or(false)
        || err.http_status_code() == Some(404)
}

/// Maps a GCS SDK error into a `GcsErr`. Timeouts become [`ConnectionErr`]
/// (network); everything else becomes a [`RequestFailedErr`] carrying the HTTP
/// status (when present) and the error's own message. Not-found is classified
/// by the caller before reaching here.
///
/// This is the analog of s3's `map_sdk_err`, but it takes the concrete
/// `google_cloud_gax::error::Error` (GCS has a single crate error type, not a
/// generic `SdkError<E>`). It also absorbs the body-read-error role that s3
/// splits into a separate `map_body_read_err`: GCS has no `ByteStream`, so a
/// failure reading the download body off the wire surfaces here as the same
/// `google_cloud_gax::error::Error` and is mapped by this function.
pub fn map_gcs_err(operation: &str, key: Option<&str>, err: GaxError) -> GcsErr {
    if err.is_timeout() {
        return GcsErr::ConnectionErr(ConnectionErr {
            key: key.unwrap_or_default().to_string(),
            msg: format!("request timed out: {err}"),
            trace: crate::trace!(),
        });
    }
    let status = err.http_status_code();
    GcsErr::RequestFailedErr(RequestFailedErr {
        operation: operation.to_string(),
        object: key.map(str::to_string),
        status,
        msg: err.to_string(),
        trace: crate::trace!(),
    })
}

/// Maps a local filesystem I/O error hit while streaming an object body
/// (opening the source, creating the destination, or copying) into
/// [`GcsErr::LocalIoErr`]. Mirrors s3's `map_body_io_err`.
pub fn map_body_io_err(operation: &str, obj: &Object, file: &File, err: std::io::Error) -> GcsErr {
    GcsErr::LocalIoErr(LocalIoErr {
        operation: operation.to_string(),
        object: obj.to_string(),
        msg: format!("filesystem I/O error at path '{file}': {err}"),
        trace: crate::trace!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Error as _;
    use google_cloud_gax::error::rpc::{Code, Status};

    #[test]
    fn service_error_maps_to_request_failed() {
        let err = GaxError::service(Status::default().set_code(Code::PermissionDenied));
        let mapped = map_gcs_err("get_object", Some("k"), err);
        match mapped {
            GcsErr::RequestFailedErr(e) => {
                assert_eq!(e.operation, "get_object");
                assert_eq!(e.object.as_deref(), Some("k"));
            }
            other => panic!("expected RequestFailedErr, got {other:?}"),
        }
    }

    #[test]
    fn not_found_status_is_recognized() {
        let err = GaxError::service(Status::default().set_code(Code::NotFound));
        assert!(is_not_found(&err));
        // A non-not-found service error is not misclassified.
        let other = GaxError::service(Status::default().set_code(Code::PermissionDenied));
        assert!(!is_not_found(&other));
    }

    #[test]
    fn service_error_is_not_a_network_conn_err() {
        let err = GaxError::service(Status::default().set_code(Code::Aborted));
        let mapped = map_gcs_err("delete_object", None, err);
        assert!(!mapped.is_network_conn_err());
    }

    /// Mirrors s3's `body_mappers` submodule: the local-I/O body mapper always
    /// yields a terminal `LocalIoErr`, never a network condition.
    mod body_mappers {
        use super::*;
        use crate::gcs::Object;

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
            assert!(matches!(mapped, GcsErr::LocalIoErr(_)));
            assert!(!mapped.is_network_conn_err());
            assert_eq!(mapped.http_status().as_u16(), 500);
            let msg = mapped.to_string();
            assert!(msg.contains("gs://bucket/key"));
            assert!(msg.contains("/data/out.bin"));
            assert!(msg.contains("no space left on device"));
        }
    }
}
