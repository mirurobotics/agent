// internal crates
use miru_agent::disk::agent_version;
use miru_agent::filesys::{dirs, files, WriteOptions};

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_none_when_file_missing() {
        let dir = dirs::temp("agent_version_read_missing").unwrap();
        let file = dir.file("agent_version");
        let result = agent_version::read(&file).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn returns_some_when_file_present() {
        let dir = dirs::temp("agent_version_read_present").unwrap();
        let file = dir.file("agent_version");
        files::write_string(&file, "v1.2.3\n", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = agent_version::read(&file).await.unwrap();
        assert_eq!(result, Some("v1.2.3".to_string()));
    }

    #[tokio::test]
    async fn trims_surrounding_whitespace() {
        let dir = dirs::temp("agent_version_read_trim").unwrap();
        let file = dir.file("agent_version");
        files::write_string(&file, "  v0.4.0  \n\n", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = agent_version::read(&file).await.unwrap();
        assert_eq!(result, Some("v0.4.0".to_string()));
    }
}

pub mod write {
    use super::*;

    #[tokio::test]
    async fn writes_version_with_trailing_newline() {
        let dir = dirs::temp("agent_version_write").unwrap();
        let file = dir.file("agent_version");

        agent_version::write(&file, "v0.9.0").await.unwrap();

        let read_back = files::read_string(&file).await.unwrap();
        assert_eq!(read_back, "v0.9.0\n");
    }

    #[tokio::test]
    async fn overwrites_existing_marker() {
        let dir = dirs::temp("agent_version_overwrite").unwrap();
        let file = dir.file("agent_version");

        agent_version::write(&file, "v0.0.1").await.unwrap();
        agent_version::write(&file, "v0.0.2").await.unwrap();

        let read_back = agent_version::read(&file).await.unwrap();
        assert_eq!(read_back, Some("v0.0.2".to_string()));
    }
}
