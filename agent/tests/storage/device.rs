// internal crates
use miru_agent::filesys::{dir::Dir, WriteOptions};
use miru_agent::models::device::Device;
use miru_agent::storage::{device::assert_activated, errors::StorageErr};

pub mod assert_activated {
    use super::*;

    #[tokio::test]
    async fn file_does_not_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");

        let result = assert_activated(&device_file).await.unwrap_err();
        assert!(matches!(result, StorageErr::FileSysErr { .. }));
    }

    #[tokio::test]
    async fn invalid_file_contents() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");
        device_file
            .write_string("not a valid device", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = assert_activated(&device_file).await.unwrap_err();
        assert!(matches!(result, StorageErr::FileSysErr { .. }));
    }

    #[tokio::test]
    async fn device_not_activated() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");
        let device = Device {
            activated: false,
            ..Default::default()
        };
        device_file
            .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = assert_activated(&device_file).await.unwrap_err();
        assert!(matches!(result, StorageErr::DeviceNotActivatedErr { .. }));
    }

    #[tokio::test]
    async fn device_activated() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");
        let device = Device {
            activated: true,
            ..Default::default()
        };
        device_file
            .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        assert_activated(&device_file).await.unwrap();
    }
}
