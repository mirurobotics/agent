// standard crates
use std::fmt::Display;
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::data_uploads::upload::{
    errors::{classified_executor_err, UploadErr},
    job::{Job, UploadCheckpoint},
    transfer::ObjectTransfer,
};
use crate::errors::{Error, HTTPCode};
use crate::http::{self, ClientI};
use backend_api::models::{CreateUploadRequest, UploadCredentials, UploadSource};

// external crates
use chrono::{Local, TimeZone};
use tracing::{debug, info};

/// Uploads a job's file and confirms the upload with Miru.
///
/// # Cancel safety
///
/// Either future may be dropped at any await point. [`Self::prepare`] resumes
/// from the job's checkpoint.
pub trait UploadExecutor: Send + Sync {
    /// Reuses or creates the job's upload. `None` if it was already confirmed.
    fn prepare(
        &self,
        job: &Job,
    ) -> impl std::future::Future<Output = Result<Option<PreparedUpload>, UploadErr>> + Send;

    /// Transfers the file and confirms the upload.
    fn upload(
        &self,
        job: &Job,
        prepared: &PreparedUpload,
    ) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
}

/// What [`UploadExecutor::prepare`] hands to [`UploadExecutor::upload`].
#[derive(Debug, PartialEq)]
pub struct PreparedUpload {
    pub checkpoint: UploadCheckpoint,
    pub credentials: UploadCredentials,
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

    /// The job's upload with fresh credentials, or a new upload if the backend
    /// no longer accepts it. `None` if it was already uploaded.
    async fn reuse_or_create_upload(
        &self,
        job: &Job,
    ) -> Result<Option<(UploadCheckpoint, UploadCredentials)>, UploadErr> {
        let Some(checkpoint) = &job.checkpoint else {
            return self.create_upload(job).await.map(Some);
        };
        let id = &checkpoint.miru_upload_id;
        if let Some(credentials) = self.vend_credentials(id).await? {
            return Ok(Some((checkpoint.clone(), credentials)));
        }
        if self.was_uploaded(id).await? {
            info!("upload: upload {id} was already confirmed");
            return Ok(None);
        }
        info!(
            "upload: upload {id} is gone; creating a new one for file {}",
            job.file
        );
        self.create_upload(job).await.map(Some)
    }

    async fn create_upload(
        &self,
        job: &Job,
    ) -> Result<(UploadCheckpoint, UploadCredentials), UploadErr> {
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
        let checkpoint = UploadCheckpoint {
            miru_upload_id: resp.upload.id,
            bucket_name: resp.upload.destination.bucket_name,
            object_key: resp.upload.destination.object_key,
            metadata: resp.metadata,
            multipart: None,
        };
        Ok((checkpoint, *resp.credentials))
    }

    /// Fresh credentials for an existing upload, or `None` if it is gone:
    /// deleted (404), or uploaded or failed (409).
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
        match result {
            Ok(credentials) => Ok(Some(credentials)),
            Err(e) if matches!(e.http_status(), HTTPCode::NOT_FOUND | HTTPCode::CONFLICT) => {
                Ok(None)
            }
            Err(e) => Err(classified_executor_err(e)),
        }
    }

    /// Whether a gone upload was uploaded: confirm rejects a deleted or failed one.
    async fn was_uploaded(&self, id: &str) -> Result<bool, UploadErr> {
        match self.confirm_upload(id).await {
            Ok(()) => Ok(true),
            Err(err) if err.is_terminal() => Ok(false),
            Err(err) => Err(err),
        }
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
}

impl<C: ClientI, T: TokenManagerExt, X: ObjectTransfer> UploadExecutor for LiveExecutor<C, T, X> {
    async fn prepare(&self, job: &Job) -> Result<Option<PreparedUpload>, UploadErr> {
        let Some((mut checkpoint, credentials)) = self.reuse_or_create_upload(job).await? else {
            return Ok(None);
        };
        // Best effort: on failure, the transfer rechecks the saved multipart upload.
        match self
            .transfer
            .reuse_or_create_multipart(&credentials, &checkpoint, &job.file)
            .await
        {
            Ok(multipart) => checkpoint.multipart = multipart,
            Err(err) => debug!(
                "upload: can't prepare multipart upload for {}: {err:?}",
                job.file
            ),
        }
        Ok(Some(PreparedUpload {
            checkpoint,
            credentials,
        }))
    }

    async fn upload(&self, job: &Job, prepared: &PreparedUpload) -> Result<(), UploadErr> {
        let checkpoint = &prepared.checkpoint;
        info!(
            "upload: transferring file {} to bucket {} key {}",
            job.file, checkpoint.bucket_name, checkpoint.object_key
        );
        self.transfer
            .transfer(&prepared.credentials, checkpoint, &job.file)
            .await?;

        info!(
            "upload: confirming upload {} for file {}",
            checkpoint.miru_upload_id, job.file
        );
        self.confirm_upload(&checkpoint.miru_upload_id).await
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

        let prepared = executor.prepare(&job).await.unwrap().unwrap();
        executor.upload(&job, &prepared).await.unwrap();

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
