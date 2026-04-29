// internal crates
use miru_agent::authn;
use miru_agent::filesys::{self, PathExt, WriteOptions};
use miru_agent::models::Device;
use miru_agent::storage::{self, Layout, Settings};

pub mod bootstrap {
    use super::*;

    const AGENT_VERSION: &str = "v0.0.0";

    async fn validate_storage(layout: &Layout) {
        // agent file
        let device_file = layout.device();
        let device_file_content = device_file.read_json::<Device>().await.unwrap();
        assert_eq!(device_file_content, Device::default());

        // settings file
        let settings_file = layout.settings();
        let settings_file_content = settings_file.read_json::<Settings>().await.unwrap();
        assert_eq!(settings_file_content, Settings::default());

        // token file
        let auth_layout = layout.auth();
        let token_file = auth_layout.token();
        assert!(token_file.exists());

        // private key file
        let private_key_file = auth_layout.private_key();
        assert!(private_key_file.exists());
        let private_key_contents = private_key_file.read_string().await.unwrap();
        assert!(!private_key_contents.is_empty());

        // public key file
        let public_key_file = auth_layout.public_key();
        assert!(public_key_file.exists());
        let public_key_contents = public_key_file.read_string().await.unwrap();
        assert!(!public_key_contents.is_empty());

        // events directory
        let events_dir = layout.events_dir();
        assert!(events_dir.exists());

        // marker file
        let marker = storage::agent_version::read(&layout.agent_version())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker, AGENT_VERSION);
    }

    async fn create_temp_key_files(layout: &Layout) -> (filesys::File, filesys::File) {
        let temp_dir = layout.temp_dir();
        let private_key_file = temp_dir.file("private_key.pem");
        private_key_file
            .write_string("test", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let public_key_file = temp_dir.file("public_key.pem");
        public_key_file
            .write_string("test", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        (private_key_file, public_key_file)
    }

    #[tokio::test]
    async fn src_public_key_file_doesnt_exist() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;
        public_key_file.delete().await.unwrap();

        // setup the storage
        let device = Device::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap_err();
    }

    #[tokio::test]
    async fn src_private_key_file_doesnt_exist() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;
        private_key_file.delete().await.unwrap();

        // setup the storage
        let device = Device::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap_err();
    }

    #[tokio::test]
    async fn clean_install() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // setup the storage
        let device = Device::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn device_file_already_exists() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the agent file
        let device_file = layout.device();
        device_file
            .write_json(&Device::default(), WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // setup the storage
        let device = Device::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn auth_directory_already_exists() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the auth directory
        let auth_dir = layout.auth();
        auth_dir.root.create().await.unwrap();

        // setup the storage
        let device = Device::default();
        let settings = Settings::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn private_key_file_already_exists() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // setup the storage
        let device = Device::default();
        let settings = Settings::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn public_key_file_already_exists() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // setup the storage
        let device = Device::default();
        let settings = Settings::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn storage_directory_already_exists() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the storage directory
        let resources_dir = layout.resources();
        let subfile = resources_dir.file("test");
        subfile
            .write_string("test", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(subfile.exists());

        // setup the storage
        let device = Device::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            "v0.0.0",
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;

        // subfile should be deleted
        assert!(!subfile.exists());
    }

    #[tokio::test]
    async fn events_directory_already_exists() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the events directory with a stale log file
        let events_dir = layout.events_dir();
        let subfile = events_dir.file("events.jsonl");
        subfile
            .write_string("{\"id\":1}\n", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(subfile.exists());

        // setup the storage
        let device = Device::default();
        storage::setup::bootstrap(
            &layout,
            &device,
            &settings,
            &private_key_file,
            &public_key_file,
            AGENT_VERSION,
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;

        // stale events file should be deleted
        assert!(!subfile.exists());
    }
}

pub mod reset {
    use super::*;

    const PRIVATE_KEY_CONTENTS: &str = "private-key-contents";
    const PUBLIC_KEY_CONTENTS: &str = "public-key-contents";

    async fn write_existing_keys(layout: &Layout) {
        let auth_dir = layout.auth();
        auth_dir.root.create_if_absent().await.unwrap();
        auth_dir
            .private_key()
            .write_string(PRIVATE_KEY_CONTENTS, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        auth_dir
            .public_key()
            .write_string(PUBLIC_KEY_CONTENTS, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }

    async fn assert_keys_preserved(layout: &Layout) {
        let auth_dir = layout.auth();
        let private_key = auth_dir
            .private_key()
            .read_string()
            .await
            .expect("private key should still exist after reset");
        let public_key = auth_dir
            .public_key()
            .read_string()
            .await
            .expect("public key should still exist after reset");
        assert_eq!(private_key, PRIVATE_KEY_CONTENTS);
        assert_eq!(public_key, PUBLIC_KEY_CONTENTS);
    }

    async fn assert_marker(layout: &Layout, expected_version: &str) {
        let marker = storage::agent_version::read(&layout.agent_version())
            .await
            .expect("marker read should succeed")
            .expect("marker should exist after reset");
        assert_eq!(marker, expected_version);
    }

    async fn assert_default_token(layout: &Layout) {
        let token = layout
            .auth()
            .token()
            .read_json::<authn::Token>()
            .await
            .unwrap();
        assert_eq!(token, authn::Token::default());
    }

    #[tokio::test]
    async fn preserves_keys_and_writes_marker() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        write_existing_keys(&layout).await;

        // pre-write a stale device file with arbitrary content
        layout
            .device()
            .write_string("{\"some\":\"stale\"}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let device = Device::default();
        let settings = Settings::default();
        storage::setup::reset(&layout, &device, &settings, "v9.9.9")
            .await
            .unwrap();

        assert_keys_preserved(&layout).await;

        // device + settings written from inputs
        let on_disk_device = layout.device().read_json::<Device>().await.unwrap();
        assert_eq!(on_disk_device, device);
        let on_disk_settings = layout.settings().read_json::<Settings>().await.unwrap();
        assert_eq!(on_disk_settings, settings);

        assert_default_token(&layout).await;
        assert_marker(&layout, "v9.9.9").await;

        // resources/ wiped
        assert!(!layout.resources().exists());
        // events/ recreated empty
        assert!(layout.events_dir().exists());
    }

    #[tokio::test]
    async fn wipes_resources_subtree() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        write_existing_keys(&layout).await;

        // pre-create something under resources/config_instances/contents/
        let stale = layout.config_instance_content().file("stale.json");
        stale
            .write_string("{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(stale.exists());

        storage::setup::reset(&layout, &Device::default(), &Settings::default(), "v1.0.0")
            .await
            .unwrap();

        assert!(!stale.exists());
        assert!(!layout.resources().exists());
    }

    #[tokio::test]
    async fn wipes_events_subtree() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        write_existing_keys(&layout).await;

        // pre-create something under events/
        let stale = layout.events_dir().file("events.jsonl");
        stale
            .write_string("{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(stale.exists());

        storage::setup::reset(&layout, &Device::default(), &Settings::default(), "v1.0.0")
            .await
            .unwrap();

        assert!(!stale.exists());
        assert!(layout.events_dir().exists());
        assert!(!layout.events_dir().file("events.jsonl").exists());
    }

    #[tokio::test]
    async fn no_prior_state() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        storage::setup::reset(&layout, &Device::default(), &Settings::default(), "v0.1.0")
            .await
            .unwrap();

        // device + settings + token + marker written; events dir created
        assert!(layout.device().exists());
        assert!(layout.settings().exists());
        assert!(layout.auth().token().exists());
        assert_marker(&layout, "v0.1.0").await;
        assert!(layout.events_dir().exists());
    }

    #[tokio::test]
    async fn overwrites_existing_marker() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        write_existing_keys(&layout).await;

        // pre-write an old marker
        let layout_root = layout.root();
        layout_root.create_if_absent().await.unwrap();
        storage::agent_version::write(&layout.agent_version(), "v0.0.1")
            .await
            .unwrap();

        storage::setup::reset(&layout, &Device::default(), &Settings::default(), "v0.0.2")
            .await
            .unwrap();

        assert_marker(&layout, "v0.0.2").await;
    }
}
