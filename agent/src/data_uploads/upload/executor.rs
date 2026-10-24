// standard crates
use std::fmt::Display;
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::data_uploads::upload::{
    checkpoint::Checkpointer,
    errors::{classified_executor_err, UploadErr},
    job::{Checkpoint, Job, TransferState, UploadRef},
    transfer::ObjectTransfer,
};
use crate::errors::{Error, HTTPCode};
use crate::http::{self, ClientI, HTTPErr};
use backend_api::models::{
    CreateUploadRequest, UploadCredentials, UploadDestination, UploadSource, UploadWithCredentials,
};

// external crates
use chrono::{Local, TimeZone};
use tracing::info;

/// The seam between the upload actor and the transfer mechanics.
///
/// The production executor gets short-lived downscoped cloud credentials from
/// the backend, transfers the file with the storage SDK, then confirms the
/// upload. This trait keeps the actor independent of those mechanics.
///
/// The executor reports state a later attempt needs (the backend upload id,
/// a started multipart upload) through `checkpointer`, and reads it back from
/// `job` on the next attempt, so a retry or a restart continues the same
/// upload instead of starting a new one.
///
/// # Cancel safety
///
/// The actor drops an in-progress `upload` future on shutdown or when the attempt
/// deadline expires, so implementations must tolerate being cancelled at any await
/// point. An interrupted transfer is re-driven from the persisted queue, which keeps
/// the entry and its checkpoints until the upload is confirmed. Scanner
/// re-observation does NOT re-drive it — the scanner ledger records that a file
/// was reported, not that it was uploaded.
pub trait UploadExecutor: Send + Sync {
    fn upload<K: Checkpointer>(
        &self,
        job: &Job,
        checkpointer: &K,
    ) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
}

/// What an attempt still has to do, given the job's persisted upload state.
enum Plan {
    /// Transfer the file, then confirm.
    Transfer {
        upload: UploadRef,
        credentials: UploadCredentials,
        state: Option<TransferState>,
    },
    /// The object already landed; only the confirm is left.
    Confirm(UploadRef),
    /// The backend already has the upload as confirmed.
    Done,
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

