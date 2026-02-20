// internal crates
use miru_agent::filesys::dir::Dir;
use miru_agent::models::device::Device;
use miru_agent::services::device::get;
use miru_agent::services::errors::*;
use miru_agent::storage::{self, Layout};

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn device_file_shutdown() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device_file(), Device::default())
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
            storage::Device::spawn_with_default(64, layout.device_file(), Device::default())
                .await
                .unwrap();

        layout.device_file().delete().await.unwrap();

        let device = get::get_device(&device_file).await.unwrap();
        assert_eq!(device, Device::default());
    }

    #[tokio::test]
    async fn device_file_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device_file(), Device::default())
                .await
                .unwrap();

        let result = get::get_device(&device_file).await.unwrap();
        assert_eq!(result, Device::default());
    }
}
