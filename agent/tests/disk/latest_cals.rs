// internal crates
use crate::test_utils::calibrations::{latest_cals_layout, latest_store, report};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::cache::dir_impl::entry_file_name;
use miru_agent::disk::latest_cals::prune_orphaned_contents;
use miru_agent::disk::layout::LatestCalsLayout;
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};

// external crates
use uuid::Uuid;

/// Write a content file that no index entry names.
async fn write_orphan(layout: &LatestCalsLayout) -> File {
    dirs::create(&layout.contents()).await.unwrap();
    let name = format!("{}.json", Uuid::new_v4());
    let orphan = layout.contents().file(&name);
    files::write_string(&orphan, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    orphan
}

pub mod prune_orphaned_contents {
    use super::*;

    #[tokio::test]
    async fn deletes_only_the_contents_no_entry_names() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = latest_cals_layout(&dir);
        let named = report("a");
        let store = latest_store(&dir, 10).await;
        store
            .put(named.metadata.clone(), &named.data)
            .await
            .unwrap();
        let orphan = write_orphan(&layout).await;

        prune_orphaned_contents(&layout).await;

        let report_id = named.metadata.report_id.to_string();
        let named_content = layout.contents().file(&entry_file_name(&report_id));
        assert!(named_content.exists());
        assert!(!orphan.exists());
    }

    #[tokio::test]
    async fn keeps_the_contents_when_the_index_cannot_be_read() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = latest_cals_layout(&dir);
        let index_dir = layout.root.subdir("index.json");
        dirs::create(&index_dir).await.unwrap();
        let orphan = write_orphan(&layout).await;

        prune_orphaned_contents(&layout).await;

        assert!(orphan.exists());
    }

    #[tokio::test]
    async fn keeps_the_contents_when_the_index_is_corrupt() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = latest_cals_layout(&dir);
        let orphan = write_orphan(&layout).await;
        files::write_string(&layout.index(), "{", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        prune_orphaned_contents(&layout).await;

        assert!(orphan.exists());
    }
}
