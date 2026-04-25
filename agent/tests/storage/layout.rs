// internal crates
use miru_agent::filesys::{self, PathExt};
use miru_agent::storage::Layout;

pub mod storage_layout {
    use super::*;

    #[test]
    fn default_uses_filesystem_root() {
        let layout = Layout::default();
        assert_eq!(
            layout.filesystem_root.path(),
            &std::path::PathBuf::from("/")
        );
    }

    #[test]
    fn root_dir() {
        let layout = Layout::new(filesys::Dir::new("/"));
        let dir = layout.root();
        assert_eq!(dir.to_string(), "/var/lib/miru");
    }

    #[test]
    fn root_dir_custom_filesystem_root() {
        let layout = Layout::new(filesys::Dir::new("/custom"));
        let dir = layout.root();
        assert_eq!(dir.to_string(), "/custom/var/lib/miru");
    }

    #[test]
    fn temp_dir() {
        let layout = Layout::default();
        let dir = layout.temp_dir();
        assert_eq!(dir.to_string(), "/var/lib/miru/tmp");
    }

    #[test]
    fn settings() {
        let layout = Layout::default();
        let file = layout.settings();
        assert_eq!(file.to_string(), "/var/lib/miru/settings.json");
    }

    #[test]
    fn device() {
        let layout = Layout::default();
        let file = layout.device();
        assert_eq!(file.to_string(), "/var/lib/miru/device.json");
    }

    #[test]
    fn resources() {
        let layout = Layout::default();
        let dir = layout.resources();
        assert_eq!(dir.to_string(), "/var/lib/miru/resources");
    }

    #[test]
    fn config_instance_meta() {
        let layout = Layout::default();
        let file = layout.config_instance_meta();
        assert_eq!(
            file.to_string(),
            "/var/lib/miru/resources/config_instances/metadata.json"
        );
    }

    #[test]
    fn config_instance_content() {
        let layout = Layout::default();
        let dir = layout.config_instance_content();
        assert_eq!(
            dir.to_string(),
            "/var/lib/miru/resources/config_instances/contents"
        );
    }

    #[test]
    fn deployments() {
        let layout = Layout::default();
        let file = layout.deployments();
        assert_eq!(file.to_string(), "/var/lib/miru/resources/deployments.json");
    }

    #[test]
    fn releases() {
        let layout = Layout::default();
        let file = layout.releases();
        assert_eq!(file.to_string(), "/var/lib/miru/resources/releases.json");
    }

    #[test]
    fn git_commits() {
        let layout = Layout::default();
        let file = layout.git_commits();
        assert_eq!(file.to_string(), "/var/lib/miru/resources/git_commits.json");
    }
}

pub mod auth_layout {
    use super::*;

    #[test]
    fn auth_dir_path() {
        let layout = Layout::default();
        let auth = layout.auth();
        assert_eq!(auth.root.to_string(), "/var/lib/miru/auth");
    }

    #[test]
    fn private_key_file() {
        let layout = Layout::default();
        let auth = layout.auth();
        let file = auth.private_key();
        assert_eq!(file.to_string(), "/var/lib/miru/auth/private_key.pem");
    }

    #[test]
    fn public_key_file() {
        let layout = Layout::default();
        let auth = layout.auth();
        let file = auth.public_key();
        assert_eq!(file.to_string(), "/var/lib/miru/auth/public_key.pem");
    }

    #[test]
    fn token_file() {
        let layout = Layout::default();
        let auth = layout.auth();
        let file = auth.token();
        assert_eq!(file.to_string(), "/var/lib/miru/auth/token.json");
    }
}
