// standard crates
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// internal crates
use crate::mocks::http_client::MockClient;
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models::Device;
use miru_agent::crypt::base64;
use miru_agent::disk::{layout::INSTALLER_SENTINEL, Layout, Settings};
use miru_agent::filesys::{dirs, files, Dir, FileSysErr, PathExt, WriteOptions};
use miru_agent::http::{errors::MockErr, HTTPErr};
use miru_agent::provisioning::errors::InstallerLayoutErr;
use miru_agent::provisioning::{assert_installer_layout, provision, ProvisionErr};

// external crates
use serde_json::json;

pub(super) const DEVICE_ID: &str = "75899aa4-b08a-4047-8526-880b1b832973";
// Stands in for the Windows installer's sentinel folder in tmp\.
pub(super) const TEMP_SENTINEL: &str = INSTALLER_SENTINEL;

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
    /// Held only for RAII: the `TempDir` guard deletes the temp directory when
    /// the `Env` is dropped (or `cleanup` is called).
    _root: test_dirs::TempDir,
    pub layout: Layout,
    pub settings: Settings,
    pub token: String,
}

impl Env {
    pub async fn new(prefix: &str) -> Self {
        let root = test_dirs::temp(prefix).unwrap();
        let layout = Layout::new(root.to_dir());
        dirs::create_if_absent(&layout.temp_dir().subdir(TEMP_SENTINEL))
            .await
            .unwrap();
        Self {
            _root: root,
            layout,
            settings: Settings::default(),
            token: new_jwt(DEVICE_ID),
        }
    }

    /// Run a provision with a mock that returns `new_device(DEVICE_ID, name)`.
    /// Used as a setup helper to seed an already-provisioned state.
    pub async fn seed_provision(&self, name: impl Into<String>) {
        let name = name.into();
        provision::provision(
            &mock_ok_provision(name.clone()),
            &self.layout,
            &self.settings,
            &self.token,
            Some(name),
        )
        .await
        .expect("seed provision must succeed");
    }

    pub async fn cleanup(self) {
        // The `TempDir` guard deletes the directory when dropped here.
        drop(self);
    }
}

/// MockClient that succeeds with a `Device` named `name` for `/devices/provision`.
pub(super) fn mock_ok_provision(name: impl Into<String>) -> MockClient {
    let name = name.into();
    MockClient {
        provision_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, &name))),
        ..MockClient::default()
    }
}

/// MockClient that returns a network-flagged HTTP error for `/devices/provision`.
pub(super) fn mock_failing_provision() -> MockClient {
    MockClient {
        provision_device_fn: Box::new(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        }),
        ..MockClient::default()
    }
}

/// MockClient that succeeds with a `Device` named `name` for `/devices/reprovision`.
pub(super) fn mock_ok_reprovision(name: impl Into<String>) -> MockClient {
    let name = name.into();
    MockClient {
        reprovision_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, &name))),
        ..MockClient::default()
    }
}

/// MockClient that returns a network-flagged HTTP error for `/devices/reprovision`.
pub(super) fn mock_failing_reprovision() -> MockClient {
    MockClient {
        reprovision_device_fn: Box::new(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        }),
        ..MockClient::default()
    }
}

/// Asserts that a successful provision/reprovision persisted every required
/// blob (`device.json`, `settings.json`, both keys, the token), that the
/// device record carries `DEVICE_ID` and `expected_name`, and that the temp
/// dir was cleaned up.
pub(super) async fn validate_storage(layout: &Layout, expected_name: &str) {
    let device_file = layout.device();
    assert!(device_file.exists(), "device.json missing");
    let device_json: serde_json::Value =
        serde_json::from_str(&files::read_string(&device_file).await.unwrap()).unwrap();
    assert_eq!(device_json["device_id"], DEVICE_ID);
    assert_eq!(device_json["name"], expected_name);

    assert!(layout.settings().exists(), "settings missing");

    let auth = layout.auth();
    assert!(auth.private_key().exists(), "private key missing");
    assert!(auth.public_key().exists(), "public key missing");
    assert!(auth.token().exists(), "token missing");

    assert_temp_dir_cleaned(layout).await;
}

pub(super) async fn assert_temp_dir_cleaned(layout: &Layout) {
    let temp_dir = layout.temp_dir();
    let files = dirs::files(&temp_dir).await.unwrap();
    assert!(files.is_empty(), "temp dir still contains files: {files:?}");

    let subdirs: Vec<String> = dirs::subdirs(&temp_dir)
        .await
        .unwrap()
        .iter()
        .map(|dir| dir.name().unwrap().to_string())
        .collect();
    assert_eq!(subdirs, vec![TEMP_SENTINEL.to_string()]);
}

/// Byte-exact snapshot of every persisted blob, used to verify a failing
/// provision/reprovision doesn't mutate on-disk state.
pub(super) struct StorageSnapshot {
    device: Option<String>,
    settings: Option<String>,
    private_key: Option<String>,
    public_key: Option<String>,
    token: Option<String>,
}

impl StorageSnapshot {
    pub async fn capture(layout: &Layout) -> Self {
        let auth = layout.auth();
        Self {
            device: files::read_string(&layout.device()).await.ok(),
            settings: files::read_string(&layout.settings()).await.ok(),
            private_key: files::read_string(&auth.private_key()).await.ok(),
            public_key: files::read_string(&auth.public_key()).await.ok(),
            token: files::read_string(&auth.token()).await.ok(),
        }
    }

