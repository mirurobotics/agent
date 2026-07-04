// internal crates
use miru_agent::disk::agent_version;
use miru_agent::filesys::{self, WriteOptions};

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_none_when_file_missing() {
        let dir = filesys::dirs::create_temp("agent_version_read_missing")
            .await
            .unwrap();
        let file = dir.file("agent_version");
        let result = agent_version::read(&file).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn returns_some_when_file_present() {
        let dir = filesys::dirs::create_temp("agent_version_read_present")
            .await
            .unwrap();
        let file = dir.file("agent_version");
        filesys::files::write_string(&file, "v1.2.3\n", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = agent_version::read(&file).await.unwrap();
        assert_eq!(result, Some("v1.2.3".to_string()));
    }

    #[tokio::test]
    async fn trims_surrounding_whitespace() {
        let dir = filesys::dirs::create_temp("agent_version_read_trim")
            .await
            .unwrap();
        let file = dir.file("agent_version");
        filesys::files::write_string(&file, "  v0.4.0  \n\n", WriteOptions::OVERWRITE_ATOMIC)
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
        let dir = filesys::dirs::create_temp("agent_version_write")
            .await
            .unwrap();
        let file = dir.file("agent_version");

        agent_version::write(&file, "v0.9.0").await.unwrap();

        let read_back = filesys::files::read_string(&file).await.unwrap();
        assert_eq!(read_back, "v0.9.0\n");
    }

    #[tokio::test]
    async fn overwrites_existing_marker() {
        let dir = filesys::dirs::create_temp("agent_version_overwrite")
            .await
            .unwrap();
        let file = dir.file("agent_version");

        agent_version::write(&file, "v0.0.1").await.unwrap();
        agent_version::write(&file, "v0.0.2").await.unwrap();

        let read_back = agent_version::read(&file).await.unwrap();
        assert_eq!(read_back, Some("v0.0.2".to_string()));
    }
}
