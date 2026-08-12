// standard crates
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

// internal crates
use miru_agent::app::state::AppState;
use miru_agent::authn::{Token, TokenManagerExt};
use miru_agent::deploy::fsm;
use miru_agent::disk::{Capacities, DiskErr, Layout};
use miru_agent::filesys::{dirs, files, Dir, FileSysErr, PathExt, WriteOptions};
use miru_agent::http;
use miru_agent::logs;
use miru_agent::models::{self, Device, DeviceStatus};
use miru_agent::scan::ScannerExt;
use miru_agent::server::ServerErr;
use miru_agent::sync::SyncerExt;
use miru_agent::upload::UploaderExt;

// external crates
use chrono::Utc;
use tokio::time::Duration;

// Outer wall-clock net around join handles in each test. Purely hang
// protection -- its value is NOT part of the verified behavior. It
// must absorb coverage-instrumented, loaded-machine runs, so keep it
// generous; on success it never elapses and costs nothing.
const HANG_GUARD: Duration = Duration::from_secs(60);

type ShutdownHandle = Pin<Box<dyn Future<Output = ()> + Send>>;

struct TestEnv {
    _dir: dirs::TempDir,
    layout: Layout,
}

impl TestEnv {
    fn new() -> Self {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        Self { _dir: dir, layout }
    }

    async fn valid() -> Self {
        let env = Self::new();
        env.write_keys().await;
        env.write_device(&Device::default()).await;
        env
    }

    async fn write_private_key(&self) {
        files::write_string(
            &self.layout.auth().private_key(),
            "test",
            WriteOptions::default(),
        )
        .await
        .unwrap();
    }

    async fn write_public_key(&self) {
        files::write_string(
            &self.layout.auth().public_key(),
            "test",
            WriteOptions::default(),
        )
        .await
        .unwrap();
    }

    async fn write_keys(&self) {
        self.write_private_key().await;
        self.write_public_key().await;
    }

    async fn write_device(&self, device: &Device) {
        files::write_json(
            &self.layout.device(),
            device,
            WriteOptions::OVERWRITE_NONATOMIC,
        )
        .await
        .unwrap();
    }

    async fn init(&self) -> Result<(AppState, ShutdownHandle), ServerErr> {
        let (state, handle) = AppState::init(
            &self.layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await?;
        Ok((state, Box::pin(handle)))
    }
}

pub mod init {
    use super::*;

    #[tokio::test]
    async fn fail_missing_private_key_file() {
        let env = TestEnv::new();
        let result = env.init().await;
        match result {
            Err(ServerErr::FileSysErr(e)) => {
                assert!(matches!(e, FileSysErr::PathDoesNotExistErr(_)));
            }
            Err(e) => {
                panic!("Expected FileSysErr not {e:?}");
            }
            Ok(_) => {
                panic!("expected error from initializing server state");
            }
        }
    }

    #[tokio::test]
    async fn fail_missing_public_key_file() {
        let env = TestEnv::new();
        env.write_private_key().await;
        let result = env.init().await;
        match result {
            Err(ServerErr::FileSysErr(e)) => {
                assert!(matches!(e, FileSysErr::PathDoesNotExistErr(_)));
            }
            Err(e) => {
                panic!("Expected FileSysErr not {e:?}");
            }
            Ok(_) => {
                panic!("expected error from initializing server state");
            }
        }
    }

    #[tokio::test]
    async fn fail_missing_device_id() {
        let env = TestEnv::new();
        env.write_keys().await;
        let result = env.init().await;
        assert!(matches!(
            result,
            Err(ServerErr::DiskErr(DiskErr::ResolveDeviceIDErr(_)))
        ));
    }

    #[tokio::test]
    async fn success_missing_device_file_but_valid_token() {
        let begin_test = Utc::now().timestamp();
        let env = TestEnv::new();
        env.write_keys().await;

        // create the token file with a token containing a device id
        let token_file = env.layout.auth().token();
        let token = Token {
                token: "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpYXQiOjE3NDU2MzgzMTUsInN1YiI6ImNsaV8xMjMiLCJpc3MiOiJtaXJ1IiwiYXVkIjoiY2xpZW50IiwiZXhwIjoxNzIxNTE3MDM0fQ.4ARFzYZSF_i9PjPZRJtH7HcmE_vv5tuZIpKkniua6BY".to_string(),
                expires_at: Utc::now(),
            };
        files::write_json(&token_file, &token, WriteOptions::default())
            .await
            .unwrap();

        let (state, _) = env.init().await.unwrap();

        // check last activity
        assert!(state.activity_tracker.last_touched() <= Utc::now().timestamp() as u64);
        assert!(state.activity_tracker.last_touched() >= begin_test as u64);

        // the device file should now exist with some reasonable defaults
        let device_file = env.layout.device();
        let expected_device = Device {
            id: "cli_123".to_string(),
            activated: true,
            status: DeviceStatus::Offline,
            ..Device::default()
        };
        let device = files::read_json::<Device>(&device_file).await.unwrap();
        assert_eq!(device, expected_device);
    }

