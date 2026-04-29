// standard crates
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use miru_agent::app::state::AppState;
use miru_agent::authn::Token;
use miru_agent::deploy::fsm;
use miru_agent::filesys::{self, FileSysErr, WriteOptions};
use miru_agent::http;
use miru_agent::logs;
use miru_agent::models::{self, Device, DeviceStatus};
use miru_agent::server::ServerErr;
use miru_agent::storage::{Capacities, Layout, StorageErr};

// external crates
use chrono::Utc;

pub mod init {
    use super::*;

    #[tokio::test]
    async fn fail_missing_private_key_file() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let result = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await;
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
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        // create a private key file (but no public key file)
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        let result = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await;
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
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        // create a private key file
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();
        // create a public key file
        let public_key_file = layout.auth().public_key();
        public_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        let result = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServerErr::StorageErr(StorageErr::ResolveDeviceIDErr(_)))
        ));
    }

    #[tokio::test]
    async fn success_missing_device_file_but_valid_token() {
        let begin_test = Utc::now().timestamp();
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create a private key file
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create a public key file
        let public_key_file = layout.auth().public_key();
        public_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create the token file with a token containing a device id
        let token_file = layout.auth().token();
        let token = Token {
                token: "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpYXQiOjE3NDU2MzgzMTUsInN1YiI6ImNsaV8xMjMiLCJpc3MiOiJtaXJ1IiwiYXVkIjoiY2xpZW50IiwiZXhwIjoxNzIxNTE3MDM0fQ.4ARFzYZSF_i9PjPZRJtH7HcmE_vv5tuZIpKkniua6BY".to_string(),
                expires_at: Utc::now(),
            };
        token_file
            .write_json(&token, WriteOptions::default())
            .await
            .unwrap();

        let (state, _) = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await
        .unwrap();

        // check last activity
        assert!(state.activity_tracker.last_touched() <= Utc::now().timestamp() as u64);
        assert!(state.activity_tracker.last_touched() >= begin_test as u64);

        // the device file should now exist with some reasonable defaults
        let device_file = layout.device();
        let expected_device = Device {
            id: "cli_123".to_string(),
            activated: true,
            status: DeviceStatus::Offline,
            ..Device::default()
        };
        let device = device_file.read_json::<Device>().await.unwrap();
        assert_eq!(device, expected_device);
    }

    #[tokio::test]
    async fn success_missing_token_file() {
        let begin_test = Utc::now().timestamp();
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create a private key file
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create a public key file
        let public_key_file = layout.auth().public_key();
        public_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create the device file
        let device_file = layout.device();
        let device = Device::default();
        device_file
            .write_json(&device, WriteOptions::default())
            .await
            .unwrap();

        let (state, _) = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await
        .unwrap();

        // the token file should now have the default token
        let token_file = layout.auth().token();
        let token = token_file.read_json::<Token>().await.unwrap();
        assert_eq!(token.token, Token::default().token);

        // check last activity
        assert!(state.activity_tracker.last_touched() <= Utc::now().timestamp() as u64);
        assert!(state.activity_tracker.last_touched() >= begin_test as u64);
    }

    #[tokio::test]
    async fn success_set_device_to_offline_on_boot() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create a private key file
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create a public key file
        let public_key_file = layout.auth().public_key();
        public_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create the device file
        let device_file = layout.device();
        let device = Device {
            id: "dvc_123".to_string(),
            activated: true,
            status: DeviceStatus::Online,
            ..Device::default()
        };
        device_file
            .write_json(&device, WriteOptions::default())
            .await
            .unwrap();

        let _ = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await
        .unwrap();

        // the device file should now have the device set to offline
        let device_file = layout.device();
        let device = device_file.read_json::<Device>().await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn success_device_offline() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create a private key file
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create a public key file
        let public_key_file = layout.auth().public_key();
        public_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create the device file
        let device_file = layout.device();
        let device = Device::default();
        device_file
            .write_json(&device, WriteOptions::default())
            .await
            .unwrap();

        let (state, state_handle) = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await
        .unwrap();
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

        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create a private key file
        let private_key_file = layout.auth().private_key();
        private_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create a public key file
        let public_key_file = layout.auth().public_key();
        public_key_file
            .write_string("test", WriteOptions::default())
            .await
            .unwrap();

        // create the device file
        let device_file = layout.device();
        let device = Device::default();
        device_file
            .write_json(&device, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();

        let before_shutdown = Utc::now();
        let (state, state_handle) = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await
        .unwrap();

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
        let device_file = layout.device();
        let device = device_file.read_json::<Device>().await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
        assert!(device.last_disconnected_at >= before_shutdown);
        assert!(device.last_disconnected_at <= Utc::now());
    }
}
