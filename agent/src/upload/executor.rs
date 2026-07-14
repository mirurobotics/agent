// internal crates
use crate::upload::{errors::UploadErr, job::Job};

// external crates
use tracing::info;

/// The seam between the upload actor and the transfer mechanics.
///
/// The production executor (a follow-up PR) fetches short-lived downscoped
/// cloud credentials from the backend (`POST /uploads`), transfers the file
/// with the native storage SDK, then confirms the upload. This trait keeps the
/// actor independent of those mechanics.
///
/// # Cancel safety
///
/// The actor drops an in-progress `upload` future on shutdown, so
/// implementations must tolerate being cancelled at any await point. An
/// interrupted transfer is re-driven after restart via scanner re-observation
/// plus backend digest dedup.
pub trait UploadExecutor: Send + Sync {
    fn upload(&self, job: &Job) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
}

/// Placeholder executor that logs the job and reports success. Stands in until
/// the real credential-fetch + native-SDK executor lands.
pub struct LogExecutor;

impl UploadExecutor for LogExecutor {
    async fn upload(&self, job: &Job) -> Result<(), UploadErr> {
        info!("LogExecutor: pretending to upload {job:?}");
        Ok(())
    }
}
