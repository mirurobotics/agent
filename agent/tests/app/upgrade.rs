// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use backend_api::models as backend_client;
use miru_agent::app::upgrade::{needs_upgrade, reconcile, reconcile_impl};
use miru_agent::app::UpgradeErr;
use miru_agent::crypt::rsa;
use miru_agent::filesys::{self, Overwrite, PathExt, WriteOptions};
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::models::Device;
use miru_agent::storage::{self, Layout};

// external crates
use chrono::{Duration, Utc};

// ============================ TEST HARNESS ============================ //

const PLACEHOLDER_PUBLIC_KEY: &str = "PLACEHOLDER_PUBLIC_KEY";

/// Build a Layout backed by a temp dir, generate a real RSA keypair under
/// `auth/`, and pre-populate `device.json` with a known device id so that
/// `resolve_device_id` and the JWT-signing path inside `reconcile` both work
/// without contacting a real backend.
async fn prepare_layout(name: &str, device_id: &str) -> (Layout, filesys::Dir) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let layout = Layout::new(dir.clone());

    // generate a real RSA keypair under auth/
    let auth_dir = layout.auth();
    auth_dir.root.create_if_absent().await.unwrap();
    rsa::gen_key_pair(
        2048,
        &auth_dir.private_key(),
        &auth_dir.public_key(),
        Overwrite::Allow,
    )
    .await
    .unwrap();

    // write a device file so resolve_device_id can find the id
    let device = Device {
        id: device_id.to_string(),
        ..Device::default()
    };
    layout
        .device()
        .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    (layout, dir)
}

/// MockClient pre-wired so JWT issuance succeeds (an RFC3339 expires_at) and
/// `GET /device` returns the supplied backend device.
fn make_mock_client(device: backend_client::Device) -> Arc<MockClient> {
    let device_for_get = device.clone();
    Arc::new(MockClient {
        issue_device_token_fn: Box::new(|| {
            Ok(backend_client::TokenResponse {
                token: "mock.jwt.token".to_string(),
                expires_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
            })
        }),
        get_device_fn: Mutex::new(Box::new(move || Ok(device_for_get.clone()))),
        ..MockClient::default()
    })
}

fn backend_device(id: &str, name: &str) -> backend_client::Device {
    backend_client::Device {
        id: id.to_string(),
        name: name.to_string(),
        agent_version: Some("v0.0.0".to_string()),
        session_id: "ses_1".to_string(),
        ..backend_client::Device::default()
    }
}

async fn read_keys(layout: &Layout) -> (String, String) {
    let auth_dir = layout.auth();
    let private = auth_dir.private_key().read_string().await.unwrap();
    let public = auth_dir.public_key().read_string().await.unwrap();
    (private, public)
}

// ============================ TESTS ============================ //

#[tokio::test]
async fn reconcile_is_noop_when_marker_matches() {
    let (layout, _dir) = prepare_layout("upgrade_noop", "dvc_1").await;

    // pre-write the marker with the same version we're about to call reconcile
    // with; reconcile() should make zero HTTP calls.
    storage::agent_version::write(&layout.agent_version(), "v1.0.0")
        .await
        .unwrap();

    let mock = make_mock_client(backend_device("dvc_1", "alpha"));
    reconcile(&layout, mock.as_ref(), "v1.0.0", tokio::time::sleep).await;

    assert_eq!(mock.num_get_device_calls(), 0);
    assert_eq!(mock.num_update_device_calls(), 0);
    assert_eq!(mock.call_count(Call::IssueDeviceToken), 0);
}

#[tokio::test]
async fn reconcile_rebootstraps_when_marker_missing() {
    let (layout, _dir) = prepare_layout("upgrade_missing_marker", "dvc_2").await;

    // remember the keys before so we can confirm they survive
    let (priv_before, pub_before) = read_keys(&layout).await;

    let mock = make_mock_client(backend_device("dvc_2", "beta"));
    reconcile(&layout, mock.as_ref(), "v0.9.0", tokio::time::sleep).await;

    // marker present, version stamped
    let marker = storage::agent_version::read(&layout.agent_version())
        .await
        .unwrap();
    assert_eq!(marker, Some("v0.9.0".to_string()));

    // keys preserved by content
    let (priv_after, pub_after) = read_keys(&layout).await;
    assert_eq!(priv_before, priv_after);
    assert_eq!(pub_before, pub_after);
    assert_ne!(priv_after, PLACEHOLDER_PUBLIC_KEY);

    // device.json reflects the mock response with the running version
    let on_disk_device = layout.device().read_json::<Device>().await.unwrap();
    assert_eq!(on_disk_device.id, "dvc_2");
    assert_eq!(on_disk_device.name, "beta");

    // backend was told the new version exactly once
    assert_eq!(mock.num_update_device_calls(), 1);
    assert!(mock.num_get_device_calls() >= 1);
}

