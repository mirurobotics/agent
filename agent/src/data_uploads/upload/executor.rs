// standard crates
use std::fmt::Display;
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::data_uploads::upload::{
    errors::{classified_executor_err, UploadErr},
    job::{Job, UploadRef},
    transfer::ObjectTransfer,
};
use crate::errors::{Error, HTTPCode};
use crate::http::{self, ClientI, HTTPErr};
use backend_api::models::{CreateUploadRequest, Upload, UploadCredentials, UploadSource};

// external crates
use chrono::{Local, TimeZone};
use tracing::info;

/// The seam between the upload actor and the transfer mechanics.
///
/// The production executor fetches short-lived downscoped cloud credentials from the
/// backend, transfers the file with the native storage SDK, then confirms the upload.
/// This trait keeps the actor independent of those mechanics.
///
/// An attempt saves the job's upload through `checkpointer` as it goes, and the
/// next attempt reads it back from `job.upload`, so a retry or a restart
/// continues the same upload.
///
/// # Cancel safety
///
/// The actor drops an in-progress `upload` future on shutdown or when the attempt
/// deadline expires, so implementations must tolerate being cancelled at any await
/// point. An interrupted transfer is re-driven from the persisted queue, which keeps
/// the entry, with its saved upload, until the upload is confirmed. Scanner
/// re-observation does NOT re-drive it — the scanner ledger records that a file was
/// reported, not that it was uploaded.
pub trait UploadExecutor: Send + Sync {
    fn upload<K: Checkpointer>(
        &self,
        job: &Job,
        checkpointer: &K,
    ) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
}

/// Saves a job's upload while an attempt runs. `save` returns once the upload is
/// on disk (or can no longer be saved), so the caller can rely on it before its
/// next step.
pub trait Checkpointer: Send + Sync {
    fn save(&self, upload: &UploadRef) -> impl std::future::Future<Output = ()> + Send;
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
        self.token_mngr
            .get_token()
            .await
            .map_err(classified_executor_err)
    }

    /// Creates the backend upload and saves it before any bytes move, so every
    /// later attempt reuses it.
    async fn create_upload<K: Checkpointer>(
        &self,
        job: &Job,
        checkpointer: &K,
    ) -> Result<(UploadRef, UploadCredentials), UploadErr> {
        let token = self.token().await?;
        let payload = new_upl_request(job, &Local);
        let resp = http::with_retry(|| async {
            let params = http::uploads::CreateParams {
                payload: &payload,
                token: &token.token,
            };
            http::uploads::create(self.http_client.as_ref(), params).await
        })
        .await
        .map_err(classified_executor_err)?;
        info!(
            "upload: created upload {} for file {} (scheme {:?})",
            resp.upload.id, job.file, resp.credentials.scheme
        );
        let upload = UploadRef {
            id: resp.upload.id,
            bucket_name: resp.upload.destination.bucket_name,
            object_key: resp.upload.destination.object_key,
            metadata: resp.metadata,
            transfer: None,
        };
        checkpointer.save(&upload).await;
        Ok((upload, *resp.credentials))
    }

    /// Fresh credentials for an existing upload, or `None` if it is gone.
    async fn vend_credentials(&self, id: &str) -> Result<Option<UploadCredentials>, UploadErr> {
        let token = self.token().await?;
        let result = http::with_retry(|| async {
            let params = http::uploads::VendCredentialsParams {
                id,
                token: &token.token,
            };
            http::uploads::vend_credentials(self.http_client.as_ref(), params).await
        })
        .await;
        unless_gone(result)
    }

    /// Whether a gone upload is gone because it was already uploaded. Confirm
    /// is idempotent: it succeeds on an uploaded upload and fails with 404 or
    /// 409 on a deleted or failed one.
    async fn already_uploaded(&self, id: &str) -> Result<bool, UploadErr> {
        let token = self.token().await?;
        Ok(unless_gone(self.confirm(&token, id).await)?.is_some())
    }

    async fn confirm_upload(&self, id: &str) -> Result<(), UploadErr> {
        let token = self.token().await?;
        self.confirm(&token, id)
            .await
            .map(|_| ())
            .map_err(classified_executor_err)
    }

    async fn confirm(&self, token: &Token, id: &str) -> Result<Upload, HTTPErr> {
        http::with_retry(|| async {
            let params = http::uploads::ConfirmParams {
                id,
                token: &token.token,
            };
            http::uploads::confirm(self.http_client.as_ref(), params).await
        })
        .await
    }
}

