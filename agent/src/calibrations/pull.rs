//! Pulling the control plane's latest calibrations into a latest calibrations
//! store, merging by `reported_at`.

// standard crates
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{
    errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata, Filter,
};
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
    HTTPErr,
};

// external crates
use backend_api::models::Calibration;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Put the control plane's latest calibration for each key into `store`,
/// unless the store's entry is as new or newer. Skipped keys aren't downloaded.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), CalibrationsErr> {
    let calibrations = list_latest(http_client, token).await?;
    let stored = store.list(&Filter::default()).await?;
    let mut put = 0;
    for calibration in calibrations {
        put += usize::from(pull_one(store, &stored, http_client, token, calibration).await?);
    }
    info!("calibrations: wrote {put} latest calibrations from the control plane");
    Ok(())
}

async fn list_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    token: &str,
) -> Result<Vec<Calibration>, HTTPErr> {
    http::with_retry(|| {
        http::calibrations::list_all_latest(
            http_client,
            ListAllLatestParams {
                expansions: &["reported_by"],
                token,
            },
        )
    })
    .await
}

/// Put `calibration` into `store` unless `stored` has its key as new or newer.
/// Returns whether it was put.
async fn pull_one<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    stored: &[ReportMetadata],
    http_client: &HTTPClientT,
    token: &str,
    calibration: Calibration,
) -> Result<bool, CalibrationsErr> {
    let id = calibration.id.clone();
    let metadata = ReportMetadata::from(calibration);
    // `stored` predates the downloads, so this check only skips downloads.
    // put_if_newer checks again as it writes, in case intake put a newer
    // report for the key in the meantime.
    if stored.iter().any(|entry| entry.is_as_new_as(&metadata)) {
        return Ok(false);
    }
    let data = download(http_client, &id, token).await?;
    store.put_if_newer(metadata, &data).await
}

async fn download<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    id: &str,
    token: &str,
) -> Result<String, HTTPErr> {
    http::with_retry(|| {
        http::calibrations::get_content(http_client, GetContentParams { id, token })
    })
    .await
}

/// Pulls the control plane's latest calibrations into the store until a pull
/// succeeds this run. Concurrent callers share the result of the pull already
/// running.
#[derive(Debug)]
pub struct LatestPuller<HTTPClientT, TokenManagerT> {
    store: LatestStore,
    http_client: Arc<HTTPClientT>,
    token_mngr: Arc<TokenManagerT>,
    has_pulled: Mutex<bool>,
    attempts: AtomicU64,
}

impl<HTTPClientT, TokenManagerT> LatestPuller<HTTPClientT, TokenManagerT>
where
    HTTPClientT: http::ClientI,
    TokenManagerT: TokenManagerExt,
{
    pub fn new(
        store: LatestStore,
        http_client: Arc<HTTPClientT>,
        token_mngr: Arc<TokenManagerT>,
    ) -> Self {
        Self {
            store,
            http_client,
            token_mngr,
            has_pulled: Mutex::new(false),
            attempts: AtomicU64::new(0),
        }
    }

    /// The store this puller pulls into.
    pub fn store(&self) -> &LatestStore {
        &self.store
    }

    /// Pulls unless a pull has succeeded this run, giving up once `shutdown`
    /// resolves. A failure is logged, and the next call tries again, but a
    /// caller that waited for a pull doesn't.
    pub async fn ensure_pulled(&self, shutdown: impl Future<Output = ()>) {
        tokio::select! {
            _ = shutdown => {}
            _ = self.pull_unless_pulled() => {}
        }
    }

    async fn pull_unless_pulled(&self) {
        let seen = self.attempts.load(Ordering::Acquire);
        let mut has_pulled = self.has_pulled.lock().await;
        if *has_pulled || self.attempts.load(Ordering::Acquire) != seen {
            return;
        }
        match self.pull().await {
            Ok(()) => *has_pulled = true,
            Err(e) => warn!("calibrations: unable to pull the latest calibrations: {e}"),
        }
        self.attempts.fetch_add(1, Ordering::Release);
    }

    async fn pull(&self) -> Result<(), CalibrationsErr> {
        let token = self.token_mngr.get_token().await?;
        pull_latest(&self.store, self.http_client.as_ref(), &token.token).await
    }
}