#[tokio::test]
async fn reconcile_rebootstraps_when_marker_version_differs() {
    let (layout, _dir) = prepare_layout("upgrade_old_marker", "dvc_3").await;

    storage::agent_version::write(&layout.agent_version(), "v0.0.1")
        .await
        .unwrap();

    let mock = make_mock_client(backend_device("dvc_3", "gamma"));
    reconcile(&layout, mock.as_ref(), "v0.0.2", tokio::time::sleep).await;

    let marker = storage::agent_version::read(&layout.agent_version())
        .await
        .unwrap();
    assert_eq!(marker, Some("v0.0.2".to_string()));
    assert_eq!(mock.num_update_device_calls(), 1);
}

#[tokio::test]
async fn reconcile_retries_until_get_device_succeeds() {
    let (layout, _dir) = prepare_layout("upgrade_retry", "dvc_4").await;

    let device = backend_device("dvc_4", "delta");
    let mock = make_mock_client(device.clone());

    // First two GET /device calls fail with a network error, third succeeds.
    let call_counter = Arc::new(Mutex::new(0u32));
    let counter_clone = call_counter.clone();
    let device_clone = device.clone();
    mock.set_get_device(move || {
        let mut n = counter_clone.lock().unwrap();
        *n += 1;
        if *n < 3 {
            Err(HTTPErr::MockErr(HTTPMockErr {
                is_network_conn_err: true,
            }))
        } else {
            Ok(device_clone.clone())
        }
    });

    // Use a tight backoff (the upgrade module's retry sleeps in seconds; we
    // want this test to finish quickly). The cooldown::calc formula clamps
    // at max_secs, so if the test's retry logic accidentally overshoots
    // we'd hang. We rely on the real backoff (base 1s, max 12h) — that
    // means each retry waits 1s, 2s, ... so this test ends in ~3s.
    reconcile(&layout, mock.as_ref(), "v1.2.3", tokio::time::sleep).await;

    // marker now reflects the new version
    let marker = storage::agent_version::read(&layout.agent_version())
        .await
        .unwrap();
    assert_eq!(marker, Some("v1.2.3".to_string()));

    // GET /device was called at least 3 times
    assert!(mock.num_get_device_calls() >= 3);
    assert_eq!(mock.num_update_device_calls(), 1);
}

#[tokio::test]
async fn needs_upgrade_returns_true_when_marker_missing() {
    let (layout, _dir) = prepare_layout("needs_upgrade_missing", "dvc_nu1").await;
    assert!(needs_upgrade(&layout, "v1.0.0").await);
}

#[tokio::test]
async fn needs_upgrade_returns_false_when_marker_matches() {
    let (layout, _dir) = prepare_layout("needs_upgrade_match", "dvc_nu2").await;
    storage::agent_version::write(&layout.agent_version(), "v1.2.3")
        .await
        .unwrap();
    assert!(!needs_upgrade(&layout, "v1.2.3").await);
}

#[tokio::test]
async fn needs_upgrade_returns_true_when_marker_differs() {
    let (layout, _dir) = prepare_layout("needs_upgrade_differs", "dvc_nu3").await;
    storage::agent_version::write(&layout.agent_version(), "v1.0.0")
        .await
        .unwrap();
    assert!(needs_upgrade(&layout, "v2.0.0").await);
}

#[tokio::test]
async fn needs_upgrade_returns_true_when_read_errors() {
    let (layout, _dir) = prepare_layout("needs_upgrade_read_err", "dvc_nu4").await;
    // Force a read error: create a directory at the marker path. `exists()`
    // returns true for a directory, so `read_string` runs and fails with a
    // FileSysErr; `needs_upgrade` treats the error as "missing" and returns true.
    tokio::fs::create_dir_all(layout.agent_version().path())
        .await
        .unwrap();
    assert!(needs_upgrade(&layout, "v1.0.0").await);
}

#[tokio::test]
async fn reconcile_impl_happy_path_writes_marker_and_updates_backend() {
    let (layout, _dir) = prepare_layout("reconcile_impl_happy", "dvc_ri1").await;
    let mock = make_mock_client(backend_device("dvc_ri1", "happy"));

    let version = "v3.4.5";
    reconcile_impl(mock.as_ref(), &layout, version)
        .await
        .unwrap();

    let marker = storage::agent_version::read(&layout.agent_version())
        .await
        .unwrap();
    assert_eq!(marker, Some(version.to_string()));
    assert_eq!(mock.num_update_device_calls(), 1);
    assert!(mock.num_get_device_calls() >= 1);
    assert!(mock.call_count(Call::IssueDeviceToken) >= 1);
}

