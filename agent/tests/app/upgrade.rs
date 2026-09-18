// standard crates
use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::filesys::{dirs as test_dirs, files as test_files};
use backend_api::models as backend_client;
use miru_agent::app::upgrade::{needs_upgrade, reconcile, reconcile_impl, Outcome, Reconcile};
use miru_agent::app::UpgradeErr;
use miru_agent::crypt::rsa;
use miru_agent::disk::{self, Backend, BackendHost, Layout, MQTTBroker, MqttHost, Settings};
use miru_agent::filesys::{dirs, files, FileSysErr, Overwrite, PathExt, WriteOptions};
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::models::Device;
use miru_agent::shutdown::Latch;

// external crates
use chrono::{Duration, Utc};
use tokio::sync::oneshot;

// ============================ TEST HARNESS ============================ //

/// Build a Layout backed by a temp dir, generate a real RSA keypair under
/// `auth/`, and pre-populate `device.json` with a known device id so that
/// `resolve_device_id` and the JWT-signing path inside `reconcile` both work
/// without contacting a real backend.
async fn prepare_layout(name: &str) -> (Layout, test_dirs::TempDir) {
    let dir = test_dirs::temp(name).unwrap();
    let layout = Layout::new(dir.to_dir());

    // generate a real RSA keypair under auth/
    let auth_dir = layout.auth();
    dirs::create_if_absent(&auth_dir.root).await.unwrap();
    rsa::gen_key_pair(
        rsa::KeySize::Rsa2048,
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
    let private = files::read_string(&auth_dir.private_key()).await.unwrap();
    let public = files::read_string(&auth_dir.public_key()).await.unwrap();
    (private, public)
}

async fn seed_upgrade_state(layout: &Layout) {
    let device = Device::from(&backend_device("dvc_old", "old"));
    let settings = Settings {
        enable_poller: false,
        ..Settings::default()
    };
    disk::setup::reset(layout, &device, &settings, "v0.0.1")
        .await
        .unwrap();
    dirs::create(&layout.resources()).await.unwrap();
    test_files::seed(&layout.resources().file("stale.json"), "old resource").await;
    test_files::seed(&layout.events_log_file(), "old event").await;
    test_files::seed(
        &layout.auth().token(),
        r#"{"token":"old.jwt.token","expires_at":"2026-01-01T00:00:00Z"}"#,
    )
    .await;
}

async fn read_upgrade_state(layout: &Layout) -> Vec<String> {
    let mut state = Vec::new();
    for file in [
        layout.agent_version(),
        layout.device(),
        layout.settings(),
        layout.auth().token(),
        layout.resources().file("stale.json"),
        layout.events_log_file(),
    ] {
        state.push(files::read_string(&file).await.unwrap());
    }
    state
}

async fn no_sleep(_: StdDuration) {}

// ============================ TESTS ============================ //

mod reconcile {
    use super::*;

    fn ready(result: Result<Reconcile, UpgradeErr>) -> Outcome {
        match result.unwrap() {
            Reconcile::Ready(outcome) => outcome,
            Reconcile::Stopped => panic!("expected Ready"),
        }
    }

    #[tokio::test]
    async fn is_noop_when_marker_matches() {
        let (layout, _tmp) = prepare_layout("upgrade_noop").await;

        // pre-write the marker with the same version we're about to call reconcile
        // with; reconcile() should make zero HTTP calls.
        disk::agent_version::write(&layout.agent_version(), "v1.0.0")
            .await
            .unwrap();

        let mock = make_mock_client(backend_device("dvc_1", "alpha"));
        let outcome =
            ready(reconcile(&layout, mock.as_ref(), "v1.0.0", no_sleep, &Latch::new()).await);

        assert!(!outcome.upgraded);
        assert_eq!(outcome.attempts, 0);
        assert_eq!(mock.num_get_device_calls(), 0);
        assert_eq!(mock.num_update_device_calls(), 0);
        assert_eq!(mock.call_count(Call::IssueDeviceToken), 0);
    }

    #[tokio::test]
    async fn rebootstraps_when_marker_missing() {
        let (layout, _tmp) = prepare_layout("upgrade_missing_marker").await;

        // remember the keys before so we can confirm they survive
        let (priv_before, pub_before) = read_keys(&layout).await;

        let mock = make_mock_client(backend_device("dvc_2", "beta"));
        let outcome =
            ready(reconcile(&layout, mock.as_ref(), "v0.9.0", no_sleep, &Latch::new()).await);

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 0);

        // marker present, version stamped
        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some("v0.9.0".to_string()));

        // keys preserved by content
        let (priv_after, pub_after) = read_keys(&layout).await;
        assert_eq!(priv_before, priv_after);
        assert_eq!(pub_before, pub_after);

        // device.json reflects the mock response with the running version
        let on_disk_device = files::read_json::<Device>(&layout.device()).await.unwrap();
        assert_eq!(on_disk_device.id, "dvc_2");
        assert_eq!(on_disk_device.name, "beta");

        // backend was told the new version exactly once
        assert_eq!(mock.num_update_device_calls(), 1);
        assert!(mock.num_get_device_calls() >= 1);
    }

    #[tokio::test]
    async fn rebootstraps_when_marker_version_differs() {
        let (layout, _tmp) = prepare_layout("upgrade_old_marker").await;

        disk::agent_version::write(&layout.agent_version(), "v0.0.1")
            .await
            .unwrap();

        let mock = make_mock_client(backend_device("dvc_3", "gamma"));
        let outcome =
            ready(reconcile(&layout, mock.as_ref(), "v0.0.2", no_sleep, &Latch::new()).await);

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 0);

        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some("v0.0.2".to_string()));
        assert_eq!(mock.num_update_device_calls(), 1);
    }

    #[tokio::test]
    async fn retries_until_get_device_succeeds() {
        let (layout, _tmp) = prepare_layout("upgrade_retry").await;

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

        let outcome =
            ready(reconcile(&layout, mock.as_ref(), "v1.2.3", no_sleep, &Latch::new()).await);

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 2);

        // marker now reflects the new version
        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some("v1.2.3".to_string()));

        // GET /device was called at least 3 times
        assert!(mock.num_get_device_calls() >= 3);
        assert_eq!(mock.num_update_device_calls(), 1);
    }

    #[tokio::test]
    async fn reports_attempt_count_and_recovers_after_repeated_failures() {
        let (layout, _tmp) = prepare_layout("reconcile_attempt_count").await;
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

        let outcome =
            ready(reconcile(&layout, mock.as_ref(), "v9.9.9", no_sleep, &Latch::new()).await);

        assert!(outcome.upgraded);
        assert_eq!(outcome.attempts, 4);
        assert_eq!(mock.num_update_device_calls(), 1);
    }

    #[tokio::test]
    async fn stops_during_retry_wait_without_changing_state() {
        let (layout, _tmp) = prepare_layout("upgrade_stop_retry").await;
        seed_upgrade_state(&layout).await;
        let state_before = read_upgrade_state(&layout).await;
        let keys_before = read_keys(&layout).await;
        let mock = make_mock_client(backend_device("dvc_new", "new"));
        mock.set_get_device(|| {
            Err(HTTPErr::MockErr(HTTPMockErr {
                is_network_conn_err: true,
            }))
        });

        let latch = Latch::new();
        let (entered_tx, entered_rx) = oneshot::channel();
        let entered_tx = Mutex::new(Some(entered_tx));
        let sleep_fn = |_| {
            let entered_tx = entered_tx.lock().unwrap().take().unwrap();
            async move {
                entered_tx.send(()).unwrap();
                pending::<()>().await;
            }
        };
        let attempt = reconcile(&layout, mock.as_ref(), "v1.0.0", sleep_fn, &latch);
        let request_stop = async {
            entered_rx.await.unwrap();
            latch.trigger();
        };
        let (outcome, ()) = tokio::time::timeout(StdDuration::from_secs(5), async {
            tokio::join!(attempt, request_stop)
        })
        .await
        .expect("shutdown should interrupt the retry wait");

        assert_eq!(outcome.unwrap(), Reconcile::Stopped);
        assert_eq!(1, mock.call_count(Call::IssueDeviceToken));
        assert_eq!(1, mock.num_get_device_calls());
        assert_eq!(0, mock.num_update_device_calls());
        assert_eq!(state_before, read_upgrade_state(&layout).await);
        assert_eq!(keys_before, read_keys(&layout).await);
    }

    #[tokio::test]
    async fn already_requested_stop_skips_reconciliation() {
        let (layout, _tmp) = prepare_layout("upgrade_stop_before_attempt").await;
        seed_upgrade_state(&layout).await;
        let state_before = read_upgrade_state(&layout).await;
        let keys_before = read_keys(&layout).await;
        let mock = make_mock_client(backend_device("dvc_new", "new"));
        let latch = Latch::new();
        latch.trigger();

        let outcome = reconcile(&layout, mock.as_ref(), "v1.0.0", no_sleep, &latch)
            .await
            .unwrap();

        assert_eq!(outcome, Reconcile::Stopped);
        assert!(mock.requests().is_empty());
        assert_eq!(state_before, read_upgrade_state(&layout).await);
        assert_eq!(keys_before, read_keys(&layout).await);
    }

    #[tokio::test]
    async fn stop_during_attempt_waits_for_reset_and_backend_update() {
        let (layout, _tmp) = prepare_layout("upgrade_stop_during_attempt").await;
        seed_upgrade_state(&layout).await;
        let keys_before = read_keys(&layout).await;
        let settings_before = files::read_json::<Settings>(&layout.settings())
            .await
            .unwrap();
        let backend_device = backend_device("dvc_new", "new");
        let expected_device = Device::from(&backend_device);
        let mock = make_mock_client(backend_device.clone());
        let latch = Latch::new();
        let latch_on_get = latch.clone();
        mock.set_get_device(move || {
            latch_on_get.trigger();
            Ok(backend_device.clone())
        });

        let outcome = tokio::time::timeout(
            StdDuration::from_secs(5),
            reconcile(&layout, mock.as_ref(), "v1.0.0", no_sleep, &latch),
        )
        .await
        .expect("shutdown should finish after the active attempt")
        .unwrap();

        assert_eq!(outcome, Reconcile::Stopped);
        assert_eq!(1, mock.call_count(Call::IssueDeviceToken));
        assert_eq!(1, mock.num_get_device_calls());
        assert_eq!(1, mock.num_update_device_calls());
        let actual_version = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(Some("v1.0.0".to_string()), actual_version);
        let actual_device = files::read_json::<Device>(&layout.device()).await.unwrap();
        assert_eq!(expected_device, actual_device);
        let actual_settings = files::read_json::<Settings>(&layout.settings())
            .await
            .unwrap();
        assert_eq!(settings_before, actual_settings);
        assert_eq!(keys_before, read_keys(&layout).await);
        assert!(!layout.resources().exists());
        assert!(layout.events_dir().exists());
        assert!(!layout.events_log_file().exists());
    }

    #[tokio::test]
    async fn missing_key_returns_validation_error_with_pending_stop() {
        let (layout, _tmp) = prepare_layout("upgrade_missing_key").await;
        files::delete(&layout.auth().private_key()).await.unwrap();
        let mock = make_mock_client(backend_device("dvc_new", "new"));

        let result = reconcile(&layout, mock.as_ref(), "v1.0.0", no_sleep, &Latch::new()).await;

        assert!(matches!(
            result,
            Err(UpgradeErr::FileSysErr(FileSysErr::PathDoesNotExistErr(_)))
        ));
        assert!(mock.requests().is_empty());
    }
}

