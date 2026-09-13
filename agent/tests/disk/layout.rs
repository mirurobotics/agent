// standard crates
use std::path::{Path, PathBuf};

// internal crates
use miru_agent::disk::Layout;
use miru_agent::filesys::{self, PathExt};

/// Expected filesystem root of the default layout: `/` on Unix (pinned
/// byte-for-byte as a compatibility contract), the `ProgramData` env value on
/// Windows (matching the platform module's resolution).
fn expected_default_base() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/")
    }
    #[cfg(windows)]
    {
        miru_agent::platform::windows_data_root_base(std::env::var_os("ProgramData"))
    }
}

/// Expected data root under a given filesystem root.
fn expected_root_under(base: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        base.join("var").join("lib").join("miru")
    }
    #[cfg(windows)]
    {
        base.join("Miru")
    }
}

/// Expected display string for a path nested under the default data root.
fn under_root(parts: &[&str]) -> String {
    let mut path = expected_root_under(&expected_default_base());
    for part in parts {
        path = path.join(part);
    }
    path.display().to_string()
}

pub mod storage_layout {
    use super::*;

    #[test]
    fn default_uses_filesystem_root() {
        let layout = Layout::default();
        assert_eq!(layout.filesystem_root.path(), &expected_default_base());
    }

    #[test]
    fn root_dir() {
        let layout = Layout::new(filesys::Dir::new(expected_default_base()));
        let dir = layout.root();
        assert_eq!(dir.to_string(), under_root(&[]));
        // pin the Unix contract byte-for-byte so the derived helpers cannot
        // drift together with the implementation
        #[cfg(unix)]
        assert_eq!(dir.to_string(), "/var/lib/miru");
    }

    #[test]
    fn root_dir_custom_filesystem_root() {
        let tmp = crate::tests::test_utils::filesys::dirs::temp("custom-root").unwrap();
        let layout = Layout::new(tmp.to_dir());
        let dir = layout.root();
        assert_eq!(
            dir.to_string(),
            expected_root_under(tmp.path()).display().to_string()
        );
    }

    #[test]
    fn temp_dir() {
        let layout = Layout::default();
        let dir = layout.temp_dir();
        assert_eq!(dir.to_string(), under_root(&["tmp"]));
    }

    #[test]
    fn settings() {
        let layout = Layout::default();
        let file = layout.settings();
        assert_eq!(file.to_string(), under_root(&["settings.json"]));
    }

    #[test]
    fn device() {
        let layout = Layout::default();
        let file = layout.device();
        assert_eq!(file.to_string(), under_root(&["device.json"]));
    }

    #[test]
    fn scanner_snapshot() {
        let layout = Layout::default();
        let file = layout.scanner_snapshot();
        assert_eq!(file.to_string(), under_root(&["scanner.json"]));
    }

    #[test]
    fn delete_queue() {
        let layout = Layout::default();
        let file = layout.delete_queue();
        assert_eq!(file.to_string(), under_root(&["delete_queue.json"]));
    }

    #[test]
    fn resources() {
        let layout = Layout::default();
        let dir = layout.resources();
        assert_eq!(dir.to_string(), under_root(&["resources"]));
    }

    #[test]
    fn config_instance_meta() {
        let layout = Layout::default();
        let file = layout.config_instance_meta();
        assert_eq!(
            file.to_string(),
            under_root(&["resources", "config_instances", "metadata.json"])
        );
    }

    #[test]
    fn config_instance_content() {
        let layout = Layout::default();
        let dir = layout.config_instance_content();
        assert_eq!(
            dir.to_string(),
            under_root(&["resources", "config_instances", "contents"])
        );
    }

    #[test]
    fn deployments() {
        let layout = Layout::default();
        let file = layout.deployments();
        assert_eq!(
            file.to_string(),
            under_root(&["resources", "deployments.json"])
        );
    }

    #[test]
    fn releases() {
        let layout = Layout::default();
        let file = layout.releases();
        assert_eq!(
            file.to_string(),
            under_root(&["resources", "releases.json"])
        );
    }

    #[test]
    fn file_rules() {
        let layout = Layout::default();
        let file = layout.file_rules();
        assert_eq!(
            file.to_string(),
            under_root(&["resources", "file_rules.json"])
        );
    }

    #[test]
    fn git_commits() {
        let layout = Layout::default();
        let file = layout.git_commits();
        assert_eq!(
            file.to_string(),
            under_root(&["resources", "git_commits.json"])
        );
    }
}

pub mod auth_layout {
    use super::*;

    #[test]
    fn auth_dir_path() {
        let layout = Layout::default();
        let auth = layout.auth();
        assert_eq!(auth.root.to_string(), under_root(&["auth"]));
    }

    #[test]
    fn private_key_file() {
        let layout = Layout::default();
        let auth = layout.auth();
        let file = auth.private_key();
        assert_eq!(file.to_string(), under_root(&["auth", "private_key.pem"]));
    }

    #[test]
    fn public_key_file() {
        let layout = Layout::default();
        let auth = layout.auth();
        let file = auth.public_key();
        assert_eq!(file.to_string(), under_root(&["auth", "public_key.pem"]));
    }

    #[test]
    fn token_file() {
        let layout = Layout::default();
        let auth = layout.auth();
        let file = auth.token();
        assert_eq!(file.to_string(), under_root(&["auth", "token.json"]));
    }
}
