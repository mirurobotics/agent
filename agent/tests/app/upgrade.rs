// standard crates
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use backend_api::models as backend_client;
use miru_agent::app::upgrade::{needs_upgrade, reconcile, reconcile_impl};
use miru_agent::app::UpgradeErr;
use miru_agent::crypt::rsa;
use miru_agent::filesys::{self, Overwrite, PathExt};
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::models::Device;
use miru_agent::storage::{self, Layout};

// external crates
use chrono::{Duration, Utc};

// ============================ TEST HARNESS ============================ //

/// Build a Layout backed by a temp dir, generate a real RSA keypair under
/// `auth/`, and pre-populate `device.json` with a known device id so that
/// `resolve_device_id` and the JWT-signing path inside `reconcile` both work
/// without contacting a real backend.
async fn prepare_layout(name: &str) -> (Layout, filesys::Dir) {
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

    (layout, dir)
}

/// MockClient pre-wired so JWT issuance succeeds (an RFC3339 expires_at) and
/// `GET /device` returns the supplied backend device.
fn make_mock_client(device: backend_client::Device) -> Arc<MockClient> {
    Arc::new(MockClient {
        issue_device_token_fn: Box::new(|| {
            Ok(backend_client::TokenResponse {
                token: "mock.jwt.token".to_string(),
                expires_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
            })
        }),
        get_device_fn: Mutex::new(Box::new(move || Ok(device.clone()))),
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

type PublicKey = String;
type PrivateKey = String;

async fn read_keys(layout: &Layout) -> (PrivateKey, PublicKey) {
    let auth_dir = layout.auth();
    let private = auth_dir.private_key().read_string().await.unwrap();
    let public = auth_dir.public_key().read_string().await.unwrap();
    (private, public)
}

async fn no_sleep(_: StdDuration) {}

// ============================ TESTS ============================ //

mod reconcile {
    use super::*;

    #[tokio::test]
    async fn is_noop_when_marker_matches() {
        let (layout, _dir) = prepare_layout("upgrade_noop").await;

        // pre-write the marker with the same version we're about to call reconcile
        // with; reconcile() should make zero HTTP calls.
        storage::agent_version::write(&layout.agent_version(), "v1.0.0")
            .await
            .unwrap();

        let mock = make_mock_client(backend_device("dvc_1", "alpha"));
        let outcome = reconcile(&layout, mock.as_ref(), "v1.0.0", no_sleep).await.unwrap();

        assert!(!outcome.upgraded);
        assert_eq!(outcome.attempts, 0);
        assert_eq!(mock.num_get_device_calls(), 0);
        assert_eq!(mock.num_update_device_calls(), 0);
        assert_eq!(mock.call_count(Call::IssueDeviceToken), 0);
    }

    #[tokio::test]
    async fn rebootstraps_when_marker_missing() {
        let (layout, _dir) = prepare_layout("upgrade_missing_marker").await;

        // remember the keys before so we can confirm they survive
        let (priv_before, pub_before) = read_keys(&layout).await;

        let mock = make_mock_client(backend_device("dvc_2", "beta"));
        let outcome = reconcile(&layout, mock.as_ref(), "v0.9.0", no_sleep).await.unwrap();

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 0);

        // marker present, version stamped
        let marker = storage::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some("v0.9.0".to_string()));

        // keys preserved by content
        let (priv_after, pub_after) = read_keys(&layout).await;
        assert_eq!(priv_before, priv_after);
        assert_eq!(pub_before, pub_after);

        // device.json reflects the mock response with the running version
        let on_disk_device = layout.device().read_json::<Device>().await.unwrap();
        assert_eq!(on_disk_device.id, "dvc_2");
        assert_eq!(on_disk_device.name, "beta");

        // backend was told the new version exactly once
        assert_eq!(mock.num_update_device_calls(), 1);
        assert!(mock.num_get_device_calls() >= 1);
    }

    #[tokio::test]
    async fn rebootstraps_when_marker_version_differs() {
        let (layout, _dir) = prepare_layout("upgrade_old_marker").await;

        storage::agent_version::write(&layout.agent_version(), "v0.0.1")
            .await
            .unwrap();

        let mock = make_mock_client(backend_device("dvc_3", "gamma"));
        let outcome = reconcile(&layout, mock.as_ref(), "v0.0.2", no_sleep).await.unwrap();

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 0);

        let marker = storage::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some("v0.0.2".to_string()));
        assert_eq!(mock.num_update_device_calls(), 1);
    }

    #[tokio::test]
    async fn retries_until_get_device_succeeds() {
        let (layout, _dir) = prepare_layout("upgrade_retry").await;

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

        let outcome = reconcile(&layout, mock.as_ref(), "v1.2.3", no_sleep).await.unwrap();

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 2);

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
    async fn reports_attempt_count_and_recovers_after_repeated_failures() {
        let (layout, _dir) = prepare_layout("reconcile_attempt_count").await;
        let device = backend_device("dvc_cs1", "counted");
        let mock = make_mock_client(device.clone());

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

        let outcome = reconcile(&layout, mock.as_ref(), "v9.9.9", no_sleep).await.unwrap();

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 4);
        assert_eq!(mock.num_update_device_calls(), 1);
    }
}

mod needs_upgrade {
    use super::*;

    #[tokio::test]
    async fn returns_true_when_marker_missing() {
        let (layout, _dir) = prepare_layout("needs_upgrade_missing").await;
        assert!(needs_upgrade(&layout, "v1.0.0").await);
    }

    #[tokio::test]
    async fn returns_false_when_marker_matches() {
        let (layout, _dir) = prepare_layout("needs_upgrade_match").await;
        storage::agent_version::write(&layout.agent_version(), "v1.2.3")
            .await
            .unwrap();
        assert!(!needs_upgrade(&layout, "v1.2.3").await);
    }

    #[tokio::test]
    async fn returns_true_when_marker_differs() {
        let (layout, _dir) = prepare_layout("needs_upgrade_differs").await;
        storage::agent_version::write(&layout.agent_version(), "v1.0.0")
            .await
            .unwrap();
        assert!(needs_upgrade(&layout, "v2.0.0").await);
    }

    #[tokio::test]
    async fn returns_true_when_read_errors() {
        let (layout, _dir) = prepare_layout("needs_upgrade_read_err").await;
        // Force a read error: create a directory at the marker path. `exists()`
        // returns true for a directory, so `read_string` runs and fails with a
        // FileSysErr; `needs_upgrade` treats the error as "missing" and returns true.
        tokio::fs::create_dir_all(layout.agent_version().path())
            .await
            .unwrap();
        assert!(needs_upgrade(&layout, "v1.0.0").await);
    }
}

mod reconcile_impl {
    use super::*;

    #[tokio::test]
    async fn happy_path_writes_marker_and_updates_backend() {
        let (layout, _dir) = prepare_layout("reconcile_impl_happy").await;
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
    async fn returns_filesys_err_when_private_key_missing() {
        let (layout, _dir) = prepare_layout("reconcile_impl_no_pk").await;
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
    async fn returns_http_err_when_get_device_fails() {
        let (layout, _dir) = prepare_layout("reconcile_impl_get_fail").await;
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
    async fn returns_storage_err_when_reset_fails() {
        let (layout, _dir) = prepare_layout("reconcile_impl_reset_fail").await;
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
    async fn returns_http_err_when_update_device_fails() {
        let (layout, _dir) = prepare_layout("reconcile_impl_update_fail").await;
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
}