mod needs_upgrade {
    use super::*;

    #[tokio::test]
    async fn returns_true_when_marker_missing() {
        let (layout, _tmp) = prepare_layout("needs_upgrade_missing").await;
        assert!(needs_upgrade(&layout, "v1.0.0").await);
    }

    #[tokio::test]
    async fn returns_false_when_marker_matches() {
        let (layout, _tmp) = prepare_layout("needs_upgrade_match").await;
        disk::agent_version::write(&layout.agent_version(), "v1.2.3")
            .await
            .unwrap();
        assert!(!needs_upgrade(&layout, "v1.2.3").await);
    }

    #[tokio::test]
    async fn returns_true_when_marker_differs() {
        let (layout, _tmp) = prepare_layout("needs_upgrade_differs").await;
        disk::agent_version::write(&layout.agent_version(), "v1.0.0")
            .await
            .unwrap();
        assert!(needs_upgrade(&layout, "v2.0.0").await);
    }

    #[tokio::test]
    async fn returns_true_when_read_errors() {
        let (layout, _tmp) = prepare_layout("needs_upgrade_read_err").await;
        // Force a read error: create a directory at the marker path. `exists()`
        // returns true for a directory, so `read_string` runs and fails with a
        // FileSysErr; `needs_upgrade` treats the error as "missing" and returns true.
        dirs::create(&miru_agent::filesys::Dir::new(
            layout.agent_version().path().clone(),
        ))
        .await
        .unwrap();
        assert!(needs_upgrade(&layout, "v1.0.0").await);
    }
}