    async fn create_upload(&self, job: &Job) -> Result<UploadWithCredentials, UploadErr> {
        let token = self.token().await?;
        let payload = new_upl_request(job, &Local);
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

    async fn vend_credentials(&self, id: &str) -> Result<UploadCredentials, BackendErr> {
        let token = self.token().await.map_err(BackendErr::Other)?;
        http::with_retry(|| async {
            let params = http::uploads::VendCredentialsParams {
                id,
                token: &token.token,
            };
            http::uploads::vend_credentials(self.http_client.as_ref(), params).await
        })
        .await
        .map_err(BackendErr::Http)
    }

    async fn confirm_upload(&self, id: &str) -> Result<(), UploadErr> {
        self.try_confirm(id).await.map_err(|e| match e {
            BackendErr::Http(e) => classified_executor_err(e),
            BackendErr::Other(e) => e,
        })
    }

    async fn try_confirm(&self, id: &str) -> Result<(), BackendErr> {
        let token = self.token().await.map_err(BackendErr::Other)?;
        http::with_retry(|| async {
            let params = http::uploads::ConfirmParams {
                id,
                token: &token.token,
            };
            http::uploads::confirm(self.http_client.as_ref(), params).await
        })
        .await
        .map(|_| ())
        .map_err(BackendErr::Http)
    }

    /// Creates the backend upload and persists it before any bytes move, so
    /// every later attempt reuses it.
    async fn create<K: Checkpointer>(
        &self,
        job: &Job,
        checkpointer: &K,
    ) -> Result<Plan, UploadErr> {
        let resp = self.create_upload(job).await?;
        let upload = upload_ref(&resp);
        info!(
            "upload: created upload {} for file {} (scheme {:?})",
            upload.id, job.file, resp.credentials.scheme
        );
        checkpointer.save(Checkpoint::Created(upload.clone())).await;
        Ok(Plan::Transfer {
            upload,
            credentials: *resp.credentials,
            state: None,
        })
    }

    /// Works out what this attempt has to do. A job with a persisted upload
    /// gets fresh credentials for it instead of a new upload. If the backend no
    /// longer accepts that upload (404, or 409 because it is no longer
    /// pending), a confirm tells "already uploaded" apart from "failed"; a
    /// failed or deleted upload is replaced with a new one.
    async fn plan<K: Checkpointer>(&self, job: &Job, checkpointer: &K) -> Result<Plan, UploadErr> {
        let Some(upload) = job.upload.clone() else {
            return self.create(job, checkpointer).await;
        };
        if upload.transferred {
            return Ok(Plan::Confirm(upload));
        }
        let err = match self.vend_credentials(&upload.id).await {
            Ok(credentials) => {
                info!("upload: reusing upload {} for file {}", upload.id, job.file);
                return Ok(Plan::Transfer {
                    upload,
                    credentials,
                    state: job.transfer.clone(),
                });
            }
            Err(BackendErr::Http(e)) if is_gone(&e) => e,
            Err(BackendErr::Http(e)) => return Err(classified_executor_err(e)),
            Err(BackendErr::Other(e)) => return Err(e),
        };
        if err.http_status() == HTTPCode::CONFLICT {
            match self.try_confirm(&upload.id).await {
                Ok(()) => {
                    info!("upload: upload {} was already confirmed", upload.id);
                    return Ok(Plan::Done);
                }
                Err(BackendErr::Http(e)) if is_gone(&e) => {}
                Err(BackendErr::Http(e)) => return Err(classified_executor_err(e)),
                Err(BackendErr::Other(e)) => return Err(e),
            }
        }
        info!(
            "upload: backend no longer accepts upload {} ({err}); creating a new one for file {}",
            upload.id, job.file
        );
        checkpointer.save(Checkpoint::UploadCleared).await;
        self.create(job, checkpointer).await
    }
}

/// A backend call failure, kept unwrapped so `plan` can inspect its status.
enum BackendErr {
    Http(HTTPErr),
    Other(UploadErr),
}

/// The backend no longer has the upload as pending: deleted (404), or
/// uploaded or failed (409).
fn is_gone(err: &HTTPErr) -> bool {
    matches!(err.http_status(), HTTPCode::NOT_FOUND | HTTPCode::CONFLICT)
}

fn upload_ref(resp: &UploadWithCredentials) -> UploadRef {
    let destination = &resp.upload.destination;
    UploadRef {
        id: resp.upload.id.clone(),
        bucket_id: destination.bucket_id.clone(),
        bucket_name: destination.bucket_name.clone(),
        object_key: destination.object_key.clone(),
        metadata: resp.metadata.clone(),
        transferred: false,
    }
}

fn destination(upload: &UploadRef) -> UploadDestination {
    UploadDestination {
        bucket_id: upload.bucket_id.clone(),
        bucket_name: upload.bucket_name.clone(),
        object_key: upload.object_key.clone(),
    }
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> UploadExecutor for LiveExecutor<C, T, X> {
    async fn upload<K: Checkpointer>(&self, job: &Job, checkpointer: &K) -> Result<(), UploadErr> {
        let upload = match self.plan(job, checkpointer).await? {
            Plan::Done => return Ok(()),
            Plan::Confirm(upload) => upload,
            Plan::Transfer {
                upload,
                credentials,
                state,
            } => {
                let destination = destination(&upload);
                info!(
                    "upload: transferring file {} to bucket {} key {}",
                    job.file, destination.bucket_name, destination.object_key
                );
                self.transfer
                    .transfer(
                        &credentials,
                        &destination,
                        &job.file,
                        &upload.metadata,
                        state.as_ref(),
                        checkpointer,
                    )
                    .await?;
                checkpointer.save(Checkpoint::Transferred).await;
                upload
            }
        };

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
        filesys::files as test_files,
        http_client::{Call, MockClient},
        upload::{make_job, pending_response, token_manager},
    };
    use miru_agent::data_uploads::upload::{
        LiveExecutor, NoopCheckpointer, SdkTransfer, UploadExecutor,
    };
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
