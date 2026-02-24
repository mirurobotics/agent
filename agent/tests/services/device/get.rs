// internal crates
use miru_agent::filesys::dir::Dir;
use miru_agent::models::device::{Device, DeviceStatus};
use miru_agent::services::device::get;
use miru_agent::services::errors::*;
use miru_agent::storage::{self, Layout};

// external crates
use chrono::{DateTime, Utc};

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn device_file_shutdown() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();
        device_file.shutdown().await.unwrap();

        let result = get::get_device(&device_file).await;
        assert!(matches!(result, Err(ServiceErr::FileSysErr(_))));
    }
}

pub mod success {
    use super::*;

    #[tokio::test]
    async fn device_file_does_not_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        layout.device().delete().await.unwrap();

        let device = get::get_device(&device_file).await.unwrap();
        assert_eq!(device, Device::default());
    }

    #[tokio::test]
    async fn device_file_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let result = get::get_device(&device_file).await.unwrap();
        assert_eq!(result, Device::default());
    }

    #[tokio::test]
    async fn returns_custom_device_data() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let custom_device = Device {
            id: "dev-42".to_string(),
            session_id: "sess-99".to_string(),
            name: "test-robot".to_string(),
            agent_version: "1.2.3".to_string(),
            activated: true,
            status: DeviceStatus::Online,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
            last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
        };

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), custom_device.clone())
                .await
                .unwrap();

        let result = get::get_device(&device_file).await.unwrap();
        assert_eq!(result, custom_device);
    }
}
