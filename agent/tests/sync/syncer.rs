// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::authn::{
    token::Token,
    token_mngr::{TokenFile, TokenManager},
};
use miru_agent::crud::prelude::*;
use miru_agent::deploy::fsm;
use miru_agent::errors::*;
use miru_agent::filesys::dir::Dir;
use miru_agent::http::{
    client::HTTPClient,
    errors::{HTTPErr, MockErr},
};
use miru_agent::models::device::Device;
use miru_agent::storage::{
    config_instances::{ConfigInstanceCache, ConfigInstanceContentCache},
    deployments::DeploymentCache,
    device::DeviceFile,
};
use miru_agent::sync::{
    errors::SyncErr,
    syncer::{
        CooldownEnd, SingleThreadSyncer, SyncEvent, SyncFailure, SyncState, Syncer, SyncerArgs,
        SyncerExt, Worker,
    },
};
use miru_agent::utils::{calc_exp_backoff, CooldownOptions};

use crate::authn::token_mngr::spawn as spawn_token_manager;
use crate::http::mock::{MockClient, MockDevicesClient};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub async fn create_token_manager(
    dir: &Dir,
    http_client: Arc<MockDevicesClient>,
) -> (TokenManager, JoinHandle<()>) {
    let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
        .await
        .unwrap();
    let private_key_file = dir.file("private_key.pem");
    private_key_file
        .write_string("private_key", true, true)
        .await
        .unwrap();

    spawn_token_manager(
        32,
        "device_id".to_string(),
        http_client.clone(),
        token_file,
        private_key_file,
    )
    .unwrap()
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

pub mod sync_state {
    use super::*;

    #[tokio::test]
    async fn is_in_cooldown() {
        // not in cooldown
        let state = SyncState {
            last_attempted_sync_at: Utc::now(),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now() + TimeDelta::seconds(10),
            err_streak: 0,
        };
        assert!(state.is_in_cooldown());

        // in cooldown
        let state = SyncState {
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
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let http_client = Arc::new(HTTPClient::new("doesntmatter").await);
        let (syncer, worker_handler) = Syncer::spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options: CooldownOptions::default(),
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        syncer.shutdown().await.unwrap();
        worker_handler.await.unwrap();
    }
}

pub mod subscribe {
    use super::*;

    #[tokio::test]
    async fn sync_success() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;
        let http_client = Arc::new(MockClient::default());

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 1,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        let mut subscriber = syncer.subscribe().await.unwrap();
        let events = Arc::new(Mutex::new(vec![]));

        let mut subscriber_for_spawn = subscriber.clone();
        let events_for_spawn = events.clone();
        let handle = tokio::spawn(async move {
            // expect two events: 1. not synced and then 2. cooldown ended
            for _ in 0..2 {
                subscriber_for_spawn.changed().await.unwrap();
                events_for_spawn
                    .lock()
                    .unwrap()
                    .push(subscriber_for_spawn.borrow().clone());
            }
        });

        syncer.sync().await.unwrap();
        // Wait for the cooldown to end
        loop {
            subscriber.changed().await.unwrap();
            let event = subscriber.borrow().clone();
            if matches!(event, SyncEvent::CooldownEnd(CooldownEnd::FromSyncSuccess)) {
                break;
            }
        }

        let events = events.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], SyncEvent::SyncSuccess);
        assert_eq!(
            events[1],
            SyncEvent::CooldownEnd(CooldownEnd::FromSyncSuccess)
        );

        handle.await.unwrap();
    }

    #[tokio::test]
    async fn sync_failure() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        let http_client = Arc::new(MockClient::default());
        http_client.deployments_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: true,
            })))
        });
        http_client.deployments_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: true,
            })))
        });

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 1,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        let mut subscriber = syncer.subscribe().await.unwrap();
        let events = Arc::new(Mutex::new(vec![]));

        let mut subscriber_for_spawn = subscriber.clone();
        let events_for_spawn = events.clone();
        let handle = tokio::spawn(async move {
            // expect two events: 1. not synced and then 2. cooldown ended
            for _ in 0..2 {
                subscriber_for_spawn.changed().await.unwrap();
                events_for_spawn
                    .lock()
                    .unwrap()
                    .push(subscriber_for_spawn.borrow().clone());
            }
        });

        syncer.sync().await.unwrap_err();
        // Wait for the cooldown to end
        loop {
            subscriber.changed().await.unwrap();
            let event = subscriber.borrow().clone();
            if matches!(event, SyncEvent::CooldownEnd(CooldownEnd::FromSyncFailure)) {
                break;
            }
        }

        let events = events.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            SyncEvent::SyncFailed(SyncFailure {
                is_network_connection_error: true,
            })
        );
        assert_eq!(
            events[1],
            SyncEvent::CooldownEnd(CooldownEnd::FromSyncFailure)
        );

        handle.await.unwrap();
    }
}

