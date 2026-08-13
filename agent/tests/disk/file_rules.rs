// internal crates
use miru_agent::disk::{Capacities, Layout, Storage};
use miru_agent::filesys::dirs;
use miru_agent::models::{FileRule, FileRuleRetention, FileRuleSource, FileRuleUpload};

// external crates
use chrono::{DateTime, Utc};

fn rule(id: &str, digest: &str, retention: Option<FileRuleRetention>) -> FileRule {
    FileRule {
        id: id.to_string(),
        name: "camera-logs".to_string(),
        digest: digest.to_string(),
        source: FileRuleSource {
            glob: "/data/*.mcap".to_string(),
            stability_window_secs: 30,
        },
        upload: Some(FileRuleUpload {
            upload_collection_id: "upl_col_1".to_string(),
            upload_collection_name: "camera-logs".to_string(),
            bucket_id: "buck_1".to_string(),
            bucket_name: "my-bucket".to_string(),
            path: "{device_id}/logs".to_string(),
        }),
        retention,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    }
}

async fn init_storage() -> (dirs::TempDir, Storage) {
    let dir = dirs::temp("file_rules_test").unwrap();
    let layout = Layout::new(dir.to_dir());
    let (storage, _) = Storage::init(&layout, Capacities::default(), "dev".to_string())
        .await
        .unwrap();
    (dir, storage)
}

#[tokio::test]
async fn write_then_read_round_trips() {
    let (_tmp, storage) = init_storage().await;
    let original = rule(
        "upl_rule_1",
        "sha256:aaa",
        Some(FileRuleRetention {
            require_upload: true,
            ttl_secs: 0,
        }),
    );

    storage
        .file_rules
        .write_if_absent("upl_rule_1".to_string(), original.clone(), |_, _| false)
        .await
        .unwrap();

    let loaded = storage
        .file_rules
        .read_optional("upl_rule_1".to_string())
        .await
        .unwrap();

    assert_eq!(loaded, Some(original));
}

#[tokio::test]
async fn write_if_absent_does_not_overwrite_existing() {
    let (_tmp, storage) = init_storage().await;
    let first = rule("upl_rule_x", "sha256:first", None);
    let second = rule(
        "upl_rule_x",
        "sha256:second",
        Some(FileRuleRetention {
            require_upload: true,
            ttl_secs: 0,
        }),
    );

    storage
        .file_rules
        .write_if_absent("upl_rule_x".to_string(), first.clone(), |_, _| false)
        .await
        .unwrap();
    storage
        .file_rules
        .write_if_absent("upl_rule_x".to_string(), second, |_, _| false)
        .await
        .unwrap();

    let loaded = storage
        .file_rules
        .read_optional("upl_rule_x".to_string())
        .await
        .unwrap();

    assert_eq!(loaded, Some(first));
}
