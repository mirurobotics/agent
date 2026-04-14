// internal crates
use crate::mocks::http_client::MockClient;
use miru_agent::filesys;
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::models::Device;
use miru_agent::storage::{self, Layout};
use miru_agent::sync::agent_version::push;

pub mod push {
    use super::*;

    #[tokio::test]
    async fn same_version() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let agent_version = Device::default().agent_version;
        let device = Device {
            agent_version: agent_version.clone(),
            ..Device::default()
        };

        let (device_file, _) = storage::Device::spawn_with_default(64, layout.device(), device)
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
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let old_agent_version = Device::default().agent_version;
        let new_agent_version = "v1.0.1".to_string();
        let device = Device {
            agent_version: old_agent_version.clone(),
            ..Device::default()
        };

        let (device_file, _) = storage::Device::spawn_with_default(64, layout.device(), device)
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

    #[tokio::test]
    async fn http_update_failure() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let old_agent_version = Device::default().agent_version;
        let new_agent_version = "v1.0.2".to_string();
        let device = Device {
            agent_version: old_agent_version.clone(),
            ..Device::default()
        };

        let (device_file, _) = storage::Device::spawn_with_default(64, layout.device(), device)
            .await
            .unwrap();
        let http_client = MockClient {
            update_device_fn: std::sync::Mutex::new(Box::new(|| {
                Err(HTTPErr::MockErr(HTTPMockErr {
                    is_network_conn_err: false,
                }))
            })),
            ..MockClient::default()
        };

        let result = push(
            &device_file,
            &http_client,
            "token",
            new_agent_version.clone(),
        )
        .await;

        assert!(result.is_err(), "push should fail when HTTP update fails");

        // Local storage should still have the old version (HTTP call happens before patch)
        let device = device_file.read().await.unwrap();
        assert_eq!(device.agent_version, old_agent_version);
    }
}
