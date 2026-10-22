// standard crates
use std::future;
use std::sync::Arc;
use std::time::{Duration, Instant};

// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::cooldown;
use crate::deploy::apply;
use crate::disk;
use crate::events;
use crate::filesys::File;
use crate::http;
use crate::sync::{errors::*, syncer_impl::SyncerImpl};
use crate::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{error, info};

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

/// Wait for the next sync event. A missing subscription remains pending so
/// subscribers can disable this branch without preventing their other work.
pub(crate) async fn next_sync_event(
    subscriber: &mut Option<watch::Receiver<SyncEvent>>,
) -> Option<SyncEvent> {
    let Some(subscriber) = subscriber.as_mut() else {
        return future::pending().await;
    };

    if subscriber.changed().await.is_err() {
        return None;
    }

    let event = subscriber.borrow_and_update().clone();
    Some(event)
}

// ================================ ARGS AND STATE ================================= //
pub struct SyncerArgs<HTTPClientT, TokenManagerT: TokenManagerExt> {
    pub storage: Arc<disk::Storage>,
    pub http_client: Arc<HTTPClientT>,
    pub token_mngr: Arc<TokenManagerT>,
    pub deploy_opts: apply::DeployOpts,
    pub backoff: cooldown::Backoff,
    pub event_hub: events::EventHub,
    pub system_metadata_cache: File,
}

/// The syncer's state. Decisions use the monotonic instants, which a system
/// clock step cannot move; the `DateTime` fields are for display only. None of
/// it is saved, so a restart starts with no cooldown.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// System clock time of the latest sync attempt, for display.
    pub last_attempted_sync_at: DateTime<Utc>,
    /// System clock time of the latest successful sync, for display.
    pub last_synced_at: DateTime<Utc>,
    /// Monotonic instant of the latest sync attempt in this process.
    pub last_attempted_sync: Option<Instant>,
    /// Monotonic instant the current cooldown ends, if one was started.
    pub cooldown_ends: Option<Instant>,
    pub err_streak: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            last_attempted_sync: None,
            cooldown_ends: None,
            err_streak: 0,
        }
    }
}

impl State {
    /// How much of the cooldown is left; zero when not in cooldown.
    pub fn remaining_cooldown(&self) -> Duration {
        self.cooldown_ends.map_or(Duration::ZERO, |ends| {
            ends.saturating_duration_since(Instant::now())
        })
    }

    pub fn is_in_cooldown(&self) -> bool {
        !self.remaining_cooldown().is_zero()
    }

    /// When the cooldown ends by the system clock, for display: now plus the
    /// remaining cooldown.
    pub fn cooldown_ends_at(&self) -> DateTime<Utc> {
        let remaining = TimeDelta::from_std(self.remaining_cooldown()).unwrap_or(TimeDelta::MAX);
        Utc::now()
            .checked_add_signed(remaining)
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    /// Monotonic time since the latest sync attempt, or `None` before the
    /// first attempt in this process.
    pub fn since_last_attempt(&self) -> Option<Duration> {
        self.last_attempted_sync.map(|at| at.elapsed())
    }
}

// ================================ WORKER AND HANDLE ============================== //
#[allow(async_fn_in_trait)]
pub trait SyncerExt {
    async fn shutdown(&self) -> Result<(), SyncErr>;
    async fn get_sync_state(&self) -> Result<State, SyncErr>;
    async fn is_in_cooldown(&self) -> Result<bool, SyncErr>;
    async fn get_remaining_cooldown(&self) -> Result<Duration, SyncErr>;
    async fn get_since_last_attempt(&self) -> Result<Option<Duration>, SyncErr>;
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
    syncer: SyncerImpl<HTTPClientT>,
    receiver: mpsc::Receiver<Command>,
}

impl<HTTPClientT: Send> Worker<HTTPClientT> {
    pub fn new(syncer: SyncerImpl<HTTPClientT>, receiver: mpsc::Receiver<Command>) -> Self {
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
            syncer: SyncerImpl::new(args),
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

    async fn get_remaining_cooldown(&self) -> Result<Duration, SyncErr> {
        let state = self.get_sync_state().await?;
        Ok(state.remaining_cooldown())
    }

    async fn get_since_last_attempt(&self) -> Result<Option<Duration>, SyncErr> {
        let state = self.get_sync_state().await?;
        Ok(state.since_last_attempt())
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
    use std::time::{Duration, Instant};

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
    use miru_agent::filesys::{File, Overwrite};
    use miru_agent::http;
    use miru_agent::http::errors::{HTTPErr, MockErr};
    use miru_agent::models::{DplActivity, DplErrStatus, DplTarget};
    use miru_agent::sync::syncer::{
        CooldownEnd, State, SyncEvent, SyncFailure, SyncerArgs, Worker,
    };
    use miru_agent::sync::syncer_impl::SyncerImpl;
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
        let worker = Worker::new(SyncerImpl::new(args), receiver);
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
        system_metadata_cache: File,
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
            let system_metadata_cache = dir.file("system_metadata.json");

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
                    system_metadata_cache: system_metadata_cache.clone(),
                },
            )
            .unwrap();

