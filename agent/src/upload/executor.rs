// internal crates
use crate::upload::{errors::UploadErr, job::Job};
use backend_api::models::{CreateUploadRequest, UploadSource};

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

/// Maps a [`Job`] into the backend's `POST /uploads` payload. Kept separate so
/// the wire mapping is unit-testable without mocks. Timestamps are RFC 3339
/// (the repo-wide `DateTime<Utc>` wire convention); a size beyond `i64` (never
/// expected in practice) saturates instead of silently wrapping. The scanner
/// has no incomplete-file signal yet, so `incomplete` stays unset and the
/// backend defaults it to false.
pub fn create_request(job: &Job) -> CreateUploadRequest {
    CreateUploadRequest {
        upload_rule_id: job.upload_rule_id.clone(),
        source: Box::new(UploadSource {
            file_path: job.file.to_string(),
            mtime: job.mtime.to_rfc3339(),
            first_observed_at: job.first_observed_at.to_rfc3339(),
            last_observed_at: job.last_observed_at.to_rfc3339(),
        }),
        digest: job.digest.clone(),
        size: i64::try_from(job.size).unwrap_or(i64::MAX),
        incomplete: None,
        release_id: job.release_id.clone(),
        deployment_id: job.deployment_id.clone(),
    }
}
