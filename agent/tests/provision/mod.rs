// standard crates
use std::env;
use std::sync::{Mutex, OnceLock};

// internal crates
use crate::mocks::{
    http_client::{Call, MockClient},
    systemctl::MockSystemctl,
};
use backend_api::models::{Device, ErrorResponse, TokenResponse};
use miru_agent::crypt::base64;
use miru_agent::filesys::{self, PathExt};
use miru_agent::http::errors::{MockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::http::HTTPErr;
use miru_agent::installer::provision::{self, ProvisionErr, SystemdErr};
use miru_agent::storage::{Layout, Settings};

// external crates
use serde_json::json;

const DEVICE_ID: &str = "75899aa4-b08a-4047-8526-880b1b832973";

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock should not be poisoned")
}

fn new_jwt(device_id: &str) -> String {
    let payload = json!({
        "iss": "miru",
        "aud": "device",
        "exp": 9999999999_i64,
        "iat": 1700000000_i64,
        "sub": device_id
    })
    .to_string();
    format!(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.fakesig",
        base64::encode_string_url_safe_no_pad(&payload)
    )
}

fn new_device(id: &str, name: &str) -> Device {
    Device {
        id: id.to_string(),
        name: name.to_string(),
        session_id: "session-abc".to_string(),
        ..Device::default()
    }
}

fn build_clients() -> (MockClient, MockClient) {
    (MockClient::default(), MockClient::default())
}

fn conflict_err() -> HTTPErr {
    HTTPErr::RequestFailed(RequestFailed {
        request: HttpParams::post("http://mock/devices", String::new())
            .meta()
            .unwrap(),
        status: reqwest::StatusCode::CONFLICT,
        error: None,
        trace: miru_agent::trace!(),
    })
}

fn server_err() -> HTTPErr {
    HTTPErr::RequestFailed(RequestFailed {
        request: HttpParams::post("http://mock/devices", String::new())
            .meta()
            .unwrap(),
        status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        error: None,
        trace: miru_agent::trace!(),
    })
}

fn device_is_active_err() -> HTTPErr {
    let body = ErrorResponse::new(backend_api::models::Error::new(
        "device_is_active".to_string(),
        std::collections::HashMap::new(),
        "device is already active".to_string(),
    ));
    HTTPErr::RequestFailed(RequestFailed {
        request: HttpParams::post("http://mock/devices/x/activation_token", String::new())
            .meta()
            .unwrap(),
        status: reqwest::StatusCode::BAD_REQUEST,
        error: Some(body),
        trace: miru_agent::trace!(),
    })
}

pub mod provision_fn {
    use super::*;