// get_sync_state, is_in_cooldown, get_cooldown_ends_at
// sync function tests below

pub mod sync {
    use super::*;

    #[tokio::test]
    async fn deployments() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        // define a backend deployment with an embedded config instance
        let backend_dep = openapi_client::models::Deployment {
            object: openapi_client::models::deployment::Object::Deployment,
            id: "dpl_1".to_string(),
            description: "test".to_string(),
            status: openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
            activity_status:
                openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
            error_status:
                openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
            target_status:
                openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            device_id: "dev_1".to_string(),
            release_id: "rls_1".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            release: None,
            config_instances: Some(vec![openapi_client::models::ConfigInstance {
                object: openapi_client::models::config_instance::Object::ConfigInstance,
                id: "ci_1".to_string(),
                config_type_name: "test_type".to_string(),
                filepath: "test/config.json".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                config_schema_id: "schema_1".to_string(),
                config_type_id: "ct_1".to_string(),
                config_type: None,
                content: Some(serde_json::json!({"key": "value"})),
            }]),
        };

        let http_client = Arc::new(MockClient::default());
        let backend_dep_cloned = backend_dep.clone();
        http_client
            .deployments_client
            .set_list_all_deployments(move || Ok(vec![backend_dep_cloned.clone()]));

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let cfg_inst_cache = Arc::new(cfg_inst_cache);
        let cfg_inst_content_cache = Arc::new(cfg_inst_content_cache);
        let deployment_cache = Arc::new(deployment_cache);
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: cfg_inst_cache.clone(),
                cfg_inst_content_cache: cfg_inst_content_cache.clone(),
                deployment_cache: deployment_cache.clone(),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        let before = Utc::now();
        syncer.sync().await.unwrap();
        let after = Utc::now();

        // check the deployment cache has the new deployment
        let cached_dep = deployment_cache
            .read_optional("dpl_1".to_string())
            .await
            .unwrap();
        assert!(cached_dep.is_some(), "deployment should be cached");

        // check the config instance metadata cache
        let ci = cfg_inst_cache
            .read_optional("ci_1".to_string())
            .await
            .unwrap();
        assert!(ci.is_some(), "config instance should be cached");

        // check the content cache has the config instance content
        let content = cfg_inst_content_cache
            .read_optional("ci_1".to_string())
            .await
            .unwrap();
        assert!(
            content.is_some(),
            "config instance content should be cached"
        );

