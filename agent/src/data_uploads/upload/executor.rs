// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::data_uploads::upload::{
    errors::{classified_executor_err, UploadErr},
    job::Job,
    transfer::ObjectTransfer,
};
use crate::http::{self, ClientI};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadWithCredentials};

// external crates
use tracing::info;

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
/// point. An interrupted transfer is re-driven from the persisted queue, which keeps
/// the entry until the upload is confirmed; duplicate transfers are absorbed by
/// backend digest dedup. Scanner re-observation does NOT re-drive it — the scanner
/// ledger records that a file was reported, not that it was uploaded.
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
        self.token_mngr
            .get_token()
            .await
            .map_err(classified_executor_err)
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
        Ok(())
    }
}

pub fn new_upl_request(job: &Job) -> CreateUploadRequest {
    CreateUploadRequest {
        file_rule_id: job.file_rule_id.clone(),
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
            SdkTransfer::with_s3_http_client(replay.clone()),
        );

        executor.upload(&job).await.unwrap();

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