    #[tokio::test]
    async fn happy_path_completes_install() {
        let device_name = "test-device";
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        let token_for_mock = token.clone();
        public_client.set_issue_activation_token(move || {
            Ok(TokenResponse {
                token: token_for_mock.clone(),
                ..TokenResponse::default()
            })
        });

        // agent client returns the activated device
        let activated_name = device_name.to_string();
        let activated_client = MockClient {
            activate_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, &activated_name))),
            ..agent_client
        };

        let systemctl = MockSystemctl::default();
        let device = provision::provision(
            &public_client,
            &activated_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            device_name,
            false,
        )
        .await
        .expect("provision should succeed");

        assert_eq!(device.id, DEVICE_ID);
        assert_eq!(device.name, device_name);

        // public-API mock saw a device-create POST and an activation-token POST
        assert_eq!(public_client.call_count(Call::CreateDevice), 1);
        assert_eq!(public_client.call_count(Call::IssueActivationToken), 1);

        // agent client saw the activation call
        assert_eq!(activated_client.call_count(Call::ActivateDevice), 1);

        // /srv/miru (test layout) is populated
        let device_file = layout.device();
        assert!(device_file.exists(), "device.json missing");
        let auth_layout = layout.auth();
        assert!(auth_layout.private_key().exists(), "private key missing");
        assert!(auth_layout.public_key().exists(), "public key missing");
        assert!(auth_layout.token().exists(), "token missing");

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn conflict_falls_back_to_get_with_name_query() {
        let device_name = "test-device";
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, _agent_client) = build_clients();

        // POST returns 409, the GET fallback returns the device wrapped in a list.
        public_client.set_create_or_fetch_device(|| Err(conflict_err()));
        public_client.set_fetch_devices_by_name(|| Ok(vec![new_device(DEVICE_ID, "test-device")]));
        let token_for_mock = token.clone();
        public_client.set_issue_activation_token(move || {
            Ok(TokenResponse {
                token: token_for_mock.clone(),
                ..TokenResponse::default()
            })
        });

        let activated_name = device_name.to_string();
        let agent_client = MockClient {
            activate_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, &activated_name))),
            ..MockClient::default()
        };

        let systemctl = MockSystemctl::default();
        let device = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            device_name,
            false,
        )
        .await
        .expect("provision should succeed via GET fallback");
        assert_eq!(device.id, DEVICE_ID);

        // POST + GET on the public API
        assert_eq!(public_client.call_count(Call::CreateDevice), 1);
        assert_eq!(public_client.call_count(Call::FetchDeviceByName), 1);
        let requests = public_client.requests();
        let get_request = requests
            .iter()
            .find(|r| r.call == Call::FetchDeviceByName)
            .expect("expected a GET fallback request");
        assert_eq!(get_request.query, vec![("name".into(), device_name.into())]);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn backend_5xx_on_post_returns_backend_err() {
        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Err(server_err()));

        let systemctl = MockSystemctl::default();
        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            "test-device",
            false,
        )
        .await;

        match result {
            Err(ProvisionErr::BackendErr(HTTPErr::RequestFailed(rf))) => {
                assert_eq!(rf.status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
            }
            other => panic!("expected BackendErr(RequestFailed(500)), got {other:?}"),
        }
        // We never proceeded to the activation-token call.
        assert_eq!(public_client.call_count(Call::IssueActivationToken), 0);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn device_is_active_returns_reactivation_not_allowed_err() {
        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        public_client.set_issue_activation_token(|| Err(device_is_active_err()));

        let systemctl = MockSystemctl::default();
        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            "test-device",
            false,
        )
        .await;

        match result {
            Err(ProvisionErr::ReactivationNotAllowedErr(e)) => {
                assert_eq!(e.device_id, DEVICE_ID);
            }
            other => panic!("expected ReactivationNotAllowedErr, got {other:?}"),
        }
        // We never proceeded to the activation call.
        assert_eq!(agent_client.call_count(Call::ActivateDevice), 0);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn token_issue_request_failed_without_device_is_active_returns_backend_err() {
        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        // RequestFailed with no error body — exercises the BackendErr re-wrap branch.
        public_client.set_issue_activation_token(|| Err(server_err()));

        let systemctl = MockSystemctl::default();
        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            "test-device",
            false,
        )
        .await;

        match result {
            Err(ProvisionErr::BackendErr(HTTPErr::RequestFailed(_))) => {}
            other => panic!("expected BackendErr(RequestFailed), got {other:?}"),
        }
        assert_eq!(agent_client.call_count(Call::ActivateDevice), 0);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn token_issue_non_request_failed_http_err_returns_backend_err() {
        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        public_client.set_issue_activation_token(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        });

        let systemctl = MockSystemctl::default();
        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            "test-device",
            false,
        )
        .await;

        match result {
            Err(ProvisionErr::BackendErr(HTTPErr::MockErr(_))) => {}
            other => panic!("expected BackendErr(MockErr), got {other:?}"),
        }
        assert_eq!(agent_client.call_count(Call::ActivateDevice), 0);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn install_failure_returns_install_err() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, _agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        let token_for_mock = token.clone();
        public_client.set_issue_activation_token(move || {
            Ok(TokenResponse {
                token: token_for_mock.clone(),
                ..TokenResponse::default()
            })
        });

        // Force install() to fail by making the agent's activate call error out.
        let agent_client = MockClient {
            activate_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            }),
            ..MockClient::default()
        };

        let systemctl = MockSystemctl::default();
        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            "test-device",
            false,
        )
        .await;

        assert!(
            matches!(result, Err(ProvisionErr::InstallErr(_))),
            "expected ProvisionErr::InstallErr, got: {:?}",
            result
        );

        root.delete().await.unwrap();
    }
}

pub mod read_api_key_from_env {
    use super::*;

    #[test]
    fn returns_api_key_when_set() {
        let _env_lock = lock_env();
        env::set_var("MIRU_API_KEY", "test-api-key-123");
        let result = provision::read_api_key_from_env();
        assert_eq!(result.unwrap(), "test-api-key-123");
        env::remove_var("MIRU_API_KEY");
    }

    #[test]
    fn returns_error_when_not_set() {
        let _env_lock = lock_env();
        env::remove_var("MIRU_API_KEY");
        let result = provision::read_api_key_from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ProvisionErr::MissingApiKeyErr(ref e) if e.name == "MIRU_API_KEY"),
            "expected MissingApiKeyErr, got: {err:?}"
        );
    }
}

pub mod assert_root {
    use super::*;
    use miru_agent::installer::provision::test_support;

    /// Guard that resets the fake euid on drop so a panicking test cannot
    /// leak state into sibling tests on the same thread.
    struct FakeEuidGuard;

    impl Drop for FakeEuidGuard {
        fn drop(&mut self) {
            test_support::set_fake_euid(None);
        }
    }

    fn set_fake_euid(euid: u32) -> FakeEuidGuard {
        test_support::set_fake_euid(Some(euid));
        FakeEuidGuard
    }

    #[test]
    fn returns_ok_when_euid_is_zero() {
        let _g = set_fake_euid(0);
        provision::assert_root().expect("euid 0 should be accepted as root");
    }