    #[tokio::test]
    async fn success_missing_token_file() {
        let begin_test = Utc::now().timestamp();
        let env = TestEnv::valid().await;

        let (state, _) = env.init().await.unwrap();

        // the token file should now have the default token
        let token_file = env.layout.auth().token();
        let token = files::read_json::<Token>(&token_file).await.unwrap();
        assert_eq!(token.token, Token::default().token);

        // check last activity
        assert!(state.activity_tracker.last_touched() <= Utc::now().timestamp() as u64);
        assert!(state.activity_tracker.last_touched() >= begin_test as u64);
    }

    #[tokio::test]
    async fn success_set_device_to_offline_on_boot() {
        let env = TestEnv::new();
        env.write_keys().await;
        let device = Device {
            id: "dvc_123".to_string(),
            activated: true,
            status: DeviceStatus::Online,
            ..Device::default()
        };
        env.write_device(&device).await;

        let _ = env.init().await.unwrap();

        // the device file should now have the device set to offline
        let device_file = env.layout.device();
        let device = files::read_json::<Device>(&device_file).await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn scanner_spawned() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init().await.unwrap();

        // the scanner actor is spawned and its snapshot file is seeded on disk
        state.scanner.get_rules().await.unwrap();
        assert!(env.layout.scanner_snapshot().exists());

        // clean up the spawned actors so they don't leak
        state.shutdown().await.unwrap();
        state_handle.await;
    }

    #[tokio::test]
    async fn scanner_degrades_when_snapshot_path_unwritable() {
        let env = TestEnv::valid().await;
        // a directory at the snapshot FILE path makes
        // ScanSnapshotFile::new_with_default fail on both read and create
        dirs::create(&Dir::new(env.layout.scanner_snapshot().path().clone()))
            .await
            .unwrap();

        let (state, state_handle) = env.init().await.unwrap();

        // fail-open: the agent boots and the scanner runs without persistence
        state.scanner.get_rules().await.unwrap();

        state.shutdown().await.unwrap();
        state_handle.await;

        // nothing replaced the blocking directory with a snapshot file
        assert!(env.layout.scanner_snapshot().path().is_dir());
    }

    #[tokio::test]
    async fn uploader_spawned() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init().await.unwrap();

        // the uploader is always spawned alongside the scanner that feeds it
        state.uploader.len().await.unwrap();

        state.shutdown().await.unwrap();
        state_handle.await;
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn success_device_offline() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init().await.unwrap();
        state.shutdown().await.unwrap();
        state_handle.await;
    }

    #[tokio::test]
    async fn success_device_online() {
        let _ = logs::init(logs::Options {
            stdout: true,
            log_level: logs::LogLevel::Info,
            log_dir: PathBuf::from("/tmp/miru"),
        });

        let env = TestEnv::valid().await;

        let before_shutdown = Utc::now();
        let (state, state_handle) = env.init().await.unwrap();

        // set the device to be online
        state
            .storage
            .device
            .patch(models::device::Updates::connected())
            .await
            .unwrap();

        state.shutdown().await.unwrap();
        state_handle.await;

        // the device file should now have the device set to offline
        let device_file = env.layout.device();
        let device = files::read_json::<Device>(&device_file).await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
        assert!(device.last_disconnected_at >= before_shutdown);
        assert!(device.last_disconnected_at <= Utc::now());
    }

    #[tokio::test]
    async fn scanner_shutdown_error_does_not_abort_teardown() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // set the device online so the offline assertion below proves the
        // storage shutdown actually ran
        state
            .storage
            .device
            .patch(models::device::Updates::connected())
            .await
            .unwrap();

        // stop the scanner actor directly so AppState::shutdown errors
        state.scanner.shutdown().await.unwrap();