            Self {
                _dir: dir,
                http_client,
                storage,
                syncer,
                backoff,
                system_metadata_cache,
                token_mngr,
            }
        }

        /// Reset cooldown so the next sync() won't be rejected.
        async fn reset_cooldown(&self) {
            let state = self.syncer.get_sync_state().await.unwrap();
            self.syncer
                .set_sync_state(State {
                    cooldown_ends: None,
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
                last_attempted_sync: None,
                cooldown_ends: None,
                err_streak: 0,
            };
            assert_eq!(state, expected);
            assert!(!state.is_in_cooldown());
            assert_eq!(state.remaining_cooldown(), Duration::ZERO);
            assert_eq!(state.since_last_attempt(), None);
        }

        #[tokio::test]
        async fn is_in_cooldown() {
            // in cooldown (cooldown_ends is in the future)
            let state = State {
                cooldown_ends: Some(Instant::now() + Duration::from_secs(10)),
                ..State::default()
            };
            assert!(state.is_in_cooldown());
            let remaining = state.remaining_cooldown();
            assert!(remaining <= Duration::from_secs(10) && remaining > Duration::from_secs(9));

            // not in cooldown (cooldown_ends has passed)
            let state = State {
                cooldown_ends: Some(Instant::now()),
                ..State::default()
            };
            assert!(!state.is_in_cooldown());
            assert_eq!(state.remaining_cooldown(), Duration::ZERO);
        }

        #[test]
        fn cooldown_ends_at_is_now_plus_the_remaining_cooldown() {
            let state = State {
                cooldown_ends: Some(Instant::now() + Duration::from_secs(60)),
                ..State::default()
            };
            let window = StateAssert::begin();
            let ends_at = state.cooldown_ends_at();
            let window = window.end();
            assert!(ends_at <= window.after + TimeDelta::seconds(60));
            assert!(ends_at >= window.before + TimeDelta::seconds(59));

            // not in cooldown: it ends now
            let state = State::default();
            let before = Utc::now();
            let ends_at = state.cooldown_ends_at();
            assert!(ends_at >= before && ends_at <= Utc::now());
        }

        #[test]
        fn since_last_attempt_is_monotonic_time_since_the_attempt() {
            let state = State {
                last_attempted_sync: Some(Instant::now() - Duration::from_secs(30)),
                ..State::default()
            };
            let since = state.since_last_attempt().unwrap();
            assert!(since >= Duration::from_secs(30) && since < Duration::from_secs(31));
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
                    system_metadata_cache: dir.file("system_metadata.json"),
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
                    cooldown_ends: Some(Instant::now() + Duration::from_secs(60)),
                    ..State::default()
                })
                .await
                .unwrap();