    #[test]
    fn returns_not_root_err_when_euid_is_nonzero() {
        let _g = set_fake_euid(1000);
        let err = provision::assert_root().expect_err("non-zero euid must fail");
        assert!(
            matches!(err, ProvisionErr::NotRootErr(_)),
            "expected NotRootErr, got: {err:?}"
        );
    }
}

pub mod systemd_lifecycle {
    use super::*;
    use crate::mocks::systemctl::SystemctlCall;

    fn systemd_err(msg: &str) -> SystemdErr {
        SystemdErr {
            msg: msg.to_string(),
            trace: miru_agent::trace!(),
        }
    }

    #[tokio::test]
    async fn happy_path_records_stop_then_restart() {
        let device_name = "test-device";
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, _agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        let token_for_mock = token.clone();
        public_client.set_issue_activation_token(move || {
            Ok(TokenResponse {
                token: token_for_mock.clone(),
                ..TokenResponse::default()
            })
        });

        let activated_name = device_name.to_string();
        let agent_client = MockClient {
            activate_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, &activated_name))),
            ..MockClient::default()
        };

        let systemctl = MockSystemctl::default();
        let device = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            device_name,
            false,
        )
        .await
        .expect("provision should succeed");
        assert_eq!(device.id, DEVICE_ID);

        // stop must precede restart, and both target the `miru` unit
        let calls = systemctl.calls();
        assert_eq!(
            calls,
            vec![
                SystemctlCall {
                    verb: "stop".to_string(),
                    unit: "miru".to_string(),
                },
                SystemctlCall {
                    verb: "restart".to_string(),
                    unit: "miru".to_string(),
                },
            ]
        );

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn stop_failure_skips_install_and_returns_systemd_err() {
        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, _agent_client) = build_clients();
        // If we ever reach the public-API call this assertion in the mock
        // would still fire, but we cross-check via call_count below.
        public_client.set_create_or_fetch_device(|| {
            panic!("create_or_fetch_device must not be called when stop fails");
        });

        let agent_client = MockClient {
            activate_device_fn: Box::new(|| {
                panic!("agent activate must not be called when stop fails");
            }),
            ..MockClient::default()
        };

        let systemctl = MockSystemctl::default();
        systemctl.set_stop(|_unit| Err(systemd_err("stop boom")));

        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            "test-device",
            false,
        )
        .await;

        match result {
            Err(ProvisionErr::SystemdErr(e)) => assert_eq!(e.msg, "stop boom"),
            other => panic!("expected SystemdErr, got {other:?}"),
        }

        // Neither HTTP path was exercised.
        assert_eq!(public_client.call_count(Call::CreateDevice), 0);
        assert_eq!(public_client.call_count(Call::IssueActivationToken), 0);
        assert_eq!(agent_client.call_count(Call::ActivateDevice), 0);

        // Only the stop call was attempted; restart never ran.
        let calls = systemctl.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].verb, "stop");

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn restart_failure_after_install_returns_systemd_err() {
        let device_name = "test-device";
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let (public_client, _agent_client) = build_clients();
        public_client.set_create_or_fetch_device(|| Ok(new_device(DEVICE_ID, "test-device")));
        let token_for_mock = token.clone();
        public_client.set_issue_activation_token(move || {
            Ok(TokenResponse {
                token: token_for_mock.clone(),
                ..TokenResponse::default()
            })
        });

        let activated_name = device_name.to_string();
        let agent_client = MockClient {
            activate_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, &activated_name))),
            ..MockClient::default()
        };

        let systemctl = MockSystemctl::default();
        systemctl.set_restart(|_unit| Err(systemd_err("restart boom")));

        let result = provision::provision(
            &public_client,
            &agent_client,
            &systemctl,
            &layout,
            &settings,
            "secret-key",
            device_name,
            false,
        )
        .await;

        match result {
            Err(ProvisionErr::SystemdErr(e)) => assert_eq!(e.msg, "restart boom"),
            other => panic!("expected SystemdErr from restart, got {other:?}"),
        }

        // Install ran end-to-end before the restart failure.
        assert_eq!(agent_client.call_count(Call::ActivateDevice), 1);
        let device_file = layout.device();
        assert!(
            device_file.exists(),
            "device.json should be written before restart failure"
        );

        let calls = systemctl.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].verb, "stop");
        assert_eq!(calls[1].verb, "restart");

        root.delete().await.unwrap();
    }
}

pub mod error_conversion {
    use super::*;
    use miru_agent::installer::errors::{InstallErr, MissingEnvVarErr};

    #[test]
    fn install_err_into_provision_err() {
        let install_err = InstallErr::MissingEnvVarErr(MissingEnvVarErr {
            name: "MIRU_ACTIVATION_TOKEN".to_string(),
            trace: miru_agent::trace!(),
        });
        let provision_err: ProvisionErr = install_err.into();
        assert!(matches!(provision_err, ProvisionErr::InstallErr(_)));
    }
}
