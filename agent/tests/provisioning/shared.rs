// internal crates
use crate::mocks::http_client::MockClient;
use backend_api::models::Device;
use miru_agent::crypt::base64;
use miru_agent::filesys::{self, PathExt};
use miru_agent::http::{errors::MockErr, HTTPErr};
use miru_agent::provisioning::provision;
use miru_agent::storage::{Layout, Settings};

// external crates
use serde_json::json;

pub(super) const DEVICE_ID: &str = "75899aa4-b08a-4047-8526-880b1b832973";

pub(super) fn new_jwt(device_id: &str) -> String {
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

pub(super) fn new_device(id: &str, name: &str) -> Device {
    Device {
        id: id.to_string(),
        name: name.to_string(),
        session_id: "session-abc".to_string(),
        ..Device::default()
    }
}

/// Per-test fixture: a fresh temp-dir layout, default settings, and a JWT
/// provisioning token. Tests should call `env.cleanup().await` at the end
/// to remove the temp dir.
pub(super) struct Env {
    pub root: filesys::Dir,
    pub layout: Layout,
    pub settings: Settings,
    pub token: String,
}

impl Env {
    pub async fn new(prefix: &str) -> Self {
        let root = filesys::Dir::create_temp_dir(prefix).await.unwrap();
        let layout = Layout::new(root.clone());
        Self {
            root,
            layout,
            settings: Settings::default(),
            token: new_jwt(DEVICE_ID),
        }
    }

    /// Run a provision with a mock that returns `new_device(DEVICE_ID, name)`.
    /// Used as a setup helper to seed an already-provisioned state.
    pub async fn seed_provision(&self, name: &'static str) {
        provision::provision(
            &mock_ok_provision(name),
            &self.layout,
            &self.settings,
            &self.token,
            Some(name.to_string()),
        )
        .await
        .expect("seed provision must succeed");
    }

    pub async fn cleanup(self) {
        self.root.delete().await.unwrap();
    }
}

/// MockClient that succeeds with a `Device` named `name` for `/devices/provision`.
pub(super) fn mock_ok_provision(name: &'static str) -> MockClient {
    MockClient {
        provision_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, name))),
        ..MockClient::default()
    }
}

/// MockClient that returns a network-flagged HTTP error for `/devices/provision`.
pub(super) fn mock_failing_provision() -> MockClient {
    MockClient {
        provision_device_fn: Box::new(|| Err(HTTPErr::MockErr(MockErr { is_network_conn_err: true }))),
        ..MockClient::default()
    }
}

/// MockClient that succeeds with a `Device` named `name` for `/devices/reprovision`.
pub(super) fn mock_ok_reprovision(name: &'static str) -> MockClient {
    MockClient {
        reprovision_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, name))),
        ..MockClient::default()
    }
}

/// MockClient that returns a network-flagged HTTP error for `/devices/reprovision`.
pub(super) fn mock_failing_reprovision() -> MockClient {
    MockClient {
        reprovision_device_fn: Box::new(|| Err(HTTPErr::MockErr(MockErr { is_network_conn_err: true }))),
        ..MockClient::default()
    }
}

/// Asserts that a successful provision/reprovision left every persisted blob
/// in place — `device.json`, `settings.json`, both keys, the token, and
/// the temp dir is gone.
pub(super) async fn assert_storage_complete(layout: &Layout, expected_name: &str) {
    let device_file = layout.device();
    assert!(device_file.exists(), "device.json missing");
    let device_json: serde_json::Value =
        serde_json::from_str(&device_file.read_string().await.unwrap()).unwrap();
    assert_eq!(device_json["device_id"], DEVICE_ID);
    assert_eq!(device_json["name"], expected_name);

    assert!(layout.settings().exists(), "settings missing");

    let auth = layout.auth();
    assert!(auth.private_key().exists(), "private key missing");
    assert!(auth.public_key().exists(), "public key missing");
    assert!(auth.token().exists(), "token missing");

    assert!(!layout.temp_dir().exists(), "temp dir not cleaned");
}

/// Byte-exact snapshot of every persisted blob, used to verify a failing
/// provision/reprovision doesn't mutate on-disk state.
pub(super) struct StorageSnapshot {
    device: String,
    settings: String,
    private_key: String,
    public_key: String,
    token: String,
}

impl StorageSnapshot {
    pub async fn capture(layout: &Layout) -> Self {
        let auth = layout.auth();
        Self {
            device: layout.device().read_string().await.unwrap(),
            settings: layout.settings().read_string().await.unwrap(),
            private_key: auth.private_key().read_string().await.unwrap(),
            public_key: auth.public_key().read_string().await.unwrap(),
            token: auth.token().read_string().await.unwrap(),
        }
    }

    pub async fn assert_unchanged(&self, layout: &Layout) {
        let auth = layout.auth();
        assert_eq!(layout.device().read_string().await.unwrap(), self.device);
        assert_eq!(layout.settings().read_string().await.unwrap(), self.settings);
        assert_eq!(auth.private_key().read_string().await.unwrap(), self.private_key);
        assert_eq!(auth.public_key().read_string().await.unwrap(), self.public_key);
        assert_eq!(auth.token().read_string().await.unwrap(), self.token);
    }
}