            assert!(f.syncer.is_in_cooldown().await.unwrap());
        }

        #[tokio::test]
        async fn false_when_not_in_cooldown() {
            let f = Fixture::new("syncer_ext_cooldown_false").await;
            // Default state has no cooldown_ends → not in cooldown
            assert!(!f.syncer.is_in_cooldown().await.unwrap());
        }
    }

    pub mod get_remaining_cooldown {
        use super::*;

        #[tokio::test]
        async fn get_remaining_cooldown() {
            let f = Fixture::new("syncer_ext_cooldown_ends").await;
            assert_eq!(
                f.syncer.get_remaining_cooldown().await.unwrap(),
                Duration::ZERO
            );

            f.syncer
                .set_sync_state(State {
                    cooldown_ends: Some(Instant::now() + Duration::from_secs(120)),
                    ..State::default()
                })
                .await
                .unwrap();

            let remaining = f.syncer.get_remaining_cooldown().await.unwrap();
            assert!(remaining <= Duration::from_secs(120));
            assert!(remaining > Duration::from_secs(119));
        }
    }

    pub mod get_since_last_attempt {
        use super::*;

        #[tokio::test]
        async fn get_since_last_attempt() {
            let f = Fixture::new("syncer_ext_last_attempted").await;
            assert_eq!(f.syncer.get_since_last_attempt().await.unwrap(), None);

            let before = Instant::now();
            f.syncer.sync().await.unwrap();

            let since = f.syncer.get_since_last_attempt().await.unwrap().unwrap();
            assert!(since <= before.elapsed());
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

            let window = StateAssert::begin();
            f.syncer.sync().await.unwrap();
            let window = window.end();

            assert_deployment_stored(&f.storage.deployments, "dpl_1").await;
            assert_cfg_inst_stored(&f.storage.cfg_insts.meta, "cfg_inst_1").await;
            let _content = read_content(&f.storage.cfg_insts.content, "cfg_inst_1").await;

            // check the sync state
            let state = f.syncer.get_sync_state().await.unwrap();
            assert!(!f.syncer.get_remaining_cooldown().await.unwrap().is_zero());
            let base_cooldown = Duration::from_secs(f.backoff.base_secs as u64);
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
            let window = StateAssert::begin();
            f.syncer.sync().await.unwrap();
            let window = window.end();

            let state = f.syncer.get_sync_state().await.unwrap();
            let base_cooldown = Duration::from_secs(f.backoff.base_secs as u64);
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
                    cooldown_ends: Some(Instant::now() + Duration::from_secs(10)),
                    ..State::default()
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

            let base_cooldown_duration = Duration::from_secs(f.backoff.base_secs as u64);
            for _ in 0..10 {
                let window = StateAssert::begin();
                let error = f.syncer.sync().await.unwrap_err();
                let window = window.end();

                // check error type
                assert!(error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert!(!f.syncer.get_remaining_cooldown().await.unwrap().is_zero());
                window.assert_failed(&state, base_cooldown_duration, 0);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends: None,
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
                let window = StateAssert::begin();
                let error = f.syncer.sync().await.unwrap_err();
                let window = window.end();

                // check error type
                assert!(!error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert!(!f.syncer.get_remaining_cooldown().await.unwrap().is_zero());
                let cooldown_secs = cooldown::calc(&f.backoff, i + 1);
                let cooldown_duration = Duration::from_secs(cooldown_secs as u64);
                window.assert_failed(&state, cooldown_duration, i + 1);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends: None,
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
                let window = StateAssert::begin();
                let error = f.syncer.sync().await.unwrap_err();
                let window = window.end();

                // check error type
                assert!(!error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert!(!f.syncer.get_remaining_cooldown().await.unwrap().is_zero());
                let cooldown_secs = cooldown::calc(&f.backoff, i + 1);
                let cooldown_duration = Duration::from_secs(cooldown_secs as u64);
                window.assert_failed(&state, cooldown_duration, i + 1);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends: None,
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
            let base_cooldown_duration = Duration::from_secs(f.backoff.base_secs as u64);
            for _ in 0..10 {
                let window = StateAssert::begin();
                let error = f.syncer.sync().await.unwrap_err();
                let window = window.end();

                // check error type
                assert!(error.is_network_conn_err());

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                window.assert_failed(&state, base_cooldown_duration, cur_err_streak);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends: None,
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
            let base_cooldown_duration = Duration::from_secs(f.backoff.base_secs as u64);
            for _ in 0..10 {
                let window = StateAssert::begin();
                f.syncer.sync().await.unwrap();
                let window = window.end();

                // check the sync state
                let state = f.syncer.get_sync_state().await.unwrap();
                assert!(!f.syncer.get_remaining_cooldown().await.unwrap().is_zero());
                window.assert_success(&state, base_cooldown_duration, 0);

                // double check sync state functions
                assert!(f.syncer.is_in_cooldown().await.unwrap());

                // reset the syncer state
                f.syncer
                    .set_sync_state(State {
                        cooldown_ends: None,
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

            let window = StateAssert::begin();
            let error = f.syncer.sync().await.unwrap_err();
            let window = window.end();
            assert!(error.is_network_conn_err());

            // err_streak should be unchanged (not incremented, not reset)
            let state = f.syncer.get_sync_state().await.unwrap();
            let base_cooldown = Duration::from_secs(f.backoff.base_secs as u64);
            window.assert_failed(&state, base_cooldown, 3);
        }
    }

    pub mod system_metadata {
        use super::*;
        use miru_agent::disk::system_metadata as cache;
        use miru_agent::models;

        fn http_err(is_network_conn_err: bool) -> HTTPErr {
            HTTPErr::MockErr(MockErr {
                is_network_conn_err,
            })
        }

        #[tokio::test]
        async fn reported_after_successful_sync() {
            let f = Fixture::new("sync_system_metadata_reported").await;

            f.syncer.sync().await.unwrap();

            assert_eq!(f.http_client.call_count(Call::UpdateDevice), 1);
            let cached = cache::read(&f.system_metadata_cache).await.unwrap();
            assert_eq!(cached, Some(models::system_metadata()));
        }

        #[tokio::test]
        async fn not_reported_again_when_unchanged() {
            let f = Fixture::new("sync_system_metadata_unchanged").await;

            f.syncer.sync().await.unwrap();
            f.reset_cooldown().await;
            f.syncer.sync().await.unwrap();

            assert_eq!(f.http_client.call_count(Call::UpdateDevice), 1);
        }

        #[tokio::test]
        async fn skipped_when_backend_unreachable() {
            let f = Fixture::new("sync_system_metadata_unreachable").await;
            f.http_client
                .set_list_all_deployments(|| Err(http_err(true)));

            f.syncer.sync().await.unwrap_err();

            assert_eq!(f.http_client.call_count(Call::UpdateDevice), 0);
            let cached = cache::read(&f.system_metadata_cache).await.unwrap();
            assert_eq!(cached, None);
        }

        #[tokio::test]
        async fn reported_after_non_network_sync_failure() {
            let f = Fixture::new("sync_system_metadata_non_network_failure").await;
            f.http_client
                .set_list_all_deployments(|| Err(http_err(false)));

            f.syncer.sync().await.unwrap_err();

            assert_eq!(f.http_client.call_count(Call::UpdateDevice), 1);
            let cached = cache::read(&f.system_metadata_cache).await.unwrap();
            assert_eq!(cached, Some(models::system_metadata()));
        }

        #[tokio::test]
        async fn failure_does_not_fail_sync() {
            let f = Fixture::new("sync_system_metadata_failure").await;
            f.http_client.set_update_device(|| Err(http_err(false)));

            let window = StateAssert::begin();
            f.syncer.sync().await.unwrap();
            let window = window.end();

            let state = f.syncer.get_sync_state().await.unwrap();
            let base_cooldown = Duration::from_secs(f.backoff.base_secs as u64);
            window.assert_success(&state, base_cooldown, 0);

            assert_eq!(f.http_client.call_count(Call::UpdateDevice), 1);
            let cached = cache::read(&f.system_metadata_cache).await.unwrap();
            assert_eq!(cached, None);
        }
    }

    pub mod sync_if_not_in_cooldown {
        use super::*;

        #[tokio::test]
        async fn skips_when_in_cooldown() {
            let f = Fixture::new("sync_if_not_in_cooldown_skip").await;

            f.syncer
                .set_sync_state(State {
                    cooldown_ends: Some(Instant::now() + Duration::from_secs(10)),
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

            let window = StateAssert::begin();
            f.syncer.sync_if_not_in_cooldown().await.unwrap();
            let window = window.end();

            let state = f.syncer.get_sync_state().await.unwrap();
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
            let syncer_cooldown = state.remaining_cooldown();
            assert!(
                syncer_cooldown <= Duration::from_secs(2),
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

    mod next_sync_event {
        use super::super::*;

        #[tokio::test]
        async fn returns_next_event() {
            let (tx, rx) = watch::channel(SyncEvent::SyncSuccess);
            let mut subscriber = Some(rx);
            tx.send(SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess))
                .unwrap();

            assert!(matches!(
                next_sync_event(&mut subscriber).await,
                Some(SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess))
            ));
        }

        #[tokio::test]
        async fn returns_none_when_stream_ends() {
            let (tx, rx) = watch::channel(SyncEvent::SyncSuccess);
            let mut subscriber = Some(rx);
            drop(tx);

            assert!(next_sync_event(&mut subscriber).await.is_none());
        }

        #[tokio::test]
        async fn missing_subscription_remains_pending() {
            let mut subscriber = None;

            assert!(tokio::time::timeout(
                std::time::Duration::from_millis(10),
                next_sync_event(&mut subscriber),
            )
            .await
            .is_err());
        }
    }
}