    pub async fn assert_unchanged(&self, layout: &Layout) {
        let auth = layout.auth();
        assert_eq!(files::read_string(&layout.device()).await.ok(), self.device);
        assert_eq!(
            files::read_string(&layout.settings()).await.ok(),
            self.settings
        );
        assert_eq!(
            files::read_string(&auth.private_key()).await.ok(),
            self.private_key
        );
        assert_eq!(
            files::read_string(&auth.public_key()).await.ok(),
            self.public_key
        );
        assert_eq!(files::read_string(&auth.token()).await.ok(), self.token);
    }
}

pub mod assert_installer_layout {
    use super::*;

    fn new_layout() -> (test_dirs::TempDir, Layout) {
        let tmp = test_dirs::temp("installer-layout").unwrap();
        let layout = Layout::new(tmp.to_dir());
        (tmp, layout)
    }

    async fn create_sentinels(layout: &Layout) {
        for sentinel in layout.installer_sentinels() {
            dirs::create_if_absent(&sentinel).await.unwrap();
        }
    }

    fn expect_layout_err(result: Result<(), ProvisionErr>) -> InstallerLayoutErr {
        match result {
            Err(ProvisionErr::InstallerLayoutErr(e)) => e,
            other => panic!("expected InstallerLayoutErr, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ok_when_both_sentinels_are_dirs() {
        let (_tmp, layout) = new_layout();
        create_sentinels(&layout).await;

        assert_installer_layout(&layout).unwrap();
    }

    #[test]
    fn missing_layout_is_rejected_and_nothing_is_created() {
        let (_tmp, layout) = new_layout();
        let [auth_sentinel, _] = layout.installer_sentinels();

        let err = expect_layout_err(assert_installer_layout(&layout));

        assert_eq!(&err.missing, auth_sentinel.path());
        let msg = err.to_string();
        assert!(
            msg.contains("not created by the installer"),
            "message: {msg}"
        );
        assert!(msg.contains("msiexec"), "message: {msg}");
        assert!(
            msg.contains(&auth_sentinel.path().display().to_string()),
            "message: {msg}"
        );
        // the guard only reads metadata; it must not create the state dir
        assert!(!layout.root().exists());
    }

    #[tokio::test]
    async fn missing_tmp_sentinel_is_named() {
        let (_tmp, layout) = new_layout();
        let [auth_sentinel, tmp_sentinel] = layout.installer_sentinels();
        dirs::create_if_absent(&auth_sentinel).await.unwrap();

        let err = expect_layout_err(assert_installer_layout(&layout));

        assert_eq!(&err.missing, tmp_sentinel.path());
    }

    #[tokio::test]
    async fn sentinel_that_is_a_file_is_rejected() {
        let (_tmp, layout) = new_layout();
        let [auth_sentinel, tmp_sentinel] = layout.installer_sentinels();
        dirs::create_if_absent(&auth_sentinel).await.unwrap();
        let file = layout.temp_dir().file(INSTALLER_SENTINEL);
        files::write_string(&file, "not a dir", WriteOptions::default())
            .await
            .unwrap();

        let err = expect_layout_err(assert_installer_layout(&layout));

        assert_eq!(&err.missing, tmp_sentinel.path());
    }

    #[tokio::test]
    async fn parent_that_is_a_file_is_treated_as_missing() {
        let (_tmp, layout) = new_layout();
        let [auth_sentinel, _] = layout.installer_sentinels();
        // `auth` is a regular file, so the sentinel lookup fails with
        // ENOTDIR on Unix and NotFound on Windows; both mean "missing"
        let auth_file = layout.root().file("auth");
        files::write_string(&auth_file, "not a dir", WriteOptions::default())
            .await
            .unwrap();

        let err = expect_layout_err(assert_installer_layout(&layout));

        assert_eq!(&err.missing, auth_sentinel.path());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_sentinel_is_rejected() {
        let (tmp, layout) = new_layout();
        let [auth_sentinel, tmp_sentinel] = layout.installer_sentinels();
        dirs::create_if_absent(&auth_sentinel).await.unwrap();
        dirs::create_if_absent(&layout.temp_dir()).await.unwrap();
        let target = tmp.subdir("target");
        dirs::create_if_absent(&target).await.unwrap();
        std::os::unix::fs::symlink(target.path(), tmp_sentinel.path()).unwrap();

        let err = expect_layout_err(assert_installer_layout(&layout));

        assert_eq!(&err.missing, tmp_sentinel.path());
    }

    #[test]
    fn unreadable_sentinel_path_returns_dir_metadata_err() {
        let layout = Layout::new(Dir::new("invalid\0path"));
        let [auth_sentinel, _] = layout.installer_sentinels();

        let result = assert_installer_layout(&layout);

        match result {
            Err(ProvisionErr::FileSysErr(FileSysErr::DirMetadataErr(e))) => {
                assert_eq!(e.source.kind(), std::io::ErrorKind::InvalidInput);
                assert_eq!(e.dir.path(), auth_sentinel.path());
            }
            other => panic!("expected DirMetadataErr, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn permission_denied_returns_dir_metadata_err() {
        let (_tmp, layout) = new_layout();
        create_sentinels(&layout).await;
        let [auth_sentinel, _] = layout.installer_sentinels();
        let auth_root = layout.auth().root;

        // without search permission on `auth`, the sentinel's metadata is unreadable
        dirs::set_permissions(&auth_root, std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();
        let result = assert_installer_layout(&layout);
        dirs::set_permissions(&auth_root, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        match result {
            Err(ProvisionErr::FileSysErr(FileSysErr::DirMetadataErr(e))) => {
                assert_eq!(e.source.kind(), std::io::ErrorKind::PermissionDenied);
                assert_eq!(e.dir.path(), auth_sentinel.path());
                let msg = e.to_string();
                assert!(msg.contains("os error 13"), "message: {msg}");
            }
            other => panic!("expected DirMetadataErr, got {other:?}"),
        }
    }
}
