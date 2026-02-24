// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::authn::token_mngr::{TokenManager, TokenManagerExt};
use crate::cooldown;
use crate::deploy::apply;
use crate::errors::*;
use crate::http;
use crate::storage;
use crate::sync::errors::*;
use crate::sync::{agent_version, deployments};
use crate::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

// =============================== SYNCER EVENTS ================================== //
#[derive(Debug, Clone, PartialEq)]
pub struct SyncFailure {
    pub is_network_conn_err: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CooldownEnd {
    SyncSuccess,
    SyncFailure,
    DeploymentWait,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncEvent {
    SyncSuccess,
    SyncFailed(SyncFailure),
    CooldownEnd(CooldownEnd),
}

// ======================== SINGLE-THREADED IMPLEMENTATION ========================= //
pub struct SyncerArgs<HTTPClientT, TokenManagerT: TokenManagerExt> {
    pub storage: Arc<storage::Storage>,
    pub http_client: Arc<HTTPClientT>,
    pub token_mngr: Arc<TokenManagerT>,
    pub deploy_opts: apply::DeployOpts,
    pub backoff: cooldown::Backoff,
    pub agent_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub last_attempted_sync_at: DateTime<Utc>,
    pub last_synced_at: DateTime<Utc>,
    pub cooldown_ends_at: DateTime<Utc>,
    pub err_streak: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
            err_streak: 0,
        }
    }
}

impl State {
    pub fn is_in_cooldown(&self) -> bool {
        Utc::now() < self.cooldown_ends_at
    }
}

pub struct SingleThreadSyncer<HTTPClientT> {
    http_client: Arc<HTTPClientT>,
    storage: Arc<storage::Storage>,
    token_mngr: Arc<TokenManager>,
    deploy_opts: apply::DeployOpts,
    agent_version: String,

    // subscribers
    subscriber_tx: watch::Sender<SyncEvent>,
    subscriber_rx: watch::Receiver<SyncEvent>,

    // syncer state
    backoff: cooldown::Backoff,
    state: State,
}

impl<HTTPClientT: http::ClientI> SingleThreadSyncer<HTTPClientT> {
    pub fn new(args: SyncerArgs<HTTPClientT, TokenManager>) -> Self {
        let (subscriber_tx, subscriber_rx) = watch::channel(SyncEvent::SyncSuccess);
        Self {
            storage: args.storage,
            http_client: args.http_client,
            token_mngr: args.token_mngr,
            deploy_opts: args.deploy_opts,
            backoff: args.backoff,
            agent_version: args.agent_version,
            state: State::default(),
            subscriber_tx,
            subscriber_rx,
        }
    }

    fn subscribe(&self) -> Result<watch::Receiver<SyncEvent>, SyncErr> {
        Ok(self.subscriber_rx.clone())
    }

