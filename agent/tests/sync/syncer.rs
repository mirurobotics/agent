// standard crates
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::sync::helpers::*;
use miru_agent::authn::token_mngr::TokenFile;
use miru_agent::authn::{Token, TokenManager, TokenManagerExt};
use miru_agent::cooldown;
use miru_agent::deploy::{apply, fsm};
use miru_agent::errors::*;
use miru_agent::events::hub::{EventHub, SpawnOptions};
use miru_agent::filesys::{self, Overwrite, WriteOptions};
use miru_agent::http;
use miru_agent::http::errors::{HTTPErr, MockErr};
use miru_agent::models::{Device, DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage::{
    self, CfgInstContent, CfgInstStor, CfgInsts, Deployments, GitCommits, Releases, Storage,
};
use miru_agent::sync::syncer::{
    CooldownEnd, SingleThreadSyncer, State, SyncEvent, SyncFailure, SyncerArgs, Worker,
};
use miru_agent::sync::{SyncErr, Syncer, SyncerExt};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub async fn create_token_manager(
    dir: &filesys::Dir,
    http_client: Arc<MockClient>,
) -> (TokenManager, JoinHandle<()>) {
    let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
        .await
        .unwrap();
    let private_key_file = dir.file("private_key.pem");
    private_key_file
        .write_string("private_key", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    TokenManager::spawn(
        32,
        "device_id".to_string(),
        http_client.clone(),
        token_file,
        private_key_file,
    )
    .unwrap()
}

pub async fn create_storage(dir: &filesys::Dir) -> Storage {
    let (cfg_inst_stor, _) = CfgInsts::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
        .await
        .unwrap();
    let (cfg_inst_content_stor, _) =
        CfgInstContent::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
            .await
            .unwrap();
    let (deployment_stor, _) = Deployments::spawn(16, dir.file("deployment_cache.json"), 1000)
        .await
        .unwrap();
    let (device_stor, _) =
        storage::Device::spawn_with_default(64, dir.file("device.json"), Device::default())
            .await
            .unwrap();
    let (release_stor, _) = Releases::spawn(16, dir.file("releases_cache.json"), 1000)
        .await
        .unwrap();
    let (git_commit_stor, _) = GitCommits::spawn(16, dir.file("git_commits_cache.json"), 1000)
        .await
        .unwrap();

    Storage {
        device: Arc::new(device_stor),
        cfg_insts: CfgInstStor {
            meta: Arc::new(cfg_inst_stor),
            content: Arc::new(cfg_inst_content_stor),
        },
        deployments: Arc::new(deployment_stor),
        releases: Arc::new(release_stor),
        git_commits: Arc::new(git_commit_stor),
    }
}

pub fn spawn(
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
    _dir: filesys::Dir,
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
        Self::new_with_opts(name, backoff, Device::default().agent_version).await
    }

    async fn new_with_opts(name: &str, backoff: cooldown::Backoff, agent_version: String) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let auth_client = Arc::new(MockClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;
        let token_mngr = Arc::new(token_mngr);
        let http_client = Arc::new(MockClient::default());
        let storage = Arc::new(create_storage(&dir).await);

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
                agent_version,
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
        #[cfg(feature = "test")]
        let state = self.syncer.get_sync_state().await.unwrap();
        #[cfg(feature = "test")]
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
        let dir = filesys::Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        let storage = Arc::new(create_storage(&dir).await);

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
                agent_version: Device::default().agent_version,
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

        #[cfg(feature = "test")]
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

        #[cfg(feature = "test")]
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
    async fn agent_version() {
        let new_agent_version = "v1.0.1".to_string();
        let f = Fixture::new_with_opts(
            "sync_agent_version",
            cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 12 * 60 * 60,
            },
            new_agent_version.clone(),
        )
        .await;

        let before = Utc::now();
        f.syncer.sync().await.unwrap();
        let after = Utc::now();

        // check the device file has the correct version
        let device = f.storage.device.read().await.unwrap();
        assert_eq!(device.agent_version, new_agent_version);

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
        #[cfg(feature = "test")]
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

    #[tokio::test]
    async fn agent_version_push_failure_does_not_block_sync() {
        let f = Fixture::new_with_opts(
            "sync_agent_ver_push_fail",
            cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 12 * 60 * 60,
            },
            "v99.0.0".to_string(), // different from default to trigger push
        )
        .await;

        // Configure update_device to fail with a non-network error
        f.http_client.set_update_device(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        // sync should succeed despite agent version push failure
        f.syncer.sync().await.unwrap();

        // update_device was called (and failed), but deployment sync continued
        assert_eq!(f.http_client.call_count(Call::UpdateDevice), 1);
        assert_eq!(
            f.http_client.call_count(Call::ListDeployments),
            1,
            "deployment sync should proceed despite agent version push failure"
        );

        // err_streak should be 0 (sync succeeded overall)
        let state = f.syncer.get_sync_state().await.unwrap();
        assert_eq!(state.err_streak, 0);
    }

    #[tokio::test]
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
            #[cfg(feature = "test")]
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
            #[cfg(feature = "test")]
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
            #[cfg(feature = "test")]
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
            #[cfg(feature = "test")]
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
            #[cfg(feature = "test")]
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

        #[cfg(feature = "test")]
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
