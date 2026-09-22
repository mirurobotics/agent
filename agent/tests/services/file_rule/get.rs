// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::cache::CacheErr;
use miru_agent::disk::FileRules;
use miru_agent::models::{FileRule, FileRuleSource, FileRuleUpload};
use miru_agent::services::file_rule as file_rule_svc;
use miru_agent::services::ServiceErr;

async fn setup(name: &str) -> (test_dirs::TempDir, FileRules) {
    let dir = test_dirs::temp(name).unwrap();
    let (stor, _) = FileRules::spawn(16, dir.file("file_rules.json"), 1000)
        .await
        .unwrap();
    (dir, stor)
}

pub mod get_file_rule {
    use super::*;

    #[tokio::test]
    async fn returns_cached_rule() {
        let (_dir, stor) = setup("get_file_rule_cached").await;
        let rule = FileRule {
            id: "fr_1".to_string(),
            name: "logs".to_string(),
            digest: "digest_1".to_string(),
            source: FileRuleSource {
                glob: "/var/log/app/*.log".to_string(),
                stability_window_secs: 30,
            },
            upload: Some(FileRuleUpload {
                upload_collection_id: "uc_1".to_string(),
                upload_collection_name: "logs".to_string(),
                bucket_id: "bkt_1".to_string(),
                bucket_name: "fleet-logs".to_string(),
                path: "robots/".to_string(),
            }),
            ..Default::default()
        };
        stor.write_if_absent("fr_1".to_string(), rule.clone(), |_, _| false)
            .await
            .unwrap();

        let result = file_rule_svc::get(&stor, "fr_1".to_string()).await.unwrap();
        assert_eq!(result, rule);
    }

    #[tokio::test]
    async fn missing_rule_returns_not_found() {
        let (_dir, stor) = setup("get_file_rule_missing").await;

        let result = file_rule_svc::get(&stor, "missing".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }
}
