// internal crates
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys::{self, file::File};
use crate::gcs::Object;

// external crates
use google_cloud_gax::error::Error as GaxError;

#[derive(Debug, thiserror::Error)]
#[error("object not found: {object}")]
pub struct ObjectNotFoundErr {
    pub object: Object,
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
#[error("connection error for object '{object}': {msg}")]
pub struct ConnectionErr {
    pub object: Object,
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
    pub object: Object,
    pub status: Option<u16>,
    pub msg: String,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for RequestFailedErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = self
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        write!(
            f,
            "GCS {} request for object '{}' failed with status {}: {}",
            self.operation, self.object, status, self.msg
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
    pub object: Object,
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
/// delegating to [`map_gcs_err`].
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
/// Takes the concrete `google_cloud_gax::error::Error`, the single crate error
/// type. A failure reading the download body off the wire surfaces as the same
/// error type and is mapped by this function too.
pub fn map_gcs_err(operation: &str, object: &Object, err: GaxError) -> GcsErr {
    if err.is_timeout() {
        return GcsErr::ConnectionErr(ConnectionErr {
            object: object.clone(),
            msg: format!("request timed out: {err}"),
            trace: crate::trace!(),
        });
    }
    let status = err.http_status_code();
    GcsErr::RequestFailedErr(RequestFailedErr {
        operation: operation.to_string(),
        object: object.clone(),
        status,
        msg: err.to_string(),
        trace: crate::trace!(),
    })
}

/// Maps a local filesystem I/O error hit while streaming an object body
/// (opening the source, creating the destination, or copying) into
/// [`GcsErr::LocalIoErr`].
pub fn map_body_io_err(operation: &str, obj: &Object, file: &File, err: std::io::Error) -> GcsErr {
    GcsErr::LocalIoErr(LocalIoErr {
        operation: operation.to_string(),
        object: obj.clone(),
        msg: format!("filesystem I/O error at path '{file}': {err}"),
        trace: crate::trace!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Error as _;
    use google_cloud_gax::error::rpc::{Code, Status};

    fn object() -> Object {
        Object {
            bucket: "bucket".to_string(),
            key: "key".to_string(),
        }
    }

    #[test]
    fn service_error_maps_to_request_failed() {
        let err = GaxError::service(Status::default().set_code(Code::PermissionDenied));
        let mapped = map_gcs_err("get_object", &object(), err);
        match mapped {
            GcsErr::RequestFailedErr(e) => {
                assert_eq!(e.operation, "get_object");
                assert_eq!(e.object, object());
            }
            other => panic!("expected RequestFailedErr, got {other:?}"),
        }
    }

    #[test]
    fn timeout_maps_to_connection_err() {
        let err = GaxError::timeout("deadline exceeded");
        let mapped = map_gcs_err("get_object", &object(), err);
        assert!(mapped.is_network_conn_err());
        match mapped {
            GcsErr::ConnectionErr(e) => assert_eq!(e.object, object()),
            other => panic!("expected ConnectionErr, got {other:?}"),
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
    fn http_404_is_recognized_as_not_found() {
        let err = GaxError::http(404, http::HeaderMap::new(), Vec::<u8>::new().into());
        assert!(is_not_found(&err));
        // A non-404 HTTP error is not misclassified.
        let other = GaxError::http(403, http::HeaderMap::new(), Vec::<u8>::new().into());
        assert!(!is_not_found(&other));
    }

    #[test]
    fn request_failed_err_display_includes_object_and_status() {
        // The status is `Some`, so Display echoes the gs:// URI and the
        // numeric status.
        let err = GcsErr::RequestFailedErr(RequestFailedErr {
            operation: "put_object".to_string(),
            object: object(),
            status: Some(403),
            msg: "denied".to_string(),
            trace: crate::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("gs://bucket/key"));
        assert!(msg.contains("403"));
    }

    // A `FileSysErr` from a real (failing) filesystem op converts via `From`,
    // and the resulting variant delegates its trait behavior to the underlying
    // error.
    #[tokio::test]
    async fn filesys_err_delegates_to_underlying_error() {
        let fs_err: filesys::FileSysErr =
            filesys::files::read_bytes(&File::new("/nonexistent/definitely/not/here.bin"))
                .await
                .unwrap_err();
        // `Code` is not `PartialEq`, so compare its `Debug` form.
        let want_code = format!("{:?}", fs_err.code());
        let want_status = fs_err.http_status().as_u16();
        let want_display = fs_err.to_string();

        let err: GcsErr = fs_err.into();

        assert!(matches!(err, GcsErr::FileSysErr(_)));
        assert_eq!(format!("{:?}", err.code()), want_code);
        assert_eq!(err.http_status().as_u16(), want_status);
        assert!(!err.is_network_conn_err());
        assert_eq!(err.to_string(), want_display);
    }

    #[test]
    fn service_error_is_not_a_network_conn_err() {
        let err = GaxError::service(Status::default().set_code(Code::Aborted));
        let mapped = map_gcs_err("delete_object", &object(), err);
        assert!(!mapped.is_network_conn_err());
    }

    /// The local-I/O body mapper always yields a terminal `LocalIoErr`, never a
    /// network condition.
    mod body_mappers {
        use super::*;

        #[test]
        fn map_body_io_err_maps_to_local_io_err() {
            // A failure writing bytes to the local destination is a terminal
            // local I/O error, never a network condition.
            let err = std::io::Error::other("no space left on device");
            let mapped = map_body_io_err("get_object", &object(), &File::new("/data/out.bin"), err);
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
