// internal crates
use miru_agent::filesys::{self, WriteOptions};
use miru_agent::storage::agent_version;
use miru_agent::storage::{AgentVersion, StorageErr};

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_none_when_file_missing() {
        let dir = filesys::Dir::create_temp_dir("agent_version_read_missing")
            .await
            .unwrap();
        let file = dir.file("agent_version.json");
        let result = agent_version::read(&file).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn returns_some_when_file_present() {
        let dir = filesys::Dir::create_temp_dir("agent_version_read_present")
            .await
            .unwrap();
        let file = dir.file("agent_version.json");
        let marker = AgentVersion {
            version: "v1.2.3".to_string(),
        };
        file.write_json(&marker, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = agent_version::read(&file).await.unwrap();
        assert_eq!(result, Some(marker));
    }

    #[tokio::test]
    async fn propagates_error_when_file_corrupt() {
        let dir = filesys::Dir::create_temp_dir("agent_version_read_corrupt")
            .await
            .unwrap();
        let file = dir.file("agent_version.json");
        file.write_string("not valid json", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let err = agent_version::read(&file).await.unwrap_err();
        assert!(matches!(err, StorageErr::FileSysErr(_)));
    }
}

pub mod write {
    use super::*;

    #[tokio::test]
    async fn writes_marker_atomically() {
        let dir = filesys::Dir::create_temp_dir("agent_version_write")
            .await
            .unwrap();
        let file = dir.file("agent_version.json");
        let marker = AgentVersion {
            version: "v0.9.0".to_string(),
        };

        agent_version::write(&file, &marker).await.unwrap();

        let read_back = file.read_json::<AgentVersion>().await.unwrap();
        assert_eq!(read_back, marker);
    }

    #[tokio::test]
    async fn overwrites_existing_marker() {
        let dir = filesys::Dir::create_temp_dir("agent_version_overwrite")
            .await
            .unwrap();
        let file = dir.file("agent_version.json");
        let first = AgentVersion {
            version: "v0.0.1".to_string(),
        };
        let second = AgentVersion {
            version: "v0.0.2".to_string(),
        };

        agent_version::write(&file, &first).await.unwrap();
        agent_version::write(&file, &second).await.unwrap();

        let read_back = file.read_json::<AgentVersion>().await.unwrap();
        assert_eq!(read_back, second);
    }
}
