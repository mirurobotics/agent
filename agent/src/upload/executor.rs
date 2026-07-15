// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::TokenManagerExt;
use crate::http::{self, ClientI};
use crate::upload::errors::{executor_err, UploadErr};
use crate::upload::job::Job;
use crate::upload::transfer::ObjectTransfer;
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus};

/// The seam between the upload actor and the transfer mechanics.
///
/// The production executor fetches short-lived downscoped cloud credentials
/// from the backend (`POST /uploads`), transfers the file with the native
/// storage SDK, then confirms the upload. This trait keeps the actor
/// independent of those mechanics.
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

/// Production [`UploadExecutor`] driving the backend's broker upload flow:
/// mint the upload at the backend (`POST /uploads`), transfer the file's
/// bytes to cloud storage through the injected [`ObjectTransfer`], then
/// confirm (`POST /uploads/{id}/confirm`).
///
/// If the backend's digest dedup reports the file already `uploaded` in the
/// create response, the executor short-circuits without transferring or
/// confirming. Any other status (including the forward-compat catch-all
/// [`UploadStatus::UploadStatusUnknown`]) proceeds with transfer and confirm:
/// a new backend status must not strand device files, and confirm is
/// idempotent server-side.
///
/// Mid-upload credential expiry handling (re-requesting exact-key credentials
/// via `POST /uploads/{id}/credentials` and resuming the partial transfer) is
/// an explicit follow-up; until then an expired credential fails the transfer
/// and the flow is re-driven from scratch.
///
/// # Cancel safety
///
/// Every await point may be the last — the actor drops in-flight futures on
/// shutdown. No in-memory state is required for correctness: create is
/// digest-deduped and confirm is idempotent server-side, so a re-observed
/// file safely re-drives the whole flow.
pub struct BrokerExecutor<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> {
    http_client: Arc<C>,
    token_mngr: Arc<T>,
    transfer: X,
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> BrokerExecutor<C, T, X> {
    pub fn new(http_client: Arc<C>, token_mngr: Arc<T>, transfer: X) -> Self {
        Self {
            http_client,
            token_mngr,
            transfer,
        }
    }

    /// Fetches a fresh bearer token. Called immediately before each HTTP call
    /// — the transfer between create and confirm can be long, and a token
    /// fetched before create may have expired by confirm time.
    async fn token(&self) -> Result<String, UploadErr> {
        let token = self.token_mngr.get_token().await.map_err(executor_err)?;
        Ok(token.token.clone())
    }
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> UploadExecutor
    for BrokerExecutor<C, T, X>
{
    async fn upload(&self, job: &Job) -> Result<(), UploadErr> {
        let token = self.token().await?;
        let payload = create_request(job);
        let resp = http::with_retry(|| async {
            let params = http::uploads::CreateParams {
                payload: &payload,
                token: &token,
            };
            http::uploads::create(self.http_client.as_ref(), params).await
        })
        .await
        .map_err(executor_err)?;

        // Digest dedup: the backend already holds this file durably, so there
        // is nothing left to transfer or confirm.
        if resp.upload.status == UploadStatus::UPLOAD_STATUS_UPLOADED {
            return Ok(());
        }

        self.transfer
            .transfer(&resp.credentials, &resp.upload.destination, &job.file)
            .await?;

        let token = self.token().await?;
        http::with_retry(|| async {
            let params = http::uploads::ConfirmParams {
                id: &resp.upload.id,
                token: &token,
            };
            http::uploads::confirm(self.http_client.as_ref(), params).await
        })
        .await
        .map_err(executor_err)?;
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
