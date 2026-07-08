// internal crates
use miru_agent::authn;
use miru_agent::disk::{self, Layout, Settings};
use miru_agent::filesys::{self, dirs, files, PathExt, WriteOptions};
use miru_agent::models::Device;

pub mod bootstrap {
    use super::*;

    const AGENT_VERSION: &str = "v0.0.0";

    async fn validate_storage(layout: &Layout) {
        // agent file
        let device_file = layout.device();
        let device_file_content = files::read_json::<Device>(&device_file).await.unwrap();
        assert_eq!(device_file_content, Device::default());

        // settings file
        let settings_file = layout.settings();
        let settings_file_content = files::read_json::<Settings>(&settings_file).await.unwrap();
        assert_eq!(settings_file_content, Settings::default());

        // token file
        let auth_layout = layout.auth();
        let token_file = auth_layout.token();
        assert!(token_file.exists());

        // private key file
        let private_key_file = auth_layout.private_key();
        assert!(private_key_file.exists());
        let private_key_contents = files::read_string(&private_key_file).await.unwrap();
        assert!(!private_key_contents.is_empty());

        // public key file
        let public_key_file = auth_layout.public_key();
        assert!(public_key_file.exists());
        let public_key_contents = files::read_string(&public_key_file).await.unwrap();
        assert!(!public_key_contents.is_empty());

        // events directory
        let events_dir = layout.events_dir();
        assert!(events_dir.exists());

        // marker file
        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker, AGENT_VERSION);
    }

    async fn create_temp_key_files(layout: &Layout) -> (filesys::File, filesys::File) {
        let temp_dir = layout.temp_dir();
        let private_key_file = temp_dir.file("private_key.pem");
        files::write_string(&private_key_file, "test", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let public_key_file = temp_dir.file("public_key.pem");
        files::write_string(&public_key_file, "test", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        (private_key_file, public_key_file)
    }

    #[tokio::test]
    async fn src_public_key_file_doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;
        files::delete(&public_key_file).await.unwrap();

        // setup the storage
        let device = Device::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;
        files::delete(&private_key_file).await.unwrap();

        // setup the storage
        let device = Device::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // setup the storage
        let device = Device::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the agent file
        let device_file = layout.device();
        files::write_json(
            &device_file,
            &Device::default(),
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        // setup the storage
        let device = Device::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the auth directory
        let auth_dir = layout.auth();
        dirs::create(&auth_dir.root).await.unwrap();

        // setup the storage
        let device = Device::default();
        let settings = Settings::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // setup the storage
        let device = Device::default();
        let settings = Settings::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // setup the storage
        let device = Device::default();
        let settings = Settings::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the storage directory
        let resources_dir = layout.resources();
        let subfile = resources_dir.file("test");
        files::write_string(&subfile, "test", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(subfile.exists());

        // setup the storage
        let device = Device::default();
        disk::setup::bootstrap(
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the events directory with a stale log file
        let events_dir = layout.events_dir();
        let subfile = events_dir.file("events.jsonl");
        files::write_string(&subfile, "{\"id\":1}\n", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(subfile.exists());

        // setup the storage
        let device = Device::default();
        disk::setup::bootstrap(
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
        dirs::create_if_absent(&auth_dir.root).await.unwrap();
        files::write_string(
            &auth_dir.private_key(),
            PRIVATE_KEY_CONTENTS,
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
        files::write_string(
            &auth_dir.public_key(),
            PUBLIC_KEY_CONTENTS,
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
    }

    async fn assert_keys_preserved(layout: &Layout) {
        let auth_dir = layout.auth();
        let private_key = files::read_string(&auth_dir.private_key())
            .await
            .expect("private key should still exist after reset");
        let public_key = files::read_string(&auth_dir.public_key())
            .await
            .expect("public key should still exist after reset");
        assert_eq!(private_key, PRIVATE_KEY_CONTENTS);
        assert_eq!(public_key, PUBLIC_KEY_CONTENTS);
    }

    async fn assert_marker(layout: &Layout, expected_version: &str) {
        let marker = disk::agent_version::read(&layout.agent_version())
            .await
            .expect("marker read should succeed")
            .expect("marker should exist after reset");
        assert_eq!(marker, expected_version);
    }

    async fn assert_default_token(layout: &Layout) {
        let token = files::read_json::<authn::Token>(&layout.auth().token())
            .await
            .unwrap();
        assert_eq!(token, authn::Token::default());
    }

    #[tokio::test]
    async fn preserves_keys_and_writes_marker() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        write_existing_keys(&layout).await;

        // pre-write a stale device file with arbitrary content
        files::write_string(
            &layout.device(),
            "{\"some\":\"stale\"}",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let device = Device::default();
        let settings = Settings::default();
        disk::setup::reset(&layout, &device, &settings, "v9.9.9")
            .await
            .unwrap();

        assert_keys_preserved(&layout).await;

        // device + settings written from inputs
        let on_disk_device = files::read_json::<Device>(&layout.device()).await.unwrap();
        assert_eq!(on_disk_device, device);
        let on_disk_settings = files::read_json::<Settings>(&layout.settings())
            .await
            .unwrap();
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        write_existing_keys(&layout).await;

        // pre-create something under resources/config_instances/contents/
        let stale = layout.config_instance_content().file("stale.json");
        files::write_string(&stale, "{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(stale.exists());

        disk::setup::reset(&layout, &Device::default(), &Settings::default(), "v1.0.0")
            .await
            .unwrap();

        assert!(!stale.exists());
        assert!(!layout.resources().exists());
    }

    #[tokio::test]
    async fn wipes_events_subtree() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        write_existing_keys(&layout).await;

        // pre-create something under events/
        let stale = layout.events_dir().file("events.jsonl");
        files::write_string(&stale, "{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        assert!(stale.exists());

        disk::setup::reset(&layout, &Device::default(), &Settings::default(), "v1.0.0")
            .await
            .unwrap();

        assert!(!stale.exists());
        assert!(layout.events_dir().exists());
        assert!(!layout.events_dir().file("events.jsonl").exists());
    }

    #[tokio::test]
    async fn no_prior_state() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        disk::setup::reset(&layout, &Device::default(), &Settings::default(), "v0.1.0")
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        write_existing_keys(&layout).await;

        // pre-write an old marker
        let layout_root = layout.root();
        dirs::create_if_absent(&layout_root).await.unwrap();
        disk::agent_version::write(&layout.agent_version(), "v0.0.1")
            .await
            .unwrap();

        disk::setup::reset(&layout, &Device::default(), &Settings::default(), "v0.0.2")
            .await
            .unwrap();

        assert_marker(&layout, "v0.0.2").await;
    }
}
