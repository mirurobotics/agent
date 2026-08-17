// internal crates
pub use crate::data_uploads::queue::QueueFullErr;
use crate::errors::Trace;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
#[error("upload executor error: {source}")]
pub struct ExecutorErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub is_terminal: bool,
    pub is_network_conn_err: bool,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ExecutorErr {
    fn is_terminal(&self) -> bool {
        self.is_terminal
    }
    fn is_network_conn_err(&self) -> bool {
        self.is_network_conn_err
    }
}

#[derive(Debug, thiserror::Error)]
#[error("upload attempt for file {file} ({size} bytes) exceeded its {deadline:?} deadline")]
pub struct AttemptTimeoutErr {
    pub file: String,
    pub size: u64,
    pub deadline: std::time::Duration,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for AttemptTimeoutErr {}

/// The transfer layer's own preconditions, as a type so tests and callers can
/// tell them apart. Carried as the `source` of an [`ExecutorErr`];
/// classification is unchanged (non-terminal, non-network) and the `Display`
/// text is what already appeared in logs.
#[derive(Debug, thiserror::Error)]
pub enum TransferErr {
    #[error("unrecognized upload credential scheme")]
    UnrecognizedScheme,
    #[error("s3 scheme is missing s3_credentials")]
    MissingS3Credentials,
    #[error("gcs scheme is missing gcs_credentials")]
    MissingGcsCredentials,
}

impl crate::errors::Error for TransferErr {}

#[derive(Debug, thiserror::Error)]
pub enum UploadErr {
    #[error(transparent)]
    QueueFullErr(#[from] QueueFullErr),
    #[error(transparent)]
    ExecutorErr(ExecutorErr),
    #[error(transparent)]
    AttemptTimeoutErr(AttemptTimeoutErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(UploadErr {
    QueueFullErr,
    ExecutorErr,
    AttemptTimeoutErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single
/// error surface the actor sees from executor and transfer failures.
pub(crate) fn executor_err<E>(source: E) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    wrap_executor_err(source, false, false)
}

/// Wraps a Miru error while preserving its terminal and network-connection
/// classifications.
pub(crate) fn classified_executor_err<E>(source: E) -> UploadErr
where
    E: crate::errors::Error + Send + Sync + 'static,
{
    let is_terminal = source.is_terminal();
    let is_network_conn_err = source.is_network_conn_err();
    wrap_executor_err(source, is_terminal, is_network_conn_err)
}

fn wrap_executor_err<E>(source: E, is_terminal: bool, is_network_conn_err: bool) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: source.into(),
        is_terminal,
        is_network_conn_err,
        trace: crate::trace!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authn::errors::{AuthnErr, MockError};
    use crate::errors::{Error as _, HTTPCode};
    use crate::gcs;
    use crate::http::errors::{HTTPErr, MockErr, RequestFailed};
    use crate::http::request::Params;
    use crate::s3;

    #[test]
    fn classified_network_source_sets_flag() {
        let err = classified_executor_err(HTTPErr::MockErr(MockErr {
            is_network_conn_err: true,
        }));
        assert!(err.is_network_conn_err());
        assert!(!err.is_terminal());
    }

    #[test]
    fn classified_non_network_source_clears_flag() {
        let err = classified_executor_err(HTTPErr::MockErr(MockErr {
            is_network_conn_err: false,
        }));
        assert!(!err.is_network_conn_err());
    }

    #[test]
    fn classified_s3_connection_err_is_network() {
        let err = classified_executor_err(s3::S3Err::ConnectionErr(s3::errors::ConnectionErr {
            object: s3::Object {
                bucket: "bucket".to_string(),
                key: "key".to_string(),
            },
            msg: "connection refused".to_string(),
            trace: crate::trace!(),
        }));
        assert!(err.is_network_conn_err());
    }

    #[test]
    fn classified_gcs_connection_err_is_network() {
        let err = classified_executor_err(gcs::GcsErr::ConnectionErr(gcs::errors::ConnectionErr {
            object: gcs::Object {
                bucket: "bucket".to_string(),
                key: "key".to_string(),
            },
            msg: "connection refused".to_string(),
            trace: crate::trace!(),
        }));
        assert!(err.is_network_conn_err());
    }

    #[test]
    fn classified_authn_mock_network_err_is_network() {
        let err = classified_executor_err(AuthnErr::MockError(MockError {
            is_network_conn_err: true,
            trace: crate::trace!(),
        }));
        assert!(err.is_network_conn_err());
    }

    #[test]
    fn executor_err_defaults_to_non_network() {
        let err = executor_err("some message");
        assert!(!err.is_network_conn_err());
        assert!(!err.is_terminal());
    }

    #[test]
    fn classified_terminal_source_is_terminal() {
        let err = classified_executor_err(HTTPErr::RequestFailed(RequestFailed {
            request: Params::get("http://test/uploads").meta().unwrap(),
            status: HTTPCode::BAD_REQUEST,
            error: None,
            trace: crate::trace!(),
        }));
        assert!(err.is_terminal());
        assert!(!err.is_network_conn_err());
    }
}
