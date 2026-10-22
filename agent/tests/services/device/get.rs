// standard crates
use std::time::{Duration, Instant};

// internal crates
use crate::mocks::syncer::MockSyncer;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::disk::{self, Layout};
use miru_agent::filesys::files;
use miru_agent::models::{Device, DeviceStatus};
use miru_agent::services::device as dvc_svc;
use miru_agent::services::ServiceErr;
use miru_agent::sync::syncer::State;
use miru_agent::sync::Syncer;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn device_file_shutdown() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        let (device_file, _) =
            disk::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();
        device_file.shutdown().await.unwrap();

        let result = dvc_svc::get(&device_file).await;
        assert!(matches!(result, Err(ServiceErr::FileSysErr(_))));
    }
}

pub mod success {
    use super::*;

    #[tokio::test]
    async fn device_file_does_not_exist() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        let (device_file, _) =
            disk::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        files::delete(&layout.device()).await.unwrap();

        let device = dvc_svc::get(&device_file).await.unwrap();
        assert_eq!(device, Device::default());
    }

    #[tokio::test]
    async fn device_file_exists() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        let (device_file, _) =
            disk::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let result = dvc_svc::get(&device_file).await.unwrap();
        assert_eq!(result, Device::default());
    }

    #[tokio::test]
    async fn returns_custom_device_data() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        let custom_device = Device {
            id: "dev-42".to_string(),
            session_id: "sess-99".to_string(),
            name: "test-robot".to_string(),
            activated: true,
            status: DeviceStatus::Online,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
            last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
        };

        let (device_file, _) =
            disk::Device::spawn_with_default(64, layout.device(), custom_device.clone())
                .await
                .unwrap();

        let result = dvc_svc::get(&device_file).await.unwrap();
        assert_eq!(result, custom_device);
    }
}

pub mod get_secs_since_last_sync {
    use super::*;

    #[tokio::test]
    async fn none_before_first_sync() {
        let syncer = MockSyncer::default();
        syncer.set_state(State::default());

        assert_eq!(dvc_svc::get_secs_since_last_sync(&syncer).await, None);
    }

    #[tokio::test]
    async fn seconds_since_last_sync() {
        let syncer = MockSyncer::default();
        syncer.set_state(State {
            last_synced: Instant::now().checked_sub(Duration::from_secs(5)),
            ..State::default()
        });

        let secs = dvc_svc::get_secs_since_last_sync(&syncer).await.unwrap();
        assert!((5..60).contains(&secs), "got {secs}");
    }

    #[tokio::test]
    async fn none_when_syncer_unreachable() {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        let syncer = Syncer::new(sender);

        assert_eq!(dvc_svc::get_secs_since_last_sync(&syncer).await, None);
    }
}
