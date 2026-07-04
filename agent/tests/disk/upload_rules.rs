// internal crates
use miru_agent::disk::{Capacities, Layout, Storage};
use miru_agent::filesys;
use miru_agent::models::{DeletePolicy, UploadRule, UploadRuleDestination, UploadRuleSource};

// external crates
use chrono::{DateTime, Utc};

fn rule(id: &str, digest: &str, policy: DeletePolicy) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        upload_collection_id: "upl_col_1".to_string(),
        upload_collection_name: "camera-logs".to_string(),
        digest: digest.to_string(),
        source: UploadRuleSource {
            glob: "/data/*.mcap".to_string(),
            stability_window_secs: 30,
        },
        destination: UploadRuleDestination {
            bucket_id: "buck_1".to_string(),
            bucket_name: "my-bucket".to_string(),
            path: "{device_id}/logs".to_string(),
            delete_policy: policy,
        },
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    }
}

async fn init_storage() -> Storage {
    let dir = filesys::dirs::create_temp("upload_rules_test")
        .await
        .unwrap();
    let layout = Layout::new(dir);
    let (storage, _) = Storage::init(&layout, Capacities::default(), "dev".to_string())
        .await
        .unwrap();
    storage
}

#[tokio::test]
async fn write_then_read_round_trips() {
    let storage = init_storage().await;
    let original = rule("upl_rule_1", "sha256:aaa", DeletePolicy::AfterUpload);

    storage
        .upload_rules
        .write_if_absent("upl_rule_1".to_string(), original.clone(), |_, _| false)
        .await
        .unwrap();

    let loaded = storage
        .upload_rules
        .read_optional("upl_rule_1".to_string())
        .await
        .unwrap();

    assert_eq!(loaded, Some(original));
}

#[tokio::test]
async fn write_if_absent_does_not_overwrite_existing() {
    let storage = init_storage().await;
    let first = rule("upl_rule_x", "sha256:first", DeletePolicy::Never);
    let second = rule("upl_rule_x", "sha256:second", DeletePolicy::AfterUpload);

    storage
        .upload_rules
        .write_if_absent("upl_rule_x".to_string(), first.clone(), |_, _| false)
        .await
        .unwrap();
    storage
        .upload_rules
        .write_if_absent("upl_rule_x".to_string(), second, |_, _| false)
        .await
        .unwrap();

    let loaded = storage
        .upload_rules
        .read_optional("upl_rule_x".to_string())
        .await
        .unwrap();

    assert_eq!(loaded, Some(first));
}
