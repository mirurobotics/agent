// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models as backend_client;
use miru_agent::app::metadata_sync::{run_on_boot, sync_system_metadata, Synced};
use miru_agent::app::MetadataSyncErr;
use miru_agent::disk::{self, BackendHost, Layout};
use miru_agent::filesys::{dirs, files, Dir, PathExt, WriteOptions};
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::models::{self, Device, SystemMetadata};

// ============================ TEST HARNESS ============================ //

/// Layout backed by a temp dir. The returned `TempDir` deletes the directory on
/// drop, so bind it for the lifetime of the test.
fn layout(name: &str) -> (Layout, test_dirs::TempDir) {
    let dir = test_dirs::temp(name).unwrap();
    let layout = Layout::new(dir.to_dir());
    (layout, dir)
}

/// Seed `device.json` with a known id so `resolve_device_id` succeeds without a
/// token or backend call.
async fn seed_device(layout: &Layout, id: &str) {
    let device = Device {
        id: id.to_string(),
        ..Device::default()
    };
    files::write_json(&layout.device(), &device, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
}

fn network_err() -> HTTPErr {
    HTTPErr::MockErr(HTTPMockErr {
        is_network_conn_err: true,
    })
}

// ============================ TESTS ============================ //

mod sync_system_metadata {
    use super::*;

    #[tokio::test]
    async fn returns_unchanged_when_cache_matches_live() {
        let (layout, _tmp) = layout("metadata_sync_unchanged");
        seed_device(&layout, "dvc_unchanged").await;
        let live = models::system_metadata();
        disk::system_metadata::write(&layout.system_metadata(), &live)
            .await
            .unwrap();
        let mock = MockClient::default();

        let outcome = sync_system_metadata(&mock, &layout, "test-token")
            .await
            .unwrap();

        assert_eq!(outcome, Synced::Unchanged);
        assert_eq!(mock.num_update_device_calls(), 0);
        let cache_after = disk::system_metadata::read(&layout.system_metadata())
            .await
            .unwrap();
        assert_eq!(cache_after, Some(live));
    }

    #[tokio::test]
    async fn patches_backend_and_writes_cache_when_cache_absent() {
        let (layout, _tmp) = layout("metadata_sync_absent");
        seed_device(&layout, "dvc_absent").await;
        let live = models::system_metadata();
        let mock = MockClient::default();

        let outcome = sync_system_metadata(&mock, &layout, "test-token")
            .await
            .unwrap();

        assert_eq!(outcome, Synced::Updated);
        assert_eq!(mock.num_update_device_calls(), 1);

        let requests = mock.requests();
        let update = requests
            .iter()
            .find(|r| r.call == Call::UpdateDevice)
            .expect("an UpdateDevice request was captured");
        let body = update.body.as_deref().expect("the request carried a body");
        assert!(!body.contains("agent_version"));
        let sent: backend_client::UpdateDeviceFromAgentRequest =
            serde_json::from_str(body).unwrap();
        assert_eq!(sent, live.to_update_request());
        assert_eq!(update.token.as_deref(), Some("test-token"));

        let cache_after = disk::system_metadata::read(&layout.system_metadata())
            .await
            .unwrap();
        assert_eq!(cache_after, Some(live));
    }

    #[tokio::test]
    async fn returns_err_and_leaves_cache_untouched_when_patch_fails() {
        let (layout, _tmp) = layout("metadata_sync_patch_fails");
        seed_device(&layout, "dvc_patch_fail").await;
        // Seed a cache that differs from live so the differing-Some comparison
        // branch runs and an update is attempted.
        let stale = SystemMetadata {
            hostname: Some("stale-host-xyz".to_string()),
            ..SystemMetadata::default()
        };
        disk::system_metadata::write(&layout.system_metadata(), &stale)
            .await
            .unwrap();
        let mock = MockClient::default();
        mock.set_update_device(|| Err(network_err()));

        let err = sync_system_metadata(&mock, &layout, "test-token")
            .await
            .expect_err("expected HTTPErr from the failed PATCH");

        assert!(matches!(err, MetadataSyncErr::HTTPErr(_)));
        assert_eq!(mock.num_update_device_calls(), 1);
        // The cache is never overwritten with live because the PATCH failed
        // before the write: it still holds the stale value.
        let cache_after = disk::system_metadata::read(&layout.system_metadata())
            .await
            .unwrap();
        assert_eq!(cache_after, Some(stale));
    }

    #[tokio::test]
    async fn treats_unreadable_cache_as_absent_and_attempts_update() {
        let (layout, _tmp) = layout("metadata_sync_unreadable_cache");
        seed_device(&layout, "dvc_unreadable").await;
        // A directory at the cache path makes `exists()` true but `read_json`
        // fail; the read error is swallowed and treated as "no cache", so an
        // update is attempted.
        dirs::create(&Dir::new(layout.system_metadata().path().clone()))
            .await
            .unwrap();
        let mock = MockClient::default();
        mock.set_update_device(|| Err(network_err()));

        let err = sync_system_metadata(&mock, &layout, "test-token")
            .await
            .expect_err("expected HTTPErr after the unreadable cache forced an update");

        assert!(matches!(err, MetadataSyncErr::HTTPErr(_)));
        assert_eq!(mock.num_update_device_calls(), 1);
    }
}

mod run_on_boot {
    use super::*;

    #[tokio::test]
    async fn swallows_errors_and_returns_when_token_issuance_fails() {
        // With no auth keys on disk, token issuance fails before any network
        // call. run_on_boot must log-and-swallow and return without panicking,
        // proving it never blocks boot.
        let (layout, _tmp) = layout("metadata_sync_run_on_boot_no_keys");

        run_on_boot(&layout, &BackendHost::default()).await;
    }
}