    fn schedule_cooldown_end_notification(&self, wait: TimeDelta, source: CooldownEnd) {
        if wait <= TimeDelta::zero() {
            return;
        }
        // add 1 second to the cooldown period to ensure that the cooldown period is
        // cleared when sending the cooldown end event.
        let cooldown_secs = wait.num_seconds().max(0) + 1;
        let tx = self.subscriber_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(cooldown_secs as u64)).await;
            if let Err(e) = tx.send(SyncEvent::CooldownEnd(source)) {
                error!("failed to send cooldown ended event: {:?}", e);
            }
        });
    }

    async fn get_sync_state(&self) -> Result<State, SyncErr> {
        Ok(self.state.clone())
    }

    #[cfg(feature = "test")]
    fn set_sync_state(&mut self, state: State) {
        self.state = state;
    }

    async fn sync_if_not_in_cooldown(&mut self) -> Result<(), SyncErr> {
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

    async fn sync(&mut self) -> Result<(), SyncErr> {
        if self.state.is_in_cooldown() {
            return Err(SyncErr::InCooldownErr(SyncerInCooldownErr {
                err_streak: self.state.err_streak,
                cooldown_ends_at: self.state.cooldown_ends_at,
                trace: trace!(),
            }));
        }

        self.state.last_attempted_sync_at = Utc::now();
        let result = self.sync_impl().await;
        let (event, wait) = match &result {
            Ok(None) => (CooldownEnd::SyncSuccess, self.handle_sync_success()),
            Ok(Some(deployment_wait)) => {
                let success_wait = self.handle_sync_success();
                if *deployment_wait >= success_wait {
                    (CooldownEnd::DeploymentWait, *deployment_wait)
                } else {
                    (CooldownEnd::SyncSuccess, success_wait)
                }
            }
            Err(e) => (CooldownEnd::SyncFailure, self.handle_sync_failure(e)),
        };

        self.state.cooldown_ends_at = Utc::now() + wait;
        self.schedule_cooldown_end_notification(wait, event);
        debug!(
            "backend syncer cooling down for {wait} (until {:?})",
            self.state.cooldown_ends_at
        );

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

        if let Err(e) = agent_version::push(
            self.storage.device.as_ref(),
            self.http_client.as_ref(),
            &token.token,
            self.agent_version.clone(),
        )
        .await
        {
            error!("failed to push agent version to backend: {:?}", e);
        }

        let storage_ref = self.storage.as_ref();
        let apply_storage = apply::Storage {
            deployments: storage_ref.deployments.as_ref(),
            cfg_insts: storage_ref.cfg_insts.as_ref(),
        };
        deployments::sync(&deployments::SyncArgs {
            http_client: self.http_client.as_ref(),
            storage: &apply_storage,
            opts: &self.deploy_opts,
            token: &token.token,
        })
        .await
    }
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
#[allow(async_fn_in_trait)]
pub trait SyncerExt {
    async fn shutdown(&self) -> Result<(), SyncErr>;
    async fn get_sync_state(&self) -> Result<State, SyncErr>;
    async fn is_in_cooldown(&self) -> Result<bool, SyncErr>;
    async fn get_cooldown_ends_at(&self) -> Result<DateTime<Utc>, SyncErr>;
    async fn get_last_attempted_sync_at(&self) -> Result<DateTime<Utc>, SyncErr>;
    async fn sync(&self) -> Result<(), SyncErr>;
    async fn sync_if_not_in_cooldown(&self) -> Result<(), SyncErr>;
    async fn subscribe(&self) -> Result<watch::Receiver<SyncEvent>, SyncErr>;
}

pub enum WorkerCommand {
    Shutdown {
        respond_to: oneshot::Sender<Result<(), SyncErr>>,
    },
    GetSyncState {
        respond_to: oneshot::Sender<Result<State, SyncErr>>,
    },
    #[cfg(feature = "test")]
    SetSyncState {
        state: State,
        respond_to: oneshot::Sender<Result<(), SyncErr>>,
    },
    SyncIfNotInCooldown {
        respond_to: oneshot::Sender<Result<(), SyncErr>>,
    },
    Sync {
        respond_to: oneshot::Sender<Result<(), SyncErr>>,
    },
    Subscribe {
        respond_to: oneshot::Sender<Result<watch::Receiver<SyncEvent>, SyncErr>>,
    },
}

pub struct Worker<HTTPClientT: Send> {
    syncer: SingleThreadSyncer<HTTPClientT>,
    receiver: mpsc::Receiver<WorkerCommand>,
}

impl<HTTPClientT: Send> Worker<HTTPClientT> {
    pub fn new(
        syncer: SingleThreadSyncer<HTTPClientT>,
        receiver: mpsc::Receiver<WorkerCommand>,
    ) -> Self {
        Self { syncer, receiver }
    }
}

impl<HTTPClientT: http::ClientI> Worker<HTTPClientT> {
    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                WorkerCommand::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                WorkerCommand::GetSyncState { respond_to } => {
                    let result = self.syncer.get_sync_state().await;
                    if let Err(e) = respond_to.send(result) {
                        error!("Actor failed to send state response: {:?}", e);
                    }
                }
                #[cfg(feature = "test")]
                WorkerCommand::SetSyncState { state, respond_to } => {
                    self.syncer.set_sync_state(state);
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send set sync state response: {:?}", e);
                    }
                }
                WorkerCommand::SyncIfNotInCooldown { respond_to } => {
                    let result = self.syncer.sync_if_not_in_cooldown().await;
                    if let Err(e) = respond_to.send(result) {
                        error!(
                            "Actor failed to send sync if not in cooldown response: {:?}",
                            e
                        );
                    }
                }
                WorkerCommand::Sync { respond_to } => {
                    let result = self.syncer.sync().await;
                    if let Err(e) = respond_to.send(result) {
                        error!("Actor failed to send sync response: {:?}", e);
                    }
                }
                WorkerCommand::Subscribe { respond_to } => {
                    let result = self.syncer.subscribe();
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to send subscribe response");
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Syncer {
    sender: mpsc::Sender<WorkerCommand>,
}

impl Syncer {
    pub fn spawn(
        buffer_size: usize,
        args: SyncerArgs<http::Client, TokenManager>,
    ) -> Result<(Self, JoinHandle<()>), SyncErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            syncer: SingleThreadSyncer::new(args),
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    pub fn new(sender: mpsc::Sender<WorkerCommand>) -> Self {
        Self { sender }
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> WorkerCommand,
    ) -> Result<R, SyncErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            SyncErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            SyncErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    #[cfg(feature = "test")]
    pub async fn set_sync_state(&self, state: State) -> Result<(), SyncErr> {
        self.send_command(|tx| WorkerCommand::SetSyncState {
            state,
            respond_to: tx,
        })
        .await?
    }
}

impl SyncerExt for Syncer {
    async fn shutdown(&self) -> Result<(), SyncErr> {
        self.send_command(|tx| WorkerCommand::Shutdown { respond_to: tx })
            .await??;
        info!("Syncer shutdown complete");
        Ok(())
    }

    async fn get_sync_state(&self) -> Result<State, SyncErr> {
        self.send_command(|tx| WorkerCommand::GetSyncState { respond_to: tx })
            .await?
    }

    async fn is_in_cooldown(&self) -> Result<bool, SyncErr> {
        let state = self.get_sync_state().await?;
        Ok(state.is_in_cooldown())
    }

    async fn get_cooldown_ends_at(&self) -> Result<DateTime<Utc>, SyncErr> {
        let state = self.get_sync_state().await?;
        Ok(state.cooldown_ends_at)
    }

    async fn get_last_attempted_sync_at(&self) -> Result<DateTime<Utc>, SyncErr> {
        let state = self.get_sync_state().await?;
        Ok(state.last_attempted_sync_at)
    }

    async fn sync_if_not_in_cooldown(&self) -> Result<(), SyncErr> {
        self.send_command(|tx| WorkerCommand::SyncIfNotInCooldown { respond_to: tx })
            .await?
    }

    async fn sync(&self) -> Result<(), SyncErr> {
        self.send_command(|tx| WorkerCommand::Sync { respond_to: tx })
            .await?
    }

    async fn subscribe(&self) -> Result<watch::Receiver<SyncEvent>, SyncErr> {
        self.send_command(|tx| WorkerCommand::Subscribe { respond_to: tx })
            .await?
    }
}
