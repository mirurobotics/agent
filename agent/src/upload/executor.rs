// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::delete::{DeleterExt, PendingDelete};
use crate::http::{self, ClientI};
use crate::models::DeletePolicy;
use crate::upload::{
    errors::{classified_executor_err, executor_err, UploadErr},
    job::Job,
    transfer::ObjectTransfer,
};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadWithCredentials};

// external crates
use chrono::Utc;
use tracing::{info, warn};

/// The seam between the upload actor and the transfer mechanics.
///
/// The production executor fetches short-lived downscoped cloud credentials from the
/// backend, transfers the file with the native storage SDK, then confirms the upload.
/// This trait keeps the actor independent of those mechanics.
///
/// # Cancel safety
///
/// The actor drops an in-progress `upload` future on shutdown or when the attempt
/// deadline expires, so implementations must tolerate being cancelled at any await
/// point. An interrupted transfer is re-driven after restart via scanner
/// re-observation plus backend digest dedup.
pub trait UploadExecutor: Send + Sync {
    fn upload(&self, job: &Job) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
}

pub struct LiveExecutor<C: ClientI, T: TokenManagerExt, X: ObjectTransfer, D: DeleterExt> {
    http_client: Arc<C>,
    token_mngr: Arc<T>,
    transfer: X,
    /// `None` degrades to uploads-without-deletion (a deleter spawn failure
    /// must never take uploads down with it).
    deleter: Option<Arc<D>>,
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer, D: DeleterExt> LiveExecutor<C, T, X, D> {
    pub fn new(
        http_client: Arc<C>,
        token_mngr: Arc<T>,
        transfer: X,
        deleter: Option<Arc<D>>,
    ) -> Self {
        Self {
            http_client,
            token_mngr,
            transfer,
            deleter,
        }
    }

    async fn token(&self) -> Result<Arc<Token>, UploadErr> {
        self.token_mngr.get_token().await.map_err(executor_err)
    }

    async fn create_upload(&self, job: &Job) -> Result<UploadWithCredentials, UploadErr> {
        let token = self.token().await?;
        let payload = new_upl_request(job);
        http::with_retry(|| async {
            let params = http::uploads::CreateParams {
                payload: &payload,
                token: &token.token,
            };
            http::uploads::create(self.http_client.as_ref(), params).await
        })
        .await
        .map_err(classified_executor_err)
    }

    async fn confirm_upload(&self, id: &str) -> Result<(), UploadErr> {
        let token = self.token().await?;
        http::with_retry(|| async {
            let params = http::uploads::ConfirmParams {
                id,
                token: &token.token,
            };
            http::uploads::confirm(self.http_client.as_ref(), params).await
        })
        .await
        .map(|_| ())
        .map_err(classified_executor_err)
    }

    /// Hand the confirmed upload's source file to the delete worker as a
    /// [`PendingDelete`] record — the executor never deletes inline. Cancel
    /// safe: if the future is dropped before the enqueue, the file stays on
    /// disk; scanner re-observation plus backend digest dedup re-drive the
    /// upload exactly as today, and its confirmation re-enqueues the deletion.
    async fn enqueue_pending_delete(&self, job: &Job) {
        if job.delete_policy != DeletePolicy::AfterUpload {
            return;
        }
        let Some(deleter) = &self.deleter else {
            warn!(
                "upload: no deleter available; skipping deletion for {}",
                job.file
            );
            return;
        };
        let record = PendingDelete {
            file: job.file.clone(),
            size: job.size,
            mtime: job.mtime,
            digest: job.digest.clone(),
            eligible_at: Utc::now(),
            delete_delay_secs: job.delete_delay_secs,
            upload_rule_id: job.upload_rule_id.clone(),
            deployment_id: job.deployment_id.clone(),
        };
        // best-effort: the upload is already confirmed durable; a failed
        // enqueue must never fail the job (that would re-drive it).
        if let Err(e) = deleter.enqueue(record).await {
            warn!(
                "upload for {} confirmed but enqueueing its deletion failed: {e:?}",
                job.file
            );
        }
    }
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer, D: DeleterExt> UploadExecutor
    for LiveExecutor<C, T, X, D>
{
    async fn upload(&self, job: &Job) -> Result<(), UploadErr> {
        let resp = self.create_upload(job).await?;
        info!(
            "upload: created upload {} for file {} (scheme {:?})",
            resp.upload.id, job.file, resp.credentials.scheme
        );

        info!(
            "upload: transferring file {} to bucket {} key {}",
            job.file, resp.upload.destination.bucket_name, resp.upload.destination.object_key
        );
        self.transfer
            .transfer(
                &resp.credentials,
                &resp.upload.destination,
                &job.file,
                &resp.metadata,
            )
            .await?;

        info!(
            "upload: confirming upload {} for file {}",
            resp.upload.id, job.file
        );
        self.confirm_upload(&resp.upload.id).await?;

        self.enqueue_pending_delete(job).await;
        Ok(())
    }
}

pub fn new_upl_request(job: &Job) -> CreateUploadRequest {
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
        deployment_id: job.deployment_id.clone(),
    }
}
