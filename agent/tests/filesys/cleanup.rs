// standard crates
use std::collections::HashSet;

// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::filesys::{cleanup, files, PathExt, WriteOptions};

pub mod prune {
    use super::*;

    #[tokio::test]
    async fn deletes_only_the_files_not_kept() {
        let tmp = test_dirs::temp("testing").unwrap();
        let dir = tmp.to_dir();
        let kept = dir.file("kept");
        let other = dir.file("other");
        for file in [&kept, &other] {
            files::write_string(file, "x", WriteOptions::OVERWRITE_ATOMIC)
                .await
                .unwrap();
        }

        cleanup::prune(&dir, &HashSet::from(["kept".to_string()])).await;

        assert!(kept.exists());
        assert!(!other.exists());
    }

    #[tokio::test]
    async fn a_missing_dir_is_not_created() {
        let tmp = test_dirs::temp("testing").unwrap();
        let dir = tmp.to_dir().subdir("missing");

        cleanup::prune(&dir, &HashSet::new()).await;

        assert!(!dir.exists());
    }
}

pub mod delete {
    use super::*;

    #[tokio::test]
    async fn a_missing_file_is_fine() {
        let tmp = test_dirs::temp("testing").unwrap();
        let file = tmp.to_dir().file("missing");

        cleanup::delete(&file).await;

        assert!(!file.exists());
    }
}
