//! Pulling the control plane's latest calibrations into a latest calibrations
//! store, merging by `reported_at`.

// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{
    errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata, Filter,
};
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Put the control plane's latest calibration for each key into `store`,
/// unless the store's entry is as new or newer. Skipped keys aren't downloaded.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), CalibrationsErr> {
    let calibrations = http::with_retry(|| {
        http::calibrations::list_all_latest(
            http_client,
            ListAllLatestParams {
                expansions: &["reported_by"],
                token,
            },
        )
    })
    .await?;
    let stored = store.list(&Filter::default()).await?;

    let mut put = 0;
    for calibration in calibrations {
        let id = calibration.id.clone();
        let metadata = ReportMetadata::from(calibration);
        // `stored` predates the downloads, so this check only skips downloads.
        // put_if_newer checks again as it writes, in case intake put a newer
        // report for the key in the meantime.
        if stored.iter().any(|entry| entry.is_as_new_as(&metadata)) {
            continue;
        }
        let data = http::with_retry(|| {
            http::calibrations::get_content(http_client, GetContentParams { id: &id, token })
        })
        .await?;
        put += usize::from(store.put_if_newer(metadata, &data).await?);
    }
    info!("calibrations: wrote {put} latest calibrations from the control plane");
    Ok(())
}

/// Pulls the control plane's latest calibrations into the store until a pull
/// succeeds this run. Concurrent callers wait for the pull already running.
#[derive(Debug)]
pub struct LatestPuller<HTTPClientT, TokenManagerT> {
    store: LatestStore,
    http_client: Arc<HTTPClientT>,
    token_mngr: Arc<TokenManagerT>,
    has_pulled: Mutex<bool>,
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
        }
    }

    /// Pulls unless a pull has succeeded this run. A failure is logged, and the
    /// next call tries again.
    pub async fn ensure_pulled(&self) {
        let mut has_pulled = self.has_pulled.lock().await;
        if *has_pulled {
            return;
        }
        match self.pull().await {
            Ok(()) => *has_pulled = true,
            Err(e) => warn!("calibrations: unable to pull the latest calibrations: {e}"),
        }
    }

    async fn pull(&self) -> Result<(), CalibrationsErr> {
        let token = self.token_mngr.get_token().await?;
        pull_latest(&self.store, self.http_client.as_ref(), &token.token).await
    }
}