mod reconcile_impl {
    use super::*;

    #[tokio::test]
    async fn happy_path_writes_marker_and_updates_backend() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_happy").await;
        let mock = make_mock_client(backend_device("dvc_ri1", "happy"));

        let version = "v3.4.5";
        reconcile_impl(mock.as_ref(), &layout, version)
            .await
            .unwrap();

        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some(version.to_string()));
        assert_eq!(mock.num_update_device_calls(), 1);
        assert!(mock.num_get_device_calls() >= 1);
        assert!(mock.call_count(Call::IssueDeviceToken) >= 1);
    }

    #[tokio::test]
    async fn returns_authn_err_when_private_key_missing() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_no_pk").await;
        files::delete(&layout.auth().private_key()).await.unwrap();

        let mock = make_mock_client(backend_device("dvc_ri2", "no_pk"));
        let err = reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
            .await
            .expect_err("expected AuthnErr from missing private key");
        match err {
            UpgradeErr::AuthnErr(_) => {}
            other => panic!("expected UpgradeErr::AuthnErr, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn returns_http_err_when_get_device_fails() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_get_fail").await;
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
        let (layout, _tmp) = prepare_layout("reconcile_impl_reset_fail").await;
        // Place a directory at the device.json path so setup::reset's atomic
        // write of device.json cannot replace it and returns
        // DiskErr::FileSysErr(_).
        dirs::create(&miru_agent::filesys::Dir::new(
            layout.device().path().clone(),
        ))
        .await
        .unwrap();

        let mock = make_mock_client(backend_device("dvc_ri4", "reset_fail"));
        let err = reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
            .await
            .expect_err("expected DiskErr from reset failure");
        match err {
            UpgradeErr::DiskErr(_) => {}
            other => panic!("expected UpgradeErr::DiskErr, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn returns_http_err_when_update_device_fails() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_update_fail").await;
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
        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap();
        assert_eq!(marker, Some(version.to_string()));
    }

    #[tokio::test]
    async fn preserves_customized_settings() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_preserves_settings").await;

        let staging = Settings {
            backend: Backend {
                host: BackendHost::new("staging.api.mirurobotics.com").unwrap(),
            },
            mqtt_broker: MQTTBroker {
                host: MqttHost::new("staging.mqtt.mirurobotics.com").unwrap(),
            },
            ..Settings::default()
        };
        files::write_json(&layout.settings(), &staging, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let mock = make_mock_client(backend_device("dvc_ps1", "preserves"));
        reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
            .await
            .unwrap();

        let on_disk = files::read_json::<Settings>(&layout.settings())
            .await
            .unwrap();
        assert_eq!(
            on_disk.backend.host.as_str(),
            "staging.api.mirurobotics.com"
        );
        assert_eq!(
            on_disk.mqtt_broker.host.as_str(),
            "staging.mqtt.mirurobotics.com"
        );
    }

    #[tokio::test]
    async fn falls_back_to_defaults_when_settings_missing() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_settings_missing").await;

        let mock = make_mock_client(backend_device("dvc_sm1", "missing"));
        reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
            .await
            .unwrap();

        let on_disk = files::read_json::<Settings>(&layout.settings())
            .await
            .unwrap();
        assert_eq!(on_disk, Settings::default());
    }

    #[tokio::test]
    async fn falls_back_to_defaults_when_settings_corrupt() {
        let (layout, _tmp) = prepare_layout("reconcile_impl_settings_corrupt").await;

        test_files::seed(&layout.settings(), "not-json").await;

        let mock = make_mock_client(backend_device("dvc_sc1", "corrupt"));
        reconcile_impl(mock.as_ref(), &layout, "v1.0.0")
            .await
            .unwrap();

        let on_disk = files::read_json::<Settings>(&layout.settings())
            .await
            .unwrap();
        assert_eq!(on_disk, Settings::default());
    }
}