        // check the sync state
        let state = syncer.get_sync_state().await.unwrap();
        assert_eq!(
            syncer.get_cooldown_ends_at().await.unwrap(),
            state.cooldown_ends_at
        );
        assert!(state.last_attempted_sync_at > before);
        assert!(state.last_attempted_sync_at < after);
        assert!(state.last_synced_at > before);
        assert!(state.last_synced_at < after);
        let base_cooldown_duration = TimeDelta::seconds(cooldown_options.base_secs);
        assert!(state.cooldown_ends_at > before + base_cooldown_duration);
        assert!(state.cooldown_ends_at < after + base_cooldown_duration);
        assert_eq!(state.err_streak, 0);
    }

    #[tokio::test]
    async fn agent_version() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        let http_client = Arc::new(MockClient::default());

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let cfg_inst_cache = Arc::new(cfg_inst_cache);
        let cfg_inst_content_cache = Arc::new(cfg_inst_content_cache);
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();
        let device_file = Arc::new(device_file);

        let new_agent_version = "v1.0.1".to_string();
        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: device_file.clone(),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: cfg_inst_cache.clone(),
                cfg_inst_content_cache: cfg_inst_content_cache.clone(),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: new_agent_version.clone(),
            },
        )
        .unwrap();

        let before = Utc::now();
        syncer.sync().await.unwrap();
        let after = Utc::now();

        // check the device file has the correct version
        let device = device_file.read().await.unwrap();
        assert_eq!(device.agent_version, new_agent_version);

        // check the sync state
        let state = syncer.get_sync_state().await.unwrap();
        assert_eq!(
            syncer.get_cooldown_ends_at().await.unwrap(),
            state.cooldown_ends_at
        );
        assert!(state.last_attempted_sync_at > before);
        assert!(state.last_attempted_sync_at < after);
        assert!(state.last_synced_at > before);
        assert!(state.last_synced_at < after);
        let base_cooldown_duration = TimeDelta::seconds(cooldown_options.base_secs);
        assert!(state.cooldown_ends_at > before + base_cooldown_duration);
        assert!(state.cooldown_ends_at < after + base_cooldown_duration);
        assert_eq!(state.err_streak, 0);
    }

    #[tokio::test]
    async fn network_error() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        let http_client = Arc::new(MockClient::default());
        http_client.deployments_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: true,
            })))
        });
        http_client.deployments_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: true,
            })))
        });

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        let base_cooldown_duration = TimeDelta::seconds(cooldown_options.base_secs);
        for _ in 0..10 {
            let before = Utc::now();
            let error = syncer.sync().await.unwrap_err();
            let after = Utc::now();

            // check error type
            assert!(error.is_network_connection_error());

            // check the sync state
            let state = syncer.get_sync_state().await.unwrap();
            assert_eq!(
                syncer.get_cooldown_ends_at().await.unwrap(),
                state.cooldown_ends_at
            );
            assert!(state.last_attempted_sync_at > before);
            assert!(state.last_attempted_sync_at < after);
            assert_eq!(state.last_synced_at, DateTime::<Utc>::UNIX_EPOCH);
            assert!(state.cooldown_ends_at > before + base_cooldown_duration);
            assert!(state.cooldown_ends_at < after + base_cooldown_duration);
            assert_eq!(state.err_streak, 0);

            // double check sync state functions
            assert!(syncer.is_in_cooldown().await.unwrap());

            // reset the syncer state
            #[cfg(feature = "test")]
            syncer
                .set_sync_state(SyncState {
                    cooldown_ends_at: before,
                    ..state
                })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn non_network_error() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        // all errors need to be a network connection error for the syncer to return a
        // network connection error so only set one false to test this
        let http_client = Arc::new(MockClient::default());
        http_client.deployments_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: false,
            })))
        });
        http_client.deployments_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: false,
            })))
        });

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        for i in 0..10 {
            let before = Utc::now();
            let error = syncer.sync().await.unwrap_err();
            let after = Utc::now();

            // check error type
            assert!(!error.is_network_connection_error());

            // check the sync state
            let state = syncer.get_sync_state().await.unwrap();
            assert_eq!(
                syncer.get_cooldown_ends_at().await.unwrap(),
                state.cooldown_ends_at
            );
            assert!(state.last_attempted_sync_at > before);
            assert!(state.last_attempted_sync_at < after);
            assert_eq!(state.last_synced_at, DateTime::<Utc>::UNIX_EPOCH);
            let cooldown_secs = calc_exp_backoff(
                cooldown_options.base_secs,
                cooldown_options.growth_factor,
                i + 1,
                cooldown_options.max_secs,
            );
            let cooldown_duration = TimeDelta::seconds(cooldown_secs);
            assert!(state.cooldown_ends_at > before + cooldown_duration);
            assert!(state.cooldown_ends_at < after + cooldown_duration);
            assert_eq!(state.err_streak, i + 1);

            // double check sync state functions
            assert!(syncer.is_in_cooldown().await.unwrap());

            // reset the syncer state
            #[cfg(feature = "test")]
            syncer
                .set_sync_state(SyncState {
                    cooldown_ends_at: before,
                    ..state
                })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn non_network_error_to_network_error_to_recovery() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;

        let http_client = Arc::new(MockClient::default());
        http_client.deployments_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: false,
            })))
        });
        http_client.deployments_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: false,
            })))
        });

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        // non-network connection errors
        for i in 0..10 {
            let before = Utc::now();
            let error = syncer.sync().await.unwrap_err();
            let after = Utc::now();

            // check error type
            assert!(!error.is_network_connection_error());

            // check the sync state
            let state = syncer.get_sync_state().await.unwrap();
            assert_eq!(
                syncer.get_cooldown_ends_at().await.unwrap(),
                state.cooldown_ends_at
            );
            assert!(state.last_attempted_sync_at > before);
            assert!(state.last_attempted_sync_at < after);
            assert_eq!(state.last_synced_at, DateTime::<Utc>::UNIX_EPOCH);
            let cooldown_secs = calc_exp_backoff(
                cooldown_options.base_secs,
                cooldown_options.growth_factor,
                i + 1,
                cooldown_options.max_secs,
            );
            let cooldown_duration = TimeDelta::seconds(cooldown_secs);
            assert!(state.cooldown_ends_at > before + cooldown_duration);
            assert!(state.cooldown_ends_at < after + cooldown_duration);
            assert_eq!(state.err_streak, i + 1);

            // double check sync state functions
            assert!(syncer.is_in_cooldown().await.unwrap());

            // reset the syncer state
            #[cfg(feature = "test")]
            syncer
                .set_sync_state(SyncState {
                    cooldown_ends_at: before,
                    ..state
                })
                .await
                .unwrap();
        }

        // set the http client to return a network connection error
        http_client.deployments_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: true,
            })))
        });
        http_client.deployments_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(Box::new(MockErr {
                is_network_connection_error: true,
            })))
        });

        // network connection errors
        let cur_err_streak = 10;
        let base_cooldown_duration = TimeDelta::seconds(cooldown_options.base_secs);
        for _ in 0..10 {
            let before = Utc::now();
            let error = syncer.sync().await.unwrap_err();
            let after = Utc::now();

            // check error type
            assert!(error.is_network_connection_error());

            // check the sync state
            let state = syncer.get_sync_state().await.unwrap();
            assert!(state.last_attempted_sync_at > before);
            assert!(state.last_attempted_sync_at < after);
            assert_eq!(state.last_synced_at, DateTime::<Utc>::UNIX_EPOCH);
            assert!(state.cooldown_ends_at > before + base_cooldown_duration);
            assert!(state.cooldown_ends_at < after + base_cooldown_duration);
            assert_eq!(state.err_streak, cur_err_streak);

            // double check sync state functions
            assert!(syncer.is_in_cooldown().await.unwrap());

            // reset the syncer state
            #[cfg(feature = "test")]
            syncer
                .set_sync_state(SyncState {
                    cooldown_ends_at: before,
                    ..state
                })
                .await
                .unwrap();
        }

        // set the http client to not return an error
        http_client
            .deployments_client
            .set_list_all_deployments(|| Ok(vec![]));
        http_client
            .deployments_client
            .set_update_deployment(|| Ok(openapi_client::models::Deployment::default()));

        // recovery
        let base_cooldown_duration = TimeDelta::seconds(cooldown_options.base_secs);
        for _ in 0..10 {
            let before = Utc::now();
            syncer.sync().await.unwrap();
            let after = Utc::now();

            // check the sync state
            let state = syncer.get_sync_state().await.unwrap();
            assert_eq!(
                syncer.get_cooldown_ends_at().await.unwrap(),
                state.cooldown_ends_at
            );
            assert!(state.last_attempted_sync_at > before);
            assert!(state.last_attempted_sync_at < after);
            assert!(state.last_synced_at > before);
            assert!(state.last_synced_at < after);
            assert!(state.cooldown_ends_at > before + base_cooldown_duration);
            assert!(state.cooldown_ends_at < after + base_cooldown_duration);
            assert_eq!(state.err_streak, 0);

            // double check sync state functions
            assert!(syncer.is_in_cooldown().await.unwrap());

            // reset the syncer state
            #[cfg(feature = "test")]
            syncer
                .set_sync_state(SyncState {
                    cooldown_ends_at: before,
                    ..state
                })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn in_cooldown_error() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;
        let http_client = Arc::new(MockClient::default());

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        // set the syncer state to be in cooldown
        #[cfg(feature = "test")]
        syncer
            .set_sync_state(SyncState {
                last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
                last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
                cooldown_ends_at: Utc::now() + TimeDelta::seconds(10),
                err_streak: 0,
            })
            .await
            .unwrap();

        let error = syncer.sync().await.unwrap_err();
        assert!(matches!(error, SyncErr::InCooldownErr(_)));
    }
}

