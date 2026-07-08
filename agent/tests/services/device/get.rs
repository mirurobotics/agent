// internal crates
use miru_agent::disk::{self, Layout};
use miru_agent::filesys::{dirs, files};
use miru_agent::models::{Device, DeviceStatus};
use miru_agent::services::device as dvc_svc;
use miru_agent::services::ServiceErr;

// external crates
use chrono::{DateTime, Utc};

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn device_file_shutdown() {
        let dir = dirs::temp("testing").unwrap();
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
        let dir = dirs::temp("testing").unwrap();
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
        let dir = dirs::temp("testing").unwrap();
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
        let dir = dirs::temp("testing").unwrap();
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