        let err = state.shutdown().await.expect_err("scanner error surfaces");
        assert!(matches!(err, ServerErr::ScanErr(_)));
        state_handle.await;

        let device = files::read_json::<Device>(&env.layout.device())
            .await
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn uploader_shutdown_error_does_not_abort_teardown() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // set the device online so the offline assertion below proves teardown
        // continued through storage after the uploader error
        state
            .storage
            .device
            .patch(models::device::Updates::connected())
            .await
            .unwrap();

        // stop the uploader actor directly so AppState::shutdown errors on it
        // (the uploader is torn down first, so its error is the one surfaced)
        state.uploader.shutdown().await.unwrap();

        let err = state.shutdown().await.expect_err("uploader error surfaces");
        assert!(matches!(err, ServerErr::UploadErr(_)));
        state_handle.await;

        let device = files::read_json::<Device>(&env.layout.device())
            .await
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn syncer_shutdown_error_does_not_abort_teardown() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // set the device online so the offline assertion below proves the
        // storage shutdown actually ran
        state
            .storage
            .device
            .patch(models::device::Updates::connected())
            .await
            .unwrap();

        // stop the syncer actor directly so AppState::shutdown errors
        state.syncer.shutdown().await.unwrap();

        let err = state.shutdown().await.expect_err("syncer error surfaces");
        assert!(matches!(err, ServerErr::SyncErr(_)));
        state_handle.await;

        let device = files::read_json::<Device>(&env.layout.device())
            .await
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn event_hub_shutdown_error_does_not_abort_teardown() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // set the device online so the offline assertion below proves the
        // storage shutdown actually ran
        state
            .storage
            .device
            .patch(models::device::Updates::connected())
            .await
            .unwrap();

        // stop the event hub actor directly so AppState::shutdown errors
        state.event_hub.shutdown().await.unwrap();

        let err = state
            .shutdown()
            .await
            .expect_err("event hub error surfaces");
        assert!(matches!(err, ServerErr::EventsErr(_)));
        state_handle.await;

        let device = files::read_json::<Device>(&env.layout.device())
            .await
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn storage_shutdown_error_does_not_abort_teardown() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // shut storage down directly so AppState::shutdown errors
        state.storage.shutdown().await.unwrap();

        let err = state.shutdown().await.expect_err("storage error surfaces");
        assert!(matches!(err, ServerErr::DiskErr(_)));
        state_handle.await;
    }

    #[tokio::test]
    async fn token_mngr_shutdown_error_is_surfaced() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // set the device online so the offline assertion below proves the
        // storage shutdown actually ran
        state
            .storage
            .device
            .patch(models::device::Updates::connected())
            .await
            .unwrap();

        // stop the token manager actor directly so AppState::shutdown errors
        state.token_mngr.shutdown().await.unwrap();

        let err = state
            .shutdown()
            .await
            .expect_err("token manager error surfaces");
        assert!(matches!(err, ServerErr::AuthnErr(_)));
        state_handle.await;

        let device = files::read_json::<Device>(&env.layout.device())
            .await
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn success_with_scanner_enabled() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init().await.unwrap();

        // shutdown completing (and the bundled shutdown handle resolving)
        // proves the scanner actor shut down and its JoinHandle joined; a hung
        // bundle would hang the test and trip the suite-level timeout
        state.shutdown().await.unwrap();
        state_handle.await;
    }

    #[tokio::test]
    async fn returns_first_error_with_multiple_failures() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // pre-close the syncer (fails first) and token manager (fails last)
        state.syncer.shutdown().await.unwrap();
        state.token_mngr.shutdown().await.unwrap();

        // the FIRST error (the syncer's) is returned, not the token manager's
        let err = state.shutdown().await.unwrap_err();
        assert!(matches!(err, ServerErr::SyncErr(_)));

        tokio::time::timeout(HANG_GUARD, state_handle)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn continues_past_storage_substore_failure() {
        let env = TestEnv::valid().await;
        let (state, state_handle) = env.init().await.unwrap();

        // pre-close one storage substore so the storage step fails partway
        // through Storage::shutdown
        state.storage.git_commits.shutdown().await.unwrap();

        // shutdown reports the storage failure...
        let err = state.shutdown().await.unwrap_err();
        assert!(matches!(err, ServerErr::DiskErr(_)));

        // ...but the remaining stores and the token manager were still shut
        // down, so the state handle completes (pre-fix: times out)
        tokio::time::timeout(HANG_GUARD, state_handle)
            .await
            .unwrap();
    }
}
