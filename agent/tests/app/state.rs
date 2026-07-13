// standard crates
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

// internal crates
use miru_agent::app::state::AppState;
use miru_agent::authn::Token;
use miru_agent::deploy::fsm;
use miru_agent::disk::{Capacities, DiskErr, Layout};
use miru_agent::filesys::{dirs, files, Dir, FileSysErr, PathExt, WriteOptions};
use miru_agent::http;
use miru_agent::logs;
use miru_agent::models::{self, Device, DeviceStatus};
use miru_agent::scan::ScannerExt;
use miru_agent::server::ServerErr;

// external crates
use chrono::Utc;

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

    async fn init(&self, enable_scanner: bool) -> Result<(AppState, ShutdownHandle), ServerErr> {
        let (state, handle) = AppState::init(
            &self.layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
            enable_scanner,
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
        let result = env.init(false).await;
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
        let result = env.init(false).await;
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
        let result = env.init(false).await;
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

        let (state, _) = env.init(false).await.unwrap();

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

        let (state, _) = env.init(false).await.unwrap();

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

        let _ = env.init(false).await.unwrap();

        // the device file should now have the device set to offline
        let device_file = env.layout.device();
        let device = files::read_json::<Device>(&device_file).await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn scanner_spawned_when_enabled() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init(true).await.unwrap();

        // the scanner actor is spawned and its snapshot file is seeded on disk
        assert!(state.scanner.is_some());
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

        let (state, state_handle) = env.init(true).await.unwrap();

        // fail-open: the agent boots and the scanner runs without persistence
        assert!(state.scanner.is_some());

        state.shutdown().await.unwrap();
        state_handle.await;

        // nothing replaced the blocking directory with a snapshot file
        assert!(env.layout.scanner_snapshot().path().is_dir());
    }

    #[tokio::test]
    async fn scanner_absent_when_disabled() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init(false).await.unwrap();

        // no scanner actor is spawned and nothing touches the snapshot file
        assert!(state.scanner.is_none());
        assert!(!env.layout.scanner_snapshot().exists());

        state.shutdown().await.unwrap();
        state_handle.await;
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn success_device_offline() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init(false).await.unwrap();
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
        let (state, state_handle) = env.init(false).await.unwrap();

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
        let (state, state_handle) = env.init(true).await.unwrap();

        // stop the scanner actor directly so AppState::shutdown's scanner
        // step errors (send on a closed actor channel)
        state.scanner.as_ref().unwrap().shutdown().await.unwrap();

        // teardown still completes Ok and the rest of the chain ran
        state.shutdown().await.unwrap();
        state_handle.await;

        let device = files::read_json::<Device>(&env.layout.device())
            .await
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }

    #[tokio::test]
    async fn success_with_scanner_enabled() {
        let env = TestEnv::valid().await;

        let (state, state_handle) = env.init(true).await.unwrap();

        // shutdown completing (and the bundled shutdown handle resolving)
        // proves the scanner actor shut down and its JoinHandle joined; a hung
        // bundle would hang the test and trip the suite-level timeout
        state.shutdown().await.unwrap();
        state_handle.await;
    }
}
