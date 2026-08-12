// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::data_uploads::upload::{
    errors::{classified_executor_err, executor_err, UploadErr},
    job::Job,
    transfer::ObjectTransfer,
};
use crate::filesys::files;
use crate::http::{self, ClientI};
use crate::models::FileRuleRetention;
use backend_api::models::{CreateUploadRequest, UploadSource, UploadWithCredentials};

// external crates
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

pub struct LiveExecutor<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> {
    http_client: Arc<C>,
    token_mngr: Arc<T>,
    transfer: X,
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> LiveExecutor<C, T, X> {
    pub fn new(http_client: Arc<C>, token_mngr: Arc<T>, transfer: X) -> Self {
        Self {
            http_client,
            token_mngr,
            transfer,
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

    async fn delete_source_file(&self, job: &Job) {
        if job.retention
            == Some(FileRuleRetention {
                require_upload: true,
                ttl_secs: 0,
            })
        {
            info!(
                "upload: deleting local source file {} per retention policy",
                job.file
            );
            if let Err(e) = files::delete(&job.file).await {
                warn!(
                    "upload for {} confirmed but deleting the local source file failed: {e:?}",
                    job.file
                );
            }
        }
    }
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> UploadExecutor for LiveExecutor<C, T, X> {
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

        self.delete_source_file(job).await;
        Ok(())
    }
}

pub fn new_upl_request(job: &Job) -> CreateUploadRequest {
    CreateUploadRequest {
        upload_rule_id: job.file_rule_id.clone(),
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
