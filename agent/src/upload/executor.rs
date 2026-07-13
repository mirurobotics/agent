// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::authn::{TokenManager, TokenManagerExt};
use crate::filesys::PathExt;
use crate::http::{self, ClientI};
use crate::trace;
use crate::upload::{
    errors::{ExecutorErr, UploadErr},
    job::Job,
    transfer::ObjectTransfer,
};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus};

// external crates
use tracing::info;

/// The seam between the upload actor and the transfer mechanics.
///
/// [`BrokerExecutor`] is the production implementation; it fetches short-lived
/// downscoped cloud credentials from the backend (`POST /uploads`), transfers
/// the file with the native storage SDK, then confirms the upload. This trait
/// keeps the actor independent of those mechanics.
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

/// Placeholder executor that logs the job and reports success. Stands in where
/// a no-op executor is wanted (e.g. dry runs) alongside [`BrokerExecutor`].
pub struct LogExecutor;

impl UploadExecutor for LogExecutor {
    async fn upload(&self, job: &Job) -> Result<(), UploadErr> {
        info!("LogExecutor: pretending to upload {job:?}");
        Ok(())
    }
}

/// Minimal seam for obtaining a fresh backend session token. Wraps
/// [`TokenManagerExt::get_token`] behind a `Send` future so [`BrokerExecutor`]
/// stays `Send` (the ext trait's future carries no `Send` bound) while staying
/// unit-testable with a stub token source.
pub trait TokenSource: Send + Sync {
    fn token(&self) -> impl Future<Output = Result<String, UploadErr>> + Send;
}

impl TokenSource for Arc<TokenManager> {
    async fn token(&self) -> Result<String, UploadErr> {
        let token = self.get_token().await.map_err(executor_err)?;
        Ok(token.token.clone())
    }
}

/// The production [`UploadExecutor`]: runs the broker upload flow end to end.
/// It creates the upload (minting downscoped credentials), short-circuits when
/// the backend already holds the digest, transfers the bytes via the injected
/// [`ObjectTransfer`], then confirms the durable write. The HTTP calls retry on
/// transient network failures; the transfer's own retries live in the SDK.
pub struct BrokerExecutor<C, S, X> {
    client: C,
    token: S,
    transfer: X,
}

impl<C, S, X> BrokerExecutor<C, S, X>
where
    C: ClientI,
    S: TokenSource,
    X: ObjectTransfer,
{
    pub fn new(client: C, token: S, transfer: X) -> Self {
        Self {
            client,
            token,
            transfer,
        }
    }
}

impl<C, S, X> UploadExecutor for BrokerExecutor<C, S, X>
where
    C: ClientI,
    S: TokenSource,
    X: ObjectTransfer,
{
    async fn upload(&self, job: &Job) -> Result<(), UploadErr> {
        let token = self.token.token().await?;

        let request = CreateUploadRequest {
            upload_rule_id: job.upload_rule_id.clone(),
            source: Box::new(UploadSource {
                file_path: job.file.path().to_string_lossy().into_owned(),
                file_modified_at: job.mtime.to_rfc3339(),
            }),
            digest: job.digest.clone(),
            size: job.size as i64,
            incomplete: None,
            release_id: job.release_id.clone(),
            deployment_id: job.deployment_id.clone(),
        };

        let created = http::with_retry(|| async {
            http::uploads::create(
                &self.client,
                http::uploads::CreateParams {
                    payload: &request,
                    token: &token,
                },
            )
            .await
        })
        .await
        .map_err(executor_err)?;

        // The backend dedups by digest at mint time: an already-durable object
        // comes back `uploaded`, so there is nothing to transfer or confirm.
        if created.upload.status == UploadStatus::UPLOAD_STATUS_UPLOADED {
            info!(
                "upload {} for {} already durable (digest dedup); skipping transfer",
                created.upload.id, job.file
            );
            return Ok(());
        }

        self.transfer
            .transfer(&created.credentials, &created.upload.destination, &job.file)
            .await?;

        http::with_retry(|| async {
            http::uploads::confirm(
                &self.client,
                http::uploads::ConfirmParams {
                    upload_id: &created.upload.id,
                    token: &token,
                },
            )
            .await
        })
        .await
        .map_err(executor_err)?;

        info!(
            "uploaded {} to {}",
            job.file, created.upload.destination.object_key
        );
        Ok(())
    }
}

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single
/// error surface the actor sees from an executor.
fn executor_err<E>(source: E) -> UploadErr
where
    E: std::error::Error + Send + Sync + 'static,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(source),
        trace: trace!(),
    })
}
