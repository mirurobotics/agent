// standard crates
use std::sync::Arc;
use std::time::{Duration, Instant};

// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::cooldown;
use crate::deploy::apply;
use crate::disk;
use crate::errors::*;
use crate::events;
use crate::filesys::File;
use crate::http;
use crate::sync::{
    deployments,
    errors::*,
    syncer::{CooldownEnd, State, SyncEvent, SyncFailure, SyncerArgs},
    system_metadata,
};
use crate::trace;

// external crates
use chrono::{TimeDelta, Utc};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

pub struct SyncerImpl<HTTPClientT> {
    http_client: Arc<HTTPClientT>,
    storage: Arc<disk::Storage>,
    token_mngr: Arc<authn::TokenManager>,
    deploy_opts: apply::DeployOpts,
    event_hub: events::EventHub,
    system_metadata_cache: File,

    // subscribers
    subscriber_tx: watch::Sender<SyncEvent>,
    subscriber_rx: watch::Receiver<SyncEvent>,

    // syncer state
    backoff: cooldown::Backoff,
    state: State,
}

impl<HTTPClientT: http::ClientI> SyncerImpl<HTTPClientT> {
    pub fn new(args: SyncerArgs<HTTPClientT, authn::TokenManager>) -> Self {
        let (subscriber_tx, subscriber_rx) = watch::channel(SyncEvent::SyncSuccess);
        Self {
            storage: args.storage,
            http_client: args.http_client,
            token_mngr: args.token_mngr,
            deploy_opts: args.deploy_opts,
            backoff: args.backoff,
            event_hub: args.event_hub,
            system_metadata_cache: args.system_metadata_cache,
            state: State::default(),
            subscriber_tx,
            subscriber_rx,
        }
    }

    pub(super) fn subscribe(&self) -> Result<watch::Receiver<SyncEvent>, SyncErr> {
        Ok(self.subscriber_rx.clone())
    }

    fn schedule_cooldown_end_notification(&self, wait: TimeDelta, source: CooldownEnd) {
        if wait <= TimeDelta::zero() {
            return;
        }
        // add 1 second to the cooldown period to ensure that the cooldown period is
        // cleared when sending the cooldown end event.
        let cooldown_secs = (wait.num_seconds().max(0) + 1) as u64;
        let tx = self.subscriber_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(cooldown_secs)).await;
            if let Err(e) = tx.send(SyncEvent::CooldownEnd(source)) {
                error!("failed to send cooldown ended event: {:?}", e);
            }
        });
    }

    pub(super) async fn get_sync_state(&self) -> Result<State, SyncErr> {
        Ok(self.state.clone())
    }

    #[cfg(test)]
    pub(super) fn set_sync_state(&mut self, state: State) {
        self.state = state;
    }

    pub(super) async fn sync_if_not_in_cooldown(&mut self) -> Result<(), SyncErr> {
        if self.state.is_in_cooldown() {
            info!("skipping device sync since the cooldown ends at {:?} (err streak: {}, last successful sync at: {:?})",
                self.state.cooldown_ends_at,
                self.state.err_streak,
                self.state.last_synced_at
            );
            return Ok(());
        }
        self.sync().await
    }

    pub(super) async fn sync(&mut self) -> Result<(), SyncErr> {
        if self.state.is_in_cooldown() {
            return Err(SyncErr::InCooldownErr(SyncerInCooldownErr {
                err_streak: self.state.err_streak,
                cooldown_ends_at: self.state.cooldown_ends_at,
                trace: trace!(),
            }));
        }

        self.state.last_attempted_sync_at = Utc::now();
        let result = self.sync_impl().await;

        // determine the syncer's own cooldown period
        let (event, sync_wait) = match &result {
            Ok(_) => (CooldownEnd::SyncSuccess, self.handle_sync_success()),
            Err(e) => (CooldownEnd::SyncFailure, self.handle_sync_failure(e)),
        };
        self.state.cooldown_ends_at = Utc::now() + sync_wait;
        self.schedule_cooldown_end_notification(sync_wait, event);
        debug!(
            "backend syncer cooling down for {sync_wait} (until {:?})",
            self.state.cooldown_ends_at
        );

        // send an orthogonal notification when the next deployment may be attempted
        if let Ok(Some(deployment_wait)) = result {
            self.schedule_cooldown_end_notification(deployment_wait, CooldownEnd::DeploymentWait);
        }

        result.map(|_| ())
    }

    fn handle_sync_success(&mut self) -> TimeDelta {
        if let Err(e) = self.subscriber_tx.send(SyncEvent::SyncSuccess) {
            error!("failed to send sync success event: {:?}", e);
        }
        if self.state.err_streak > 0 {
            info!(
                "successfully synced with backend after an error streak of {}",
                self.state.err_streak
            );
        } else {
            info!("successfully synced with backend");
        }
        self.state.last_synced_at = Utc::now();
        self.state.last_synced = Some(Instant::now());
        self.state.err_streak = 0;
        TimeDelta::seconds(self.backoff.base_secs)
    }

    fn handle_sync_failure(&mut self, e: &SyncErr) -> TimeDelta {
        if let Err(e) = self.subscriber_tx.send(SyncEvent::SyncFailed(SyncFailure {
            is_network_conn_err: e.is_network_conn_err(),
        })) {
            error!("failed to send sync failed event: {:?}", e);
        }
        // network connection errors are expected to happen and do not count
        // toward the error streak. We want to be able to retry syncing from
        // network connection errors even if the previous errors were not
        // network connection errors so we use an error streak of 0 when
        // calculating the cooldown period
        if e.is_network_conn_err() {
            debug!(
                "unable to sync with backend due to a network connection error: {:?}",
                e
            );
            TimeDelta::seconds(self.backoff.base_secs)
        } else {
            error!("unable to sync with backend: {:?}", e);
            self.state.err_streak += 1;
            TimeDelta::seconds(cooldown::calc(&self.backoff, self.state.err_streak))
        }
    }

    async fn sync_impl(&mut self) -> Result<Option<chrono::TimeDelta>, SyncErr> {
        let token = self.token_mngr.get_token().await?;

        let storage_ref = self.storage.as_ref();
        let sync_storage = deployments::Storage {
            deployments: storage_ref.deployments.as_ref(),
            cfg_insts: storage_ref.cfg_insts.as_ref(),
            releases: storage_ref.releases.as_ref(),
            git_commits: storage_ref.git_commits.as_ref(),
            file_rules: storage_ref.file_rules.as_ref(),
        };
        let result = deployments::sync(&deployments::SyncArgs {
            http_client: self.http_client.as_ref(),
            storage: &sync_storage,
            opts: &self.deploy_opts,
            token: &token.token,
            event_hub: &self.event_hub,
        })
        .await;

        // skip while the backend is unreachable; the next sync retries
        if !matches!(&result, Err(e) if e.is_network_conn_err()) {
            self.sync_system_metadata(&token.token).await;
        }
        result
    }

    /// Best-effort: failures are logged and never affect the sync result or its
    /// cooldown.
    async fn sync_system_metadata(&self, token: &str) {
        let args = system_metadata::SyncArgs {
            http_client: self.http_client.as_ref(),
            device: self.storage.device.as_ref(),
            cache_file: &self.system_metadata_cache,
            token,
        };
        match system_metadata::sync(&args).await {
            Ok(outcome) => debug!("system metadata sync completed: {outcome:?}"),
            Err(e) => warn!("unable to sync system metadata: {e}"),
        }
    }
}
