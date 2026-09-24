// internal crates
use crate::test_utils::filesys::{dirs as test_dirs, files as test_files};
use backend_api::models as backend_client;
use miru_agent::disk::{system_metadata, DiskErr};
use miru_agent::filesys::{files, FileSysErr};
use miru_agent::models::SystemMetadata;

// external crates
use serde_json::json;

fn full_metadata() -> SystemMetadata {
    SystemMetadata {
        os: Some(backend_client::Os::OS_LINUX),
        arch: Some(backend_client::Arch::ARCH_X86_64),
        hostname: Some("robot-1.local".to_string()),
        os_version: Some("Ubuntu 22.04".to_string()),
        kernel_version: Some("5.15.0-91-generic".to_string()),
    }
}

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_none_when_file_missing() {
        let dir = test_dirs::temp("system_metadata_read_missing").unwrap();
        let file = dir.file("system_metadata.json");
        let result = system_metadata::read(&file).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn returns_some_when_file_present() {
        let dir = test_dirs::temp("system_metadata_read_present").unwrap();
        let file = dir.file("system_metadata.json");
        let contents = json!({
            "os": "linux",
            "arch": "x86_64",
            "hostname": "robot-1.local",
            "os_version": "Ubuntu 22.04",
            "kernel_version": "5.15.0-91-generic",
        });
        test_files::seed(&file, &contents.to_string()).await;

        let result = system_metadata::read(&file).await.unwrap();
        assert_eq!(result, Some(full_metadata()));
    }

    #[tokio::test]
    async fn missing_fields_read_as_none() {
        let dir = test_dirs::temp("system_metadata_read_partial").unwrap();
        let file = dir.file("system_metadata.json");
        test_files::seed(&file, r#"{"hostname": "robot-1.local"}"#).await;

        let result = system_metadata::read(&file).await.unwrap();
        let expected = SystemMetadata {
            hostname: Some("robot-1.local".to_string()),
            ..SystemMetadata::default()
        };
        assert_eq!(result, Some(expected));
    }

    #[tokio::test]
    async fn returns_error_when_file_is_not_json() {
        let dir = test_dirs::temp("system_metadata_read_invalid").unwrap();
        let file = dir.file("system_metadata.json");
        test_files::seed(&file, "not json").await;

        let result = system_metadata::read(&file).await;
        assert!(
            matches!(
                result,
                Err(DiskErr::FileSysErr(FileSysErr::ParseJSONErr(_)))
            ),
            "expected ParseJSONErr, got {result:?}"
        );
    }
}

pub mod write {
    use super::*;

    #[tokio::test]
    async fn writes_metadata_as_json() {
        let dir = test_dirs::temp("system_metadata_write").unwrap();
        let file = dir.file("system_metadata.json");

        system_metadata::write(&file, &full_metadata())
            .await
            .unwrap();

        let read_back: serde_json::Value = files::read_json(&file).await.unwrap();
        let expected = json!({
            "os": "linux",
            "arch": "x86_64",
            "hostname": "robot-1.local",
            "os_version": "Ubuntu 22.04",
            "kernel_version": "5.15.0-91-generic",
        });
        assert_eq!(read_back, expected);
    }

    #[tokio::test]
    async fn writes_none_fields_as_null() {
        let dir = test_dirs::temp("system_metadata_write_none").unwrap();
        let file = dir.file("system_metadata.json");

        system_metadata::write(&file, &SystemMetadata::default())
            .await
            .unwrap();

        let read_back: serde_json::Value = files::read_json(&file).await.unwrap();
        let expected = json!({
            "os": null,
            "arch": null,
            "hostname": null,
            "os_version": null,
            "kernel_version": null,
        });
        assert_eq!(read_back, expected);
    }

    #[tokio::test]
    async fn overwrites_existing_cache() {
        let dir = test_dirs::temp("system_metadata_overwrite").unwrap();
        let file = dir.file("system_metadata.json");
        let first = SystemMetadata {
            hostname: Some("old-host".to_string()),
            ..SystemMetadata::default()
        };
        let second = SystemMetadata {
            hostname: Some("new-host".to_string()),
            ..SystemMetadata::default()
        };

        system_metadata::write(&file, &first).await.unwrap();
        system_metadata::write(&file, &second).await.unwrap();

        let read_back = system_metadata::read(&file).await.unwrap();
        assert_eq!(read_back, Some(second));
    }
}
