// internal crates
use miru_agent::filesys::{dir::Dir, path::PathExt};
use miru_agent::storage::layout::StorageLayout;

pub mod storage_layout {
    use super::*;

    #[test]
    fn default_uses_root() {
        let layout = StorageLayout::default();
        assert_eq!(layout.root.path(), &std::path::PathBuf::from("/"));
    }

    #[test]
    fn internal_dir() {
        let layout = StorageLayout::new(Dir::new("/"));
        let dir = layout.internal_dir();
        assert_eq!(dir.to_string(), "/var/lib/miru");
    }

    #[test]
    fn internal_dir_custom_root() {
        let layout = StorageLayout::new(Dir::new("/custom"));
        let dir = layout.internal_dir();
        assert_eq!(dir.to_string(), "/custom/var/lib/miru");
    }

    #[test]
    fn temp_dir() {
        let layout = StorageLayout::default();
        let dir = layout.temp_dir();
        assert_eq!(dir.to_string(), "/var/lib/miru/tmp");
    }

    #[test]
    fn device_file() {
        let layout = StorageLayout::default();
        let file = layout.device_file();
        assert_eq!(file.to_string(), "/var/lib/miru/device.json");
    }

    #[test]
    fn settings_file() {
        let layout = StorageLayout::default();
        let file = layout.settings_file();
        assert_eq!(file.to_string(), "/var/lib/miru/settings.json");
    }

    #[test]
    fn caches_dir() {
        let layout = StorageLayout::default();
        let dir = layout.caches_dir();
        assert_eq!(dir.to_string(), "/var/lib/miru/cache");
    }

    #[test]
    fn config_instance_caches() {
        let layout = StorageLayout::default();
        let dir = layout.config_instance_caches();
        assert_eq!(dir.to_string(), "/var/lib/miru/cache/config_instances");
    }

    #[test]
    fn config_instance_cache() {
        let layout = StorageLayout::default();
        let file = layout.config_instance_cache();
        assert_eq!(
            file.to_string(),
            "/var/lib/miru/cache/config_instances/metadata.json"
        );
    }

    #[test]
    fn config_instance_content_cache() {
        let layout = StorageLayout::default();
        let dir = layout.config_instance_content_cache();
        assert_eq!(
            dir.to_string(),
            "/var/lib/miru/cache/config_instances/contents"
        );
    }

    #[test]
    fn deployment_caches() {
        let layout = StorageLayout::default();
        let dir = layout.deployment_caches();
        assert_eq!(dir.to_string(), "/var/lib/miru/cache/deployments");
    }

    #[test]
    fn deployment_cache() {
        let layout = StorageLayout::default();
        let file = layout.deployment_cache();
        assert_eq!(
            file.to_string(),
            "/var/lib/miru/cache/deployments/metadata.json"
        );
    }

    #[test]
    fn config_instance_deployment_dir() {
        let layout = StorageLayout::default();
        let dir = layout.config_instance_deployment_dir();
        assert_eq!(dir.to_string(), "/srv/miru/config_instances");
    }

    #[test]
    fn config_instance_deployment_dir_custom_root() {
        let layout = StorageLayout::new(Dir::new("/opt"));
        let dir = layout.config_instance_deployment_dir();
        assert_eq!(dir.to_string(), "/opt/srv/miru/config_instances");
    }
}

pub mod auth_layout {
    use super::*;

    #[test]
    fn auth_dir_path() {
        let layout = StorageLayout::default();
        let auth = layout.auth_dir();
        assert_eq!(auth.root.to_string(), "/var/lib/miru/auth");
    }

    #[test]
    fn private_key_file() {
        let layout = StorageLayout::default();
        let auth = layout.auth_dir();
        let file = auth.private_key_file();
        assert_eq!(file.to_string(), "/var/lib/miru/auth/private_key.pem");
    }

    #[test]
    fn public_key_file() {
        let layout = StorageLayout::default();
        let auth = layout.auth_dir();
        let file = auth.public_key_file();
        assert_eq!(file.to_string(), "/var/lib/miru/auth/public_key.pem");
    }

    #[test]
    fn token_file() {
        let layout = StorageLayout::default();
        let auth = layout.auth_dir();
        let file = auth.token_file();
        assert_eq!(file.to_string(), "/var/lib/miru/auth/token.json");
    }
}
