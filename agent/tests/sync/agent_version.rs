// internal crates
use miru_agent::filesys::dir::Dir;
use miru_agent::models::device::Device;
use miru_agent::storage::device::DeviceFile;
use miru_agent::storage::layout::StorageLayout;
use miru_agent::sync::agent_version::push;

use crate::http::mock::MockClient;

pub mod push {
    use super::*;

    #[tokio::test]
    async fn same_version() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let agent_version = Device::default().agent_version;
        let device = Device {
            agent_version: agent_version.clone(),
            ..Device::default()
        };

        let (device_file, _) = DeviceFile::spawn_with_default(64, layout.device_file(), device)
            .await
            .unwrap();
        let http_client = MockClient::default();

        push(&device_file, &http_client, "token", agent_version.clone())
            .await
            .unwrap();

        // check the device file has the same version
        let device = device_file.read().await.unwrap();
        assert_eq!(device.agent_version, agent_version);

        // check the http client has been called
        assert_eq!(http_client.num_update_device_calls(), 0);
    }

    #[tokio::test]
    async fn different_version() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let old_agent_version = Device::default().agent_version;
        let new_agent_version = "v1.0.1".to_string();
        let device = Device {
            agent_version: old_agent_version.clone(),
            ..Device::default()
        };

        let (device_file, _) = DeviceFile::spawn_with_default(64, layout.device_file(), device)
            .await
            .unwrap();
        let http_client = MockClient::default();

        push(
            &device_file,
            &http_client,
            "token",
            new_agent_version.clone(),
        )
        .await
        .unwrap();

        // check the device file has the correct version
        let device = device_file.read().await.unwrap();
        assert_eq!(device.agent_version, new_agent_version);

        // check the http client has been called
        assert_eq!(http_client.num_update_device_calls(), 1);
    }
}