/// `None` if the upload is gone: the backend deleted it (404), or it is no
/// longer pending because it was uploaded or failed (409).
fn unless_gone<T>(result: Result<T, HTTPErr>) -> Result<Option<T>, UploadErr> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) if matches!(e.http_status(), HTTPCode::NOT_FOUND | HTTPCode::CONFLICT) => Ok(None),
        Err(e) => Err(classified_executor_err(e)),
    }
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> UploadExecutor for LiveExecutor<C, T, X> {
    async fn upload<K: Checkpointer>(&self, job: &Job, checkpointer: &K) -> Result<(), UploadErr> {
        // A retry reuses the job's upload with fresh credentials, unless the
        // backend no longer accepts it.
        let (upload, credentials) = match &job.upload {
            None => self.create_upload(job, checkpointer).await?,
            Some(upload) => match self.vend_credentials(&upload.id).await? {
                Some(credentials) => (upload.clone(), credentials),
                None if self.already_uploaded(&upload.id).await? => {
                    info!("upload: upload {} was already confirmed", upload.id);
                    return Ok(());
                }
                None => {
                    info!(
                        "upload: upload {} is gone; creating a new one for file {}",
                        upload.id, job.file
                    );
                    self.create_upload(job, checkpointer).await?
                }
            },
        };

        info!(
            "upload: transferring file {} to bucket {} key {}",
            job.file, upload.bucket_name, upload.object_key
        );
        self.transfer
            .transfer(&credentials, &upload, &job.file, checkpointer)
            .await?;

        info!(
            "upload: confirming upload {} for file {}",
            upload.id, job.file
        );
        self.confirm_upload(&upload.id).await
    }
}

pub fn new_upl_request<Tz>(job: &Job, tz: &Tz) -> CreateUploadRequest
where
    Tz: TimeZone,
    Tz::Offset: Display,
{
    CreateUploadRequest {
        file_rule_id: job.file_rule_id.clone(),
        source: Box::new(UploadSource {
            file_path: job.file.to_string(),
            mtime: job.mtime.with_timezone(tz).to_rfc3339(),
            first_observed_at: job.first_observed_at.to_rfc3339(),
            last_observed_at: job.last_observed_at.to_rfc3339(),
        }),
        digest: job.digest.clone(),
        size: i64::try_from(job.size).unwrap_or(i64::MAX),
        incomplete: None,
        deployment_id: job.deployment_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::Arc;

    // internal crates
    use crate::test_utils::{
        checkpointer::NoopCheckpointer,
        filesys::files as test_files,
        http_client::{Call, MockClient},
        upload::{make_job, pending_response, token_manager},
    };
    use miru_agent::data_uploads::upload::{LiveExecutor, SdkTransfer, UploadExecutor};
    use miru_agent::filesys::{files, WriteOptions};

    // external crates
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    /// The path-style PutObject URI for the upload fixture's bucket
    /// and key with the vended `us-east-1` region (see the transfer tests).
    const S3_PUT_URI: &str =
        "https://s3.us-east-1.amazonaws.com/my-bucket/logs/a.log?x-id=PutObject";

    #[tokio::test]
    async fn end_to_end_with_sdk_transfer_over_replayed_s3() {
        // Same composition as production — LiveExecutor over the real
        // SdkTransfer — with the S3 exchange replayed offline.
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            http::Request::builder()
                .method("PUT")
                .uri(S3_PUT_URI)
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .header("content-length", "0")
                .body(SdkBody::empty())
                .unwrap(),
        )]);
        let client = Arc::new(MockClient::default());
        client.set_create_upload(|| Ok(pending_response()));
        let src = test_files::temp("upload-executor-test").unwrap();
        files::write_bytes(
            src.file(),
            b"hello world",
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
        let mut job = make_job("a.log");
        job.file = src.file().clone();
        let executor = LiveExecutor::new(
            client.clone(),
            token_manager(),
            SdkTransfer::with_http_client(replay.clone()),
        );

        executor.upload(&job, &NoopCheckpointer).await.unwrap();

        // The replay client saw exactly one PUT at the expected URI — the create
        // response's credentials and destination drove the production transfer —
        // stamped with the create response's metadata map.
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), "PUT");
        assert_eq!(requests[0].uri().to_string(), S3_PUT_URI);
        assert_eq!(
            requests[0].headers().get("x-amz-meta-device_id"),
            Some("dvc_1")
        );
        assert_eq!(client.call_count(Call::ConfirmUpload), 1);
    }
}
