// internal crates
use miru_agent::filesys::{dir::Dir, file::File, path::PathExt, WriteOptions};
use miru_agent::models::device::Device;
use miru_agent::storage::{self, Layout, Settings};

pub mod bootstrap {
    use super::*;

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

        // config instance deployment directory
        let config_instance_deployment_dir = layout.customer_configs();
        assert!(config_instance_deployment_dir.exists());
    }

    async fn create_temp_key_files(layout: &Layout) -> (File, File) {
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
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap_err();
    }

    #[tokio::test]
    async fn src_private_key_file_doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap_err();
    }

    #[tokio::test]
    async fn clean_install() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn device_file_already_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn auth_directory_already_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn private_key_file_already_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn public_key_file_already_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;
    }

    #[tokio::test]
    async fn storage_directory_already_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;

        // subfile should be deleted
        assert!(!subfile.exists());
    }

    #[tokio::test]
    async fn config_instance_deployment_directory_already_exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let settings = Settings::default();

        // create the public / private key files
        let (private_key_file, public_key_file) = create_temp_key_files(&layout).await;

        // create the config instance deployment directory
        let config_instance_deployment_dir = layout.customer_configs();
        let subfile = config_instance_deployment_dir.file("test");
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
        )
        .await
        .unwrap();

        // validate the storage
        validate_storage(&layout).await;

        // subfile should be deleted
        assert!(!subfile.exists());
    }
}
