// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models as backend_client;
use miru_agent::disk;
use miru_agent::filesys::{dirs, Dir, File, PathExt};
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::models::{self, Device, SystemMetadata};
use miru_agent::sync::system_metadata::{sync, SyncArgs, Synced};
use miru_agent::sync::SyncErr;

// ============================ TEST HARNESS ============================ //

const DEVICE_ID: &str = "dvc_1";
const TOKEN: &str = "test-token";

struct Fixture {
    _dir: test_dirs::TempDir,
    http_client: MockClient,
    device: disk::Device,
    cache_file: File,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let dir = test_dirs::temp(name).unwrap();
        let device = Device {
            id: DEVICE_ID.to_string(),
            ..Device::default()
        };
        let (device, _) = disk::Device::spawn_with_default(16, dir.file("device.json"), device)
            .await
            .unwrap();
        let cache_file = dir.file("system_metadata.json");
        Self {
            _dir: dir,
            http_client: MockClient::default(),
            device,
            cache_file,
        }
    }

    fn args(&self) -> SyncArgs<'_, MockClient> {
        SyncArgs {
            http_client: &self.http_client,
            device: &self.device,
            cache_file: &self.cache_file,
            token: TOKEN,
        }
    }

    async fn read_cache(&self) -> Option<SystemMetadata> {
        disk::system_metadata::read(&self.cache_file).await.unwrap()
    }
}

fn network_err() -> HTTPErr {
    HTTPErr::MockErr(HTTPMockErr {
        is_network_conn_err: true,
    })
}

// ============================ TESTS ============================ //

#[tokio::test]
async fn returns_unchanged_when_cache_matches_live() {
    let f = Fixture::new("system_metadata_sync_unchanged").await;
    let live = models::system_metadata();
    disk::system_metadata::write(&f.cache_file, &live)
        .await
        .unwrap();

    let outcome = sync(&f.args()).await.unwrap();

    assert_eq!(outcome, Synced::Unchanged);
    assert_eq!(f.http_client.num_update_device_calls(), 0);
    assert_eq!(f.read_cache().await, Some(live));
}

#[tokio::test]
async fn updates_device_and_writes_cache_when_cache_absent() {
    let f = Fixture::new("system_metadata_sync_absent").await;
    let live = models::system_metadata();

    let outcome = sync(&f.args()).await.unwrap();

    assert_eq!(outcome, Synced::Updated);
    assert_eq!(f.http_client.num_update_device_calls(), 1);

    let requests = f.http_client.requests();
    let update = requests
        .iter()
        .find(|r| r.call == Call::UpdateDevice)
        .expect("an UpdateDevice request was captured");
    assert_eq!(update.path, format!("/devices/{DEVICE_ID}"));
    assert_eq!(update.token.as_deref(), Some(TOKEN));
    let body = update.body.as_deref().expect("the request carried a body");
    assert!(!body.contains("agent_version"));
    let sent: backend_client::UpdateDeviceFromAgentRequest = serde_json::from_str(body).unwrap();
    assert_eq!(sent, live.to_update_request());

    assert_eq!(f.read_cache().await, Some(live));
}

#[tokio::test]
async fn returns_err_and_leaves_cache_untouched_when_update_fails() {
    let f = Fixture::new("system_metadata_sync_update_fails").await;
    // a cache that differs from live forces an update attempt
    let stale = SystemMetadata {
        hostname: Some("stale-host-xyz".to_string()),
        ..SystemMetadata::default()
    };
    disk::system_metadata::write(&f.cache_file, &stale)
        .await
        .unwrap();
    f.http_client.set_update_device(|| Err(network_err()));

    let err = sync(&f.args())
        .await
        .expect_err("expected HTTPErr from the failed update");

    assert!(matches!(err, SyncErr::HTTPClientErr(_)));
    assert_eq!(f.http_client.num_update_device_calls(), 1);
    // the cache is only written after a successful update
    assert_eq!(f.read_cache().await, Some(stale));
}

#[tokio::test]
async fn treats_unreadable_cache_as_absent_and_attempts_update() {
    let f = Fixture::new("system_metadata_sync_unreadable_cache").await;
    // a directory at the cache path makes the read fail; the error is swallowed
    // and treated as "no cache", so an update is attempted
    dirs::create(&Dir::new(f.cache_file.path().clone()))
        .await
        .unwrap();
    f.http_client.set_update_device(|| Err(network_err()));

    let err = sync(&f.args())
        .await
        .expect_err("expected HTTPErr after the unreadable cache forced an update");

    assert!(matches!(err, SyncErr::HTTPClientErr(_)));
    assert_eq!(f.http_client.num_update_device_calls(), 1);
}
