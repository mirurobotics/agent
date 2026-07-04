// internal crates
use crate::errors::{Code, HTTPCode, Trace};

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
            "GCS {} request for object '{}' failed with status {}: {}",
            self.operation, key, status, self.msg
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
pub enum GcsErr {
    #[error(transparent)]
    ObjectNotFoundErr(ObjectNotFoundErr),
    #[error(transparent)]
    ConnectionErr(ConnectionErr),
    #[error(transparent)]
    RequestFailedErr(RequestFailedErr),
    #[error(transparent)]
    InvalidResponseErr(InvalidResponseErr),
}

crate::impl_error!(GcsErr {
    ObjectNotFoundErr,
    ConnectionErr,
    RequestFailedErr,
    InvalidResponseErr,
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
        key: key.map(str::to_string),
        status,
        msg: err.to_string(),
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
                assert_eq!(e.key.as_deref(), Some("k"));
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
}
