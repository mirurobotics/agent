// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::cooldown;
use crate::deploy::apply;
use crate::disk;
use crate::errors::*;
use crate::events;
use crate::http;
use crate::sync::{deployments, errors::*};
use crate::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

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
    pub storage: Arc<disk::Storage>,
    pub http_client: Arc<HTTPClientT>,
    pub token_mngr: Arc<TokenManagerT>,
    pub deploy_opts: apply::DeployOpts,
    pub backoff: cooldown::Backoff,
    pub event_hub: events::EventHub,
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
    storage: Arc<disk::Storage>,
    token_mngr: Arc<authn::TokenManager>,
    deploy_opts: apply::DeployOpts,
    event_hub: events::EventHub,

    // subscribers
    subscriber_tx: watch::Sender<SyncEvent>,
    subscriber_rx: watch::Receiver<SyncEvent>,

    // syncer state
    backoff: cooldown::Backoff,
    state: State,
}

impl<HTTPClientT: http::ClientI> SingleThreadSyncer<HTTPClientT> {
    pub fn new(args: SyncerArgs<HTTPClientT, authn::TokenManager>) -> Self {
        let (subscriber_tx, subscriber_rx) = watch::channel(SyncEvent::SyncSuccess);
        Self {
            storage: args.storage,
            http_client: args.http_client,
            token_mngr: args.token_mngr,
            deploy_opts: args.deploy_opts,
            backoff: args.backoff,
            event_hub: args.event_hub,
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
        let cooldown_secs = (wait.num_seconds().max(0) + 1) as u64;
        let tx = self.subscriber_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(cooldown_secs)).await;
            if let Err(e) = tx.send(SyncEvent::CooldownEnd(source)) {
                error!("failed to send cooldown ended event: {:?}", e);
            }
        });
    }

    async fn get_sync_state(&self) -> Result<State, SyncErr> {
        Ok(self.state.clone())
    }

    #[cfg(test)]
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
        deployments::sync(&deployments::SyncArgs {
            http_client: self.http_client.as_ref(),
            storage: &sync_storage,
            opts: &self.deploy_opts,
            token: &token.token,
            event_hub: &self.event_hub,
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

pub enum Command {
    Shutdown {
        respond_to: oneshot::Sender<Result<(), SyncErr>>,
    },
    GetSyncState {
        respond_to: oneshot::Sender<Result<State, SyncErr>>,
    },
    #[cfg(test)]
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
    receiver: mpsc::Receiver<Command>,
}

impl<HTTPClientT: Send> Worker<HTTPClientT> {
    pub fn new(syncer: SingleThreadSyncer<HTTPClientT>, receiver: mpsc::Receiver<Command>) -> Self {
        Self { syncer, receiver }
    }
}

impl<HTTPClientT: http::ClientI> Worker<HTTPClientT> {
    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                Command::GetSyncState { respond_to } => {
                    dispatch!(
                        self.syncer.get_sync_state().await,
                        respond_to,
                        "Actor failed to send state response"
                    );
                }
                #[cfg(test)]
                Command::SetSyncState { state, respond_to } => {
                    self.syncer.set_sync_state(state);
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send set sync state response: {:?}", e);
                    }
                }
                Command::SyncIfNotInCooldown { respond_to } => {
                    dispatch!(
                        self.syncer.sync_if_not_in_cooldown().await,
                        respond_to,
                        "Actor failed to send sync if not in cooldown response"
                    );
                }
                Command::Sync { respond_to } => {
                    dispatch!(
                        self.syncer.sync().await,
                        respond_to,
                        "Actor failed to send sync response"
                    );
                }
                Command::Subscribe { respond_to } => {
                    dispatch!(
                        self.syncer.subscribe(),
                        respond_to,
                        "Actor failed to send subscribe response"
                    );
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Syncer {
    sender: mpsc::Sender<Command>,
}

impl Syncer {
    pub fn spawn(
        buffer_size: usize,
        args: SyncerArgs<http::Client, authn::TokenManager>,
    ) -> Result<(Self, JoinHandle<()>), SyncErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            syncer: SingleThreadSyncer::new(args),
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    pub fn new(sender: mpsc::Sender<Command>) -> Self {
        Self { sender }
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
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

    #[cfg(test)]
    pub(crate) async fn set_sync_state(&self, state: State) -> Result<(), SyncErr> {
        self.send_command(|tx| Command::SetSyncState {
            state,
            respond_to: tx,
        })
        .await?
    }
}

impl SyncerExt for Syncer {
    async fn shutdown(&self) -> Result<(), SyncErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Syncer shutdown complete");
        Ok(())
    }

    async fn get_sync_state(&self) -> Result<State, SyncErr> {
        self.send_command(|tx| Command::GetSyncState { respond_to: tx })
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
        self.send_command(|tx| Command::SyncIfNotInCooldown { respond_to: tx })
            .await?
    }

    async fn sync(&self) -> Result<(), SyncErr> {
        self.send_command(|tx| Command::Sync { respond_to: tx })
            .await?
    }

    async fn subscribe(&self) -> Result<watch::Receiver<SyncEvent>, SyncErr> {
        self.send_command(|tx| Command::Subscribe { respond_to: tx })
            .await?
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // internal crates
    use crate::test_utils::{
        filesys::dirs as test_dirs,
        http_client::{Call, MockClient},
        sync::{create_storage, create_token_manager},
        sync_helpers::*,
    };
    use miru_agent::authn::{TokenManager, TokenManagerExt};
    use miru_agent::cooldown;
    use miru_agent::deploy::{apply, fsm};
    use miru_agent::disk::Storage;
    use miru_agent::errors::*;
    use miru_agent::events::hub::{EventHub, SpawnOptions};
    use miru_agent::filesys::Overwrite;
    use miru_agent::http;
    use miru_agent::http::errors::{HTTPErr, MockErr};
    use miru_agent::models::{DplActivity, DplErrStatus, DplTarget};
    use miru_agent::sync::syncer::{
        CooldownEnd, SingleThreadSyncer, State, SyncEvent, SyncFailure, SyncerArgs, Worker,
    };
    use miru_agent::sync::{SyncErr, Syncer, SyncerExt};

    // external crates
    use chrono::{DateTime, TimeDelta, Utc};
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;

    fn spawn(
        buffer_size: usize,
        args: SyncerArgs<MockClient, TokenManager>,
    ) -> Result<(Syncer, JoinHandle<()>), SyncErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker::new(SingleThreadSyncer::new(args), receiver);
        let worker_handle = tokio::spawn(worker.run());
        Ok((Syncer::new(sender), worker_handle))
    }

    // ========================= FIXTURE ========================= //

    struct Fixture {
        _dir: test_dirs::TempDir,
        http_client: Arc<MockClient>,
        storage: Arc<Storage>,
        syncer: Syncer,
        backoff: cooldown::Backoff,
        token_mngr: Arc<TokenManager>,
    }

    impl Fixture {
        async fn new(name: &str) -> Self {
            Self::new_with_backoff(
                name,
                cooldown::Backoff {
                    base_secs: 1,
                    growth_factor: 2,
                    max_secs: 12 * 60 * 60,
                },
            )
            .await
        }

        async fn new_with_backoff(name: &str, backoff: cooldown::Backoff) -> Self {
            let dir = test_dirs::temp(name).unwrap();
            let auth_client = Arc::new(MockClient::default());
            let (token_mngr, _) = create_token_manager(dir.dir(), auth_client.clone()).await;
            let token_mngr = Arc::new(token_mngr);
            let http_client = Arc::new(MockClient::default());
            let storage = Arc::new(create_storage(dir.dir()).await);

            let log_file = dir.file("events.jsonl");
            let (event_hub, _hub_handle) = EventHub::spawn(log_file, SpawnOptions::default())
                .await
                .unwrap();

            let (syncer, _) = spawn(
                32,
                SyncerArgs {
                    storage: storage.clone(),
                    http_client: http_client.clone(),
                    token_mngr: token_mngr.clone(),
                    deploy_opts: apply::DeployOpts {
                        retry_policy: fsm::RetryPolicy::default(),
                    },
                    backoff,
                    event_hub,
                },
            )
            .unwrap();

            Self {
                _dir: dir,
                http_client,
                storage,
                syncer,
                backoff,
                token_mngr,
            }
        }

        /// Reset cooldown so the next sync() won't be rejected.
        async fn reset_cooldown(&self) {
            let state = self.syncer.get_sync_state().await.unwrap();
            self.syncer
                .set_sync_state(State {
                    cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
                    ..state
                })
                .await
                .unwrap();
        }
    }

    pub mod state {
        use super::*;

        #[test]
        fn default_values() {
            let state = State::default();
            let expected = State {
                last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
                last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
                cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
                err_streak: 0,
            };
            assert_eq!(state, expected);
        }

        #[tokio::test]
        async fn is_in_cooldown() {
            // in cooldown (cooldown_ends_at is in the future)
            let state = State {
                last_attempted_sync_at: Utc::now(),
                last_synced_at: Utc::now(),
                cooldown_ends_at: Utc::now() + TimeDelta::seconds(10),
                err_streak: 0,
            };
            assert!(state.is_in_cooldown());

            // not in cooldown (cooldown_ends_at is in the past)
            let state = State {
                last_attempted_sync_at: Utc::now(),
                last_synced_at: Utc::now(),
                cooldown_ends_at: Utc::now() - TimeDelta::seconds(10),
                err_streak: 0,
            };
            assert!(!state.is_in_cooldown());
        }
    }

    pub mod shutdown {
        use super::*;

        #[tokio::test]
        async fn shutdown() {
            let dir = test_dirs::temp("spawn").unwrap();
            let auth_client = Arc::new(MockClient::default());
            let (token_mngr, _) = create_token_manager(dir.dir(), auth_client.clone()).await;

            let storage = Arc::new(create_storage(dir.dir()).await);

            let log_file = dir.file("events.jsonl");
            let (event_hub, _hub_handle) = EventHub::spawn(log_file, SpawnOptions::default())
                .await
                .unwrap();

            let http_client = Arc::new(http::Client::new("doesntmatter").unwrap());
            let (syncer, worker_handler) = Syncer::spawn(
                32,
                SyncerArgs {
                    storage: storage.clone(),
                    http_client: http_client.clone(),
                    token_mngr: Arc::new(token_mngr),
                    deploy_opts: apply::DeployOpts {
                        retry_policy: fsm::RetryPolicy::default(),
                    },
                    backoff: cooldown::Backoff {
                        base_secs: 15,
                        growth_factor: 2,
                        max_secs: 12 * 60 * 60,
                    },
                    event_hub,
                },
            )
            .unwrap();

            syncer.shutdown().await.unwrap();
            worker_handler.await.unwrap();
        }
    }

    pub mod is_in_cooldown {
        use super::*;

        #[tokio::test]
        async fn true_when_in_cooldown() {
            let f = Fixture::new("syncer_ext_cooldown_true").await;

            f.syncer
                .set_sync_state(State {
                    cooldown_ends_at: Utc::now() + TimeDelta::seconds(60),
                    ..State::default()
                })
                .await
                .unwrap();

            assert!(f.syncer.is_in_cooldown().await.unwrap());
        }

        #[tokio::test]
        async fn false_when_not_in_cooldown() {
            let f = Fixture::new("syncer_ext_cooldown_false").await;
            // Default state has cooldown_ends_at = UNIX_EPOCH → not in cooldown
            assert!(!f.syncer.is_in_cooldown().await.unwrap());
        }
    }

    pub mod get_cooldown_ends_at {
        use super::*;

        #[tokio::test]
        async fn get_cooldown_ends_at() {
            let f = Fixture::new("syncer_ext_cooldown_ends").await;
            let target = Utc::now() + TimeDelta::seconds(120);

            f.syncer
                .set_sync_state(State {
                    cooldown_ends_at: target,
                    ..State::default()
                })
                .await
                .unwrap();

            assert_eq!(f.syncer.get_cooldown_ends_at().await.unwrap(), target);
        }
    }

    pub mod get_last_attempted_sync_at {
        use super::*;

        #[tokio::test]
        async fn get_last_attempted_sync_at() {
            let f = Fixture::new("syncer_ext_last_attempted").await;

            let before = Utc::now();
            f.syncer.sync().await.unwrap();
            let after = Utc::now();

            let ts = f.syncer.get_last_attempted_sync_at().await.unwrap();
            assert!(ts > before);
            assert!(ts < after);
        }
    }

    pub mod sync_success {
        use super::*;

        #[tokio::test]
        async fn deployments() {
            let f = Fixture::new("sync_deployments").await;

            // define a backend deployment with an embedded config instance
            let backend_dep = backend_api::models::Deployment {
                id: "dpl_1".to_string(),
                config_instances: Some(vec![backend_api::models::ConfigInstance {
                    id: "cfg_inst_1".to_string(),
                    content: Some(Box::new(backend_api::models::InstanceContent {
                        data: "{\"key\": \"value\"}".to_string(),
                        ..Default::default()
                    })),
                    ..Default::default()
                }]),
                ..Default::default()
            };

            let backend_dep_cloned = backend_dep.clone();
            f.http_client
                .set_list_all_deployments(move || Ok(vec![backend_dep_cloned.clone()]));

            let before = Utc::now();
            f.syncer.sync().await.unwrap();
            let after = Utc::now();

            assert_deployment_stored(&f.storage.deployments, "dpl_1").await;
            assert_cfg_inst_stored(&f.storage.cfg_insts.meta, "cfg_inst_1").await;
            let _content = read_content(&f.storage.cfg_insts.content, "cfg_inst_1").await;

            // check the sync state
            let state = f.syncer.get_sync_state().await.unwrap();
            assert_eq!(
                f.syncer.get_cooldown_ends_at().await.unwrap(),
                state.cooldown_ends_at
            );
            let window = StateAssert::new(before, after);
            let base_cooldown = TimeDelta::seconds(f.backoff.base_secs);
            window.assert_success(&state, base_cooldown, 0);
        }

        #[tokio::test]
        async fn success_resets_err_streak() {
            let f = Fixture::new("sync_success_resets_streak").await;

            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            });

            // cause 3 non-network failures
            for _ in 0..3 {
                f.syncer.sync().await.unwrap_err();
                f.reset_cooldown().await;
            }

            let state = f.syncer.get_sync_state().await.unwrap();
            assert_eq!(state.err_streak, 3);

            // fix the mock to succeed
            f.http_client.set_list_all_deployments(|| Ok(vec![]));

            // sync successfully
            let before = Utc::now();
            f.syncer.sync().await.unwrap();
            let after = Utc::now();

            let state = f.syncer.get_sync_state().await.unwrap();
            let window = StateAssert::new(before, after);
            let base_cooldown = TimeDelta::seconds(f.backoff.base_secs);
            window.assert_success(&state, base_cooldown, 0);
        }
    }

    pub mod sync_failure {
        use super::*;

        #[tokio::test]
        async fn in_cooldown_error() {
            let f = Fixture::new("sync_in_cooldown_error").await;

            // set the syncer state to be in cooldown
            f.syncer
                .set_sync_state(State {
                    last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
                    last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
                    cooldown_ends_at: Utc::now() + TimeDelta::seconds(10),
                    err_streak: 0,
                })
                .await
                .unwrap();

            let error = f.syncer.sync().await.unwrap_err();
            assert!(matches!(error, SyncErr::InCooldownErr(_)));
        }

        #[tokio::test]
        async fn token_manager_failure() {
            let f = Fixture::new("sync_token_mngr_failure").await;

            // Shut down the token manager so get_token() fails with AuthnErr
            f.token_mngr.shutdown().await.unwrap();
            // Give the actor a moment to stop
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let error = f.syncer.sync().await.unwrap_err();

            // check error type
            assert!(matches!(error, SyncErr::AuthnErr(_)));

            // err_streak should be incremented (non-network error)
            let state = f.syncer.get_sync_state().await.unwrap();
            assert_eq!(state.err_streak, 1);

            // no HTTP calls should have been made (sync_impl exits early)
            assert_eq!(f.http_client.call_count(Call::ListDeployments), 0);
            assert_eq!(f.http_client.call_count(Call::UpdateDevice), 0);
        }

        #[tokio::test(start_paused = true)]
        async fn network_error() {
            let f = Fixture::new("sync_network_error").await;

            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });
            f.http_client.set_update_deployment(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });

            let base_cooldown_duration = TimeDelta::seconds(f.backoff.base_secs);
            for _ in 0..10 {
                let before = Utc::now();
                let error = f.syncer.sync().await.unwrap_err();
                let after = Utc::now();

                // check error type
                assert!(error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert_eq!(
                    f.syncer.get_cooldown_ends_at().await.unwrap(),
                    state.cooldown_ends_at
                );
                let window = StateAssert::new(before, after);
                window.assert_failed(&state, base_cooldown_duration, 0);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends_at: before,
                        ..state
                    })
                    .await
                    .unwrap();
            }
        }

        #[tokio::test]
        async fn non_network_error() {
            let f = Fixture::new("sync_non_network_error").await;

            // all errors need to be a network connection error for the syncer to return a
            // network connection error so only set one false to test this
            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            });
            f.http_client.set_update_deployment(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });

            for i in 0..10 {
                let before = Utc::now();
                let error = f.syncer.sync().await.unwrap_err();
                let after = Utc::now();

                // check error type
                assert!(!error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert_eq!(
                    f.syncer.get_cooldown_ends_at().await.unwrap(),
                    state.cooldown_ends_at
                );
                let cooldown_secs = cooldown::calc(&f.backoff, i + 1);
                let cooldown_duration = TimeDelta::seconds(cooldown_secs);
                let window = StateAssert::new(before, after);
                window.assert_failed(&state, cooldown_duration, i + 1);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends_at: before,
                        ..state
                    })
                    .await
                    .unwrap();
            }
        }

        #[tokio::test(start_paused = true)]
        async fn non_network_error_to_network_error_to_recovery() {
            let f = Fixture::new("sync_nn_to_net_to_recovery").await;

            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            });
            f.http_client.set_update_deployment(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            });

            // non-network connection errors
            for i in 0..10 {
                let before = Utc::now();
                let error = f.syncer.sync().await.unwrap_err();
                let after = Utc::now();

                // check error type
                assert!(!error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert_eq!(
                    f.syncer.get_cooldown_ends_at().await.unwrap(),
                    state.cooldown_ends_at
                );
                let cooldown_secs = cooldown::calc(&f.backoff, i + 1);
                let cooldown_duration = TimeDelta::seconds(cooldown_secs);
                let window = StateAssert::new(before, after);
                window.assert_failed(&state, cooldown_duration, i + 1);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends_at: before,
                        ..state
                    })
                    .await
                    .unwrap();
            }

            // set the http client to return a network connection error
            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });
            f.http_client.set_update_deployment(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });

            // network connection errors
            let cur_err_streak = 10;
            let base_cooldown_duration = TimeDelta::seconds(f.backoff.base_secs);
            for _ in 0..10 {
                let before = Utc::now();
                let error = f.syncer.sync().await.unwrap_err();
                let after = Utc::now();

                // check error type
                assert!(error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                let window = StateAssert::new(before, after);
                window.assert_failed(&state, base_cooldown_duration, cur_err_streak);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends_at: before,
                        ..state
                    })
                    .await
                    .unwrap();
            }

            // set the http client to not return an error
            f.http_client.set_list_all_deployments(|| Ok(vec![]));
            f.http_client
                .set_update_deployment(|| Ok(backend_api::models::Deployment::default()));

            // recovery
            let base_cooldown_duration = TimeDelta::seconds(f.backoff.base_secs);
            for _ in 0..10 {
                let before = Utc::now();
                f.syncer.sync().await.unwrap();
                let after = Utc::now();

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert_eq!(
                    f.syncer.get_cooldown_ends_at().await.unwrap(),
                    state.cooldown_ends_at
                );
                let window = StateAssert::new(before, after);
                window.assert_success(&state, base_cooldown_duration, 0);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends_at: before,
                        ..state
                    })
                    .await
                    .unwrap();
            }
        }

        #[tokio::test(start_paused = true)]
        async fn network_error_preserves_err_streak() {
            let f = Fixture::new("sync_net_preserves_streak").await;

            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            });

            // Cause 3 non-network failures to build up err_streak = 3
            for _ in 0..3 {
                f.syncer.sync().await.unwrap_err();
                f.reset_cooldown().await;
            }

            let state = f.syncer.get_sync_state().await.unwrap();
            assert_eq!(state.err_streak, 3);

            // Switch mock to network error
            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });
            f.http_client.set_update_deployment(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });

            let before = Utc::now();
            let error = f.syncer.sync().await.unwrap_err();
            let after = Utc::now();
            assert!(error.is_network_conn_err());

            // err_streak should be unchanged (not incremented, not reset)
            let state = f.syncer.get_sync_state().await.unwrap();
            let window = StateAssert::new(before, after);
            let base_cooldown = TimeDelta::seconds(f.backoff.base_secs);
            window.assert_failed(&state, base_cooldown, 3);
        }
    }

    pub mod sync_if_not_in_cooldown {
        use super::*;

        #[tokio::test]
        async fn skips_when_in_cooldown() {
            let f = Fixture::new("sync_if_not_in_cooldown_skip").await;

            f.syncer
                .set_sync_state(State {
                    cooldown_ends_at: Utc::now() + TimeDelta::seconds(10),
                    ..State::default()
                })
                .await
                .unwrap();

            f.syncer.sync_if_not_in_cooldown().await.unwrap();
            let state = f.syncer.get_sync_state().await.unwrap();
            assert_eq!(state.last_attempted_sync_at, DateTime::<Utc>::UNIX_EPOCH);
        }

        #[tokio::test]
        async fn syncs_when_not_in_cooldown() {
            let f = Fixture::new("sync_if_not_in_cooldown_run").await;

            let before = Utc::now();
            f.syncer.sync_if_not_in_cooldown().await.unwrap();
            let after = Utc::now();

            let state = f.syncer.get_sync_state().await.unwrap();
            let window = StateAssert::new(before, after);
            window.assert_between(state.last_attempted_sync_at, TimeDelta::zero());
        }
    }

    pub mod subscribe {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn sync_success() {
            let f = Fixture::new("subscribe_sync_success").await;

            let mut subscriber = f.syncer.subscribe().await.unwrap();
            let events = Arc::new(Mutex::new(vec![]));

            let mut subscriber_for_spawn = subscriber.clone();
            let events_for_spawn = events.clone();
            let handle = tokio::spawn(async move {
                // expect two events: 1. sync success -> 2. cooldown ended
                for _ in 0..2 {
                    subscriber_for_spawn.changed().await.unwrap();
                    events_for_spawn
                        .lock()
                        .unwrap()
                        .push(subscriber_for_spawn.borrow().clone());
                }
            });

            f.syncer.sync().await.unwrap();
            // wait for the cooldown end event
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    subscriber.changed().await.unwrap();
                    let event = subscriber.borrow().clone();
                    if matches!(event, SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess)) {
                        break;
                    }
                }
            })
            .await
            .expect("timed out waiting for CooldownEnd::SyncSuccess");

            let events = events.lock().unwrap().clone();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0], SyncEvent::SyncSuccess);
            assert_eq!(events[1], SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess));

            handle.await.unwrap();
        }

        #[tokio::test(start_paused = true)]
        async fn network_sync_failure() {
            let f = Fixture::new("subscribe_sync_failure").await;

            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });
            f.http_client.set_update_deployment(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            });

            let mut subscriber = f.syncer.subscribe().await.unwrap();
            let events = Arc::new(Mutex::new(vec![]));

            let mut subscriber_for_spawn = subscriber.clone();
            let events_for_spawn = events.clone();
            let handle = tokio::spawn(async move {
                // expect two events: 1. sync failure -> 2. cooldown ended
                for _ in 0..2 {
                    subscriber_for_spawn.changed().await.unwrap();
                    events_for_spawn
                        .lock()
                        .unwrap()
                        .push(subscriber_for_spawn.borrow().clone());
                }
            });

            f.syncer.sync().await.unwrap_err();
            // wait for the cooldown to end
            loop {
                subscriber.changed().await.unwrap();
                let event = subscriber.borrow().clone();
                if matches!(event, SyncEvent::CooldownEnd(CooldownEnd::SyncFailure)) {
                    break;
                }
            }

            let events = events.lock().unwrap().clone();
            assert_eq!(events.len(), 2);
            assert_eq!(
                events[0],
                SyncEvent::SyncFailed(SyncFailure {
                    is_network_conn_err: true,
                })
            );
            assert_eq!(events[1], SyncEvent::CooldownEnd(CooldownEnd::SyncFailure));

            handle.await.unwrap();
        }

        #[tokio::test(start_paused = true)]
        async fn non_network_sync_failure() {
            let f = Fixture::new("subscribe_non_network_failure").await;

            f.http_client.set_list_all_deployments(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            });

            let mut subscriber = f.syncer.subscribe().await.unwrap();
            let events = Arc::new(Mutex::new(vec![]));

            let mut subscriber_for_spawn = subscriber.clone();
            let events_for_spawn = events.clone();
            let handle = tokio::spawn(async move {
                for _ in 0..2 {
                    subscriber_for_spawn.changed().await.unwrap();
                    events_for_spawn
                        .lock()
                        .unwrap()
                        .push(subscriber_for_spawn.borrow().clone());
                }
            });

            f.syncer.sync().await.unwrap_err();
            // wait for the cooldown to end
            loop {
                subscriber.changed().await.unwrap();
                let event = subscriber.borrow().clone();
                if matches!(event, SyncEvent::CooldownEnd(CooldownEnd::SyncFailure)) {
                    break;
                }
            }

            let events = events.lock().unwrap().clone();
            assert_eq!(events.len(), 2);
            assert_eq!(
                events[0],
                SyncEvent::SyncFailed(SyncFailure {
                    is_network_conn_err: false,
                })
            );
            assert_eq!(events[1], SyncEvent::CooldownEnd(CooldownEnd::SyncFailure));

            handle.await.unwrap();
        }

        #[tokio::test(start_paused = true)]
        async fn deployment_wait_event() {
            let f = Fixture::new_with_backoff(
                "subscribe_deployment_wait",
                // backoff.base_secs = 1 → success_wait = 1s
                // deployment_wait ~5s is scheduled separately
                cooldown::Backoff {
                    base_secs: 1,
                    growth_factor: 2,
                    max_secs: 12 * 60 * 60,
                },
            )
            .await;

            // Pre-seed deployment with future cooldown (5s)
            let seeded = miru_agent::models::deployment::Deployment {
                id: "dpl_1".to_string(),
                activity_status: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                target_status: DplTarget::Deployed,
                config_instance_ids: vec!["cfg_inst_1".to_string()],
                attempts: 1,
                cooldown_ends_at: Utc::now() + TimeDelta::seconds(5),
                ..Default::default()
            };
            f.storage
                .deployments
                .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            // Pre-cache content so content pull doesn't fail
            f.storage
                .cfg_insts
                .content
                .write(
                    "cfg_inst_1".to_string(),
                    "{}".to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            // Backend returns matching deployment with expanded CIs
            let backend_dep = backend_api::models::Deployment {
                id: "dpl_1".to_string(),
                config_instances: Some(vec![backend_api::models::ConfigInstance {
                    id: "cfg_inst_1".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            };
            let backend_dep_cloned = backend_dep.clone();
            f.http_client
                .set_list_all_deployments(move || Ok(vec![backend_dep_cloned.clone()]));

            let mut subscriber = f.syncer.subscribe().await.unwrap();

            f.syncer.sync().await.unwrap();

            // The syncer's own cooldown uses only the success_wait (1s),
            // independent of the deployment's cooldown (5s). Verify the
            // syncer's cooldown is short (success_wait = 1s, not 5s).
            let state = f.syncer.get_sync_state().await.unwrap();
            let syncer_cooldown = state
                .cooldown_ends_at
                .signed_duration_since(state.last_synced_at);
            assert!(
                syncer_cooldown <= TimeDelta::seconds(2),
                "syncer cooldown should be ~1s (success_wait), got {:?}",
                syncer_cooldown
            );

            // The DeploymentWait notification is scheduled separately and
            // fires after the deployment's cooldown (~5s + 1s grace).
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    subscriber.changed().await.unwrap();
                    let event = subscriber.borrow().clone();
                    if matches!(event, SyncEvent::CooldownEnd(CooldownEnd::DeploymentWait)) {
                        break;
                    }
                }
            })
            .await
            .expect("timed out waiting for CooldownEnd::DeploymentWait");
        }

        #[tokio::test(start_paused = true)]
        async fn success_cooldown_over_deployment_wait() {
            let f = Fixture::new_with_backoff(
                "subscribe_success_over_dpl",
                // backoff.base_secs = 10 → success_wait = 10s > deployment_wait ~5s
                // Both SyncSuccess and DeploymentWait cooldown end notifications
                // fire independently. DeploymentWait fires first (~6s), then
                // SyncSuccess (~11s).
                cooldown::Backoff {
                    base_secs: 10,
                    growth_factor: 2,
                    max_secs: 12 * 60 * 60,
                },
            )
            .await;

            // Pre-seed deployment with short cooldown (5s < base_secs)
            let seeded = miru_agent::models::deployment::Deployment {
                id: "dpl_1".to_string(),
                activity_status: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                target_status: DplTarget::Deployed,
                config_instance_ids: vec!["cfg_inst_1".to_string()],
                attempts: 1,
                cooldown_ends_at: Utc::now() + TimeDelta::seconds(5),
                ..Default::default()
            };
            f.storage
                .deployments
                .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            // Pre-cache content so content pull doesn't fail
            f.storage
                .cfg_insts
                .content
                .write(
                    "cfg_inst_1".to_string(),
                    "{}".to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            // Backend returns matching deployment with expanded CIs
            let backend_dep = backend_api::models::Deployment {
                id: "dpl_1".to_string(),
                config_instances: Some(vec![backend_api::models::ConfigInstance {
                    id: "cfg_inst_1".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            };
            let backend_dep_cloned = backend_dep.clone();
            f.http_client
                .set_list_all_deployments(move || Ok(vec![backend_dep_cloned.clone()]));

            let mut subscriber = f.syncer.subscribe().await.unwrap();

            f.syncer.sync().await.unwrap();

            // Both notifications fire: DeploymentWait (~5+1=6s) and
            // SyncSuccess (~10+1=11s).
            let mut saw_deployment_wait = false;
            let mut saw_sync_success = false;
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    subscriber.changed().await.unwrap();
                    let event = subscriber.borrow().clone();
                    match event {
                        SyncEvent::CooldownEnd(CooldownEnd::DeploymentWait) => {
                            saw_deployment_wait = true;
                        }
                        SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess) => {
                            saw_sync_success = true;
                        }
                        _ => {}
                    }
                    if saw_deployment_wait && saw_sync_success {
                        break;
                    }
                }
            })
            .await
            .expect("timed out waiting for both cooldown events");

            assert!(saw_deployment_wait);
            assert!(saw_sync_success);
        }
    }
}