#[tokio::test]
async fn reconcile_impl_returns_filesys_err_when_private_key_missing() {
    let (layout, _dir) = prepare_layout("reconcile_impl_no_pk", "dvc_ri2").await;
    tokio::fs::remove_file(layout.auth().private_key().path())
        .await
        .unwrap();

    let mock = make_mock_client(backend_device("dvc_ri2", "no_pk"));
    let err = reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
        .await
        .expect_err("expected FileSysErr from missing private key");
    match err {
        UpgradeErr::FileSysErr(_) => {}
        other => panic!("expected UpgradeErr::FileSysErr, got {other:?}"),
    }
}

#[tokio::test]
async fn reconcile_impl_returns_http_err_when_get_device_fails() {
    let (layout, _dir) = prepare_layout("reconcile_impl_get_fail", "dvc_ri3").await;
    let mock = make_mock_client(backend_device("dvc_ri3", "get_fail"));
    mock.set_get_device(|| {
        Err(HTTPErr::MockErr(HTTPMockErr {
            is_network_conn_err: true,
        }))
    });

    let err = reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
        .await
        .expect_err("expected HTTPErr from get_device failure");
    match err {
        UpgradeErr::HTTPErr(_) => {}
        other => panic!("expected UpgradeErr::HTTPErr, got {other:?}"),
    }
}

#[tokio::test]
async fn reconcile_impl_returns_storage_err_when_reset_fails() {
    let (layout, _dir) = prepare_layout("reconcile_impl_reset_fail", "dvc_ri4").await;
    // Replace device.json (a regular file) with a directory of the same name.
    // setup::reset writes device.json via an atomic write that cannot replace
    // a directory, so reset returns StorageErr::FileSysErr(_).
    tokio::fs::remove_file(layout.device().path())
        .await
        .unwrap();
    tokio::fs::create_dir_all(layout.device().path())
        .await
        .unwrap();

    let mock = make_mock_client(backend_device("dvc_ri4", "reset_fail"));
    let err = reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
        .await
        .expect_err("expected StorageErr from reset failure");
    match err {
        UpgradeErr::StorageErr(_) => {}
        other => panic!("expected UpgradeErr::StorageErr, got {other:?}"),
    }
}

#[tokio::test]
async fn reconcile_impl_returns_http_err_when_update_device_fails() {
    let (layout, _dir) = prepare_layout("reconcile_impl_update_fail", "dvc_ri5").await;
    let mock = make_mock_client(backend_device("dvc_ri5", "update_fail"));
    mock.set_update_device(|| {
        Err(HTTPErr::MockErr(HTTPMockErr {
            is_network_conn_err: true,
        }))
    });

    let version = "v7.8.9";
    let err = reconcile_impl(mock.as_ref(), &layout, version)
        .await
        .expect_err("expected HTTPErr from update_device failure");
    match err {
        UpgradeErr::HTTPErr(_) => {}
        other => panic!("expected UpgradeErr::HTTPErr, got {other:?}"),
    }

    // setup::reset wrote the marker before update_device ran, so the marker
    // reflects the new version even though update_device failed.
    let marker = storage::agent_version::read(&layout.agent_version())
        .await
        .unwrap();
    assert_eq!(marker, Some(version.to_string()));
}

#[tokio::test]
async fn reconcile_uses_injected_sleep_and_recovers_after_repeated_failures() {
    let (layout, _dir) = prepare_layout("reconcile_counted_sleep", "dvc_cs1").await;
    let device = backend_device("dvc_cs1", "counted");
    let mock = make_mock_client(device.clone());

    // Fail get_device 4 times, then succeed. The reconcile loop should sleep
    // exactly 4 times via the injected counting sleep_fn before the 5th
    // attempt succeeds.
    let call_counter = Arc::new(Mutex::new(0u32));
    let counter_clone = call_counter.clone();
    let device_clone = device.clone();
    mock.set_get_device(move || {
        let mut n = counter_clone.lock().unwrap();
        *n += 1;
        if *n <= 4 {
            Err(HTTPErr::MockErr(HTTPMockErr {
                is_network_conn_err: true,
            }))
        } else {
            Ok(device_clone.clone())
        }
    });

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let sleep_fn = move |_: StdDuration| {
        counter.fetch_add(1, Ordering::SeqCst);
        async {}
    };

    reconcile(&layout, mock.as_ref(), "v9.9.9", sleep_fn).await;

    assert_eq!(
        sleep_count.load(Ordering::SeqCst),
        4,
        "expected exactly 4 sleeps for 4 injected failures"
    );
    assert_eq!(mock.num_update_device_calls(), 1);
}
