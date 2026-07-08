// internal crates
use crate::upload::{errors::UploadErr, job::UploadJob};

// external crates
use tracing::info;

/// The seam between the upload actor and the transfer mechanics.
///
/// The eventual real executor will fetch short-lived cloud credentials from
/// the backend (`POST /uploads` returning S3 STS / GCS OAuth2 credentials),
/// transfer the file with the native storage SDK, then confirm the upload.
/// This trait keeps the actor independent of those mechanics.
///
/// # Cancel safety
///
/// The actor drops an in-progress `upload` future on shutdown, so
/// implementations must tolerate being cancelled at any await point. An
/// interrupted transfer is re-driven after restart via scanner re-observation
/// plus backend digest dedup.
pub trait UploadExecutor: Send + Sync {
    fn upload(
        &self,
        job: &UploadJob,
    ) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
}

/// Placeholder executor that logs the job and reports success. Stands in
/// until the real credential-fetch + native-SDK executor lands.
pub struct LogExecutor;

impl UploadExecutor for LogExecutor {
    async fn upload(&self, job: &UploadJob) -> Result<(), UploadErr> {
        info!("LogExecutor: pretending to upload {job:?}");
        Ok(())
    }
}
