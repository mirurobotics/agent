// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::filesys::{self, files, PathExt, WriteOptions};
use miru_agent::server::{
    auth::BearerToken,
    discovery::{self, SCHEMA_VERSION},
    ServerErr,
};

// external crates
use serde_json::{json, Value};

fn discovery_file(dir: &test_dirs::TempDir) -> filesys::File {
    dir.dir().subdir("device-api").file("device-api.json")
}

pub mod write {
    use super::*;

    #[tokio::test]
    async fn write_then_remove() {
        let dir = test_dirs::temp("discovery_write_then_remove").unwrap();
        let file = discovery_file(&dir);
        let token = BearerToken::generate().unwrap();

        discovery::write(&file, 6478, &token).await.unwrap();
        let value: Value = files::read_json(&file).await.unwrap();
        assert_eq!(
            value,
            json!({"schema_version": SCHEMA_VERSION, "port": 6478, "token": token.expose()})
        );

        discovery::remove(&file).await.unwrap();
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn replaces_existing_file_atomically() {
        let dir = test_dirs::temp("discovery_replaces_existing_file").unwrap();
        let file = discovery_file(&dir);
        files::write_string(&file, "junk", WriteOptions::default())
            .await
            .unwrap();
        let token = BearerToken::generate().unwrap();

        discovery::write(&file, 1234, &token).await.unwrap();
        let value: Value = files::read_json(&file).await.unwrap();
        assert_eq!(
            value,
            json!({"schema_version": SCHEMA_VERSION, "port": 1234, "token": token.expose()})
        );

        // the atomic write leaves no temp files behind
        let names: Vec<String> = std::fs::read_dir(file.parent().unwrap().path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["device-api.json".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sets_mode_0640() {
        use std::os::unix::fs::PermissionsExt;

        let dir = test_dirs::temp("discovery_sets_mode_0640").unwrap();
        let file = discovery_file(&dir);
        let token = BearerToken::generate().unwrap();

        discovery::write(&file, 6478, &token).await.unwrap();
        let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    #[tokio::test]
    async fn fails_when_parent_is_a_file() {
        let dir = test_dirs::temp("discovery_fails_when_parent_is_a_file").unwrap();
        let file = discovery_file(&dir);
        std::fs::write(file.parent().unwrap().path(), b"x").unwrap();
        let token = BearerToken::generate().unwrap();

        let result = discovery::write(&file, 6478, &token).await;
        assert!(
            matches!(result, Err(ServerErr::FileSysErr(_))),
            "expected FileSysErr, got {result:?}"
        );
    }
}

pub mod remove {
    use super::*;

    #[tokio::test]
    async fn remove_missing_is_ok() {
        let dir = test_dirs::temp("discovery_remove_missing_is_ok").unwrap();
        let file = discovery_file(&dir);

        discovery::remove(&file).await.unwrap();
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn fails_when_path_is_a_directory() {
        let dir = test_dirs::temp("discovery_fails_when_path_is_a_directory").unwrap();
        let file = discovery_file(&dir);
        std::fs::create_dir_all(file.path()).unwrap();

        let result = discovery::remove(&file).await;
        assert!(
            matches!(result, Err(ServerErr::FileSysErr(_))),
            "expected FileSysErr, got {result:?}"
        );
    }
}

#[cfg(windows)]
pub mod windows_sharing {
    use super::*;

    use std::os::windows::fs::OpenOptionsExt;
    use std::time::Duration;

    // FILE_SHARE_READ without FILE_SHARE_DELETE, as Python's `open` does
    const FILE_SHARE_READ: u32 = 0x1;

    /// Open the file the way a reader without delete sharing does, and close
    /// it after `hold` on another thread.
    fn hold_open(file: &filesys::File, hold: Duration) -> std::thread::JoinHandle<()> {
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(file.path())
            .unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(hold);
            drop(handle);
        })
    }

    #[tokio::test]
    async fn write_waits_out_a_reader_without_delete_sharing() {
        let dir = test_dirs::temp("discovery_write_waits_out_reader").unwrap();
        let file = discovery_file(&dir);
        let old = BearerToken::generate().unwrap();
        discovery::write(&file, 1234, &old).await.unwrap();

        let reader = hold_open(&file, Duration::from_millis(150));
        let token = BearerToken::generate().unwrap();
        discovery::write(&file, 6478, &token).await.unwrap();
        reader.join().unwrap();

        let value: Value = files::read_json(&file).await.unwrap();
        assert_eq!(
            value,
            json!({"schema_version": SCHEMA_VERSION, "port": 6478, "token": token.expose()})
        );
    }

    #[tokio::test]
    async fn remove_waits_out_a_reader_without_delete_sharing() {
        let dir = test_dirs::temp("discovery_remove_waits_out_reader").unwrap();
        let file = discovery_file(&dir);
        let token = BearerToken::generate().unwrap();
        discovery::write(&file, 6478, &token).await.unwrap();

        let reader = hold_open(&file, Duration::from_millis(150));
        discovery::remove(&file).await.unwrap();
        reader.join().unwrap();

        assert!(!file.exists());
    }
}