pub mod sync_if_not_in_cooldown {
    use super::*;

    #[tokio::test]
    async fn sync_if_not_in_cooldown() {
        let dir = Dir::create_temp_dir("spawn").await.unwrap();
        let auth_client = Arc::new(MockDevicesClient::default());
        let (token_mngr, _) = create_token_manager(&dir, auth_client.clone()).await;
        let http_client = Arc::new(MockClient::default());

        // create the caches
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
                .await
                .unwrap();
        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, dir.file("device.json"), Device::default())
                .await
                .unwrap();

        let cooldown_options = CooldownOptions {
            base_secs: 10,
            ..CooldownOptions::default()
        };
        let (syncer, _) = spawn(
            32,
            SyncerArgs {
                device_id: "device_id".to_string(),
                device_file: Arc::new(device_file),
                http_client: http_client.clone(),
                token_mngr: Arc::new(token_mngr),
                cfg_inst_cache: Arc::new(cfg_inst_cache),
                cfg_inst_content_cache: Arc::new(cfg_inst_content_cache),
                deployment_cache: Arc::new(deployment_cache),
                deployment_dir: dir.subdir("deployments"),
                staging_dir: dir.subdir("staging"),
                fsm_settings: fsm::Settings::default(),
                cooldown_options,
                agent_version: Device::default().agent_version,
            },
        )
        .unwrap();

        // set the syncer state to be in cooldown
        #[cfg(feature = "test")]
        syncer
            .set_sync_state(SyncState {
                last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
                last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
                cooldown_ends_at: Utc::now() + TimeDelta::seconds(10),
                err_streak: 0,
            })
            .await
            .unwrap();

        syncer.sync_if_not_in_cooldown().await.unwrap();
        assert_eq!(
            syncer
                .get_sync_state()
                .await
                .unwrap()
                .last_attempted_sync_at,
            DateTime::<Utc>::UNIX_EPOCH
        );

        // set the syncer state to be in cooldown
        #[cfg(feature = "test")]
        syncer
            .set_sync_state(SyncState {
                last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
                last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
                cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
                err_streak: 0,
            })
            .await
            .unwrap();

        let before = Utc::now();
        syncer.sync_if_not_in_cooldown().await.unwrap();
        let after = Utc::now();
        assert!(
            syncer
                .get_sync_state()
                .await
                .unwrap()
                .last_attempted_sync_at
                > before
        );
        assert!(
            syncer
                .get_sync_state()
                .await
                .unwrap()
                .last_attempted_sync_at
                < after
        );
    }
}
