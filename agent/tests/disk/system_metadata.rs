// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models as backend_client;
use miru_agent::disk::system_metadata;
use miru_agent::models::SystemMetadata;

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_none_when_file_missing() {
        let dir = test_dirs::temp("system_metadata_read_missing").unwrap();
        let file = dir.file("system_metadata.json");

        let result = system_metadata::read(&file).await.unwrap();

        assert_eq!(result, None);
    }
}

pub mod round_trip {
    use super::*;

    #[tokio::test]
    async fn writes_then_reads_fully_populated_metadata() {
        let dir = test_dirs::temp("system_metadata_round_trip_full").unwrap();
        let file = dir.file("system_metadata.json");
        let meta = SystemMetadata {
            os: Some(backend_client::Os::OS_LINUX),
            arch: Some(backend_client::Arch::ARCH_X86_64),
            hostname: Some("robot-1.local".to_string()),
            os_version: Some("Ubuntu 22.04".to_string()),
            kernel_version: Some("5.15.0-91-generic".to_string()),
        };

        system_metadata::write(&file, &meta).await.unwrap();
        let read_back = system_metadata::read(&file).await.unwrap();

        assert_eq!(read_back, Some(meta));
    }

    #[tokio::test]
    async fn writes_then_reads_metadata_with_none_fields() {
        let dir = test_dirs::temp("system_metadata_round_trip_none").unwrap();
        let file = dir.file("system_metadata.json");
        let meta = SystemMetadata::default();

        system_metadata::write(&file, &meta).await.unwrap();
        let read_back = system_metadata::read(&file).await.unwrap();

        assert_eq!(read_back, Some(SystemMetadata::default()));
    }

    #[tokio::test]
    async fn write_overwrites_existing_cache() {
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
