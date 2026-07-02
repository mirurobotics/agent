// internal crates
use miru_agent::filesys::{self, Overwrite};
use miru_agent::models::{self, DplActivity};
use miru_agent::storage::{Deployments, Releases, UploadRules};
use miru_agent::sync::upload_rules::{active_upload_rules, Storage};
use miru_agent::sync::SyncErr;

// ========================= FIXTURE ========================= //

struct Fixture {
    _dir: filesys::Dir,
    deployment_stor: Deployments,
    release_stor: Releases,
    upload_rule_stor: UploadRules,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let (deployment_stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (release_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
            .await
            .unwrap();
        let (upload_rule_stor, _) = UploadRules::spawn(16, dir.file("upload_rules.json"), 1000)
            .await
            .unwrap();
        Self {
            _dir: dir,
            deployment_stor,
            release_stor,
            upload_rule_stor,
        }
    }

    fn storage(&self) -> Storage<'_> {
        Storage {
            deployments: &self.deployment_stor,
            releases: &self.release_stor,
            upload_rules: &self.upload_rule_stor,
        }
    }

    async fn write_deployment(&self, id: &str, release_id: &str, activity_status: DplActivity) {
        let deployment = models::Deployment {
            id: id.to_string(),
            release_id: release_id.to_string(),
            activity_status,
            ..Default::default()
        };
        self.deployment_stor
            .write(id.to_string(), deployment, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
    }

    async fn write_release(&self, id: &str, rule_ids: &[&str]) {
        let release = models::Release {
            id: id.to_string(),
            upload_rule_ids: rule_ids.iter().map(|rid| rid.to_string()).collect(),
            ..Default::default()
        };
        self.release_stor
            .write(id.to_string(), release, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
    }

    async fn write_rule(&self, id: &str) {
        let rule = models::UploadRule {
            id: id.to_string(),
            ..Default::default()
        };
        self.upload_rule_stor
            .write(id.to_string(), rule, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
    }
}

async fn resolved_rule_ids(f: &Fixture) -> Vec<String> {
    let mut ids: Vec<String> = active_upload_rules(&f.storage())
        .await
        .unwrap()
        .into_iter()
        .map(|rule| rule.id)
        .collect();
    ids.sort();
    ids
}

pub mod active_upload_rules_fn {
    use super::*;

    #[tokio::test]
    async fn resolves_rules_for_a_deployed_deployment() {
        let f = Fixture::new("traversal-resolved").await;
        f.write_deployment("dpl_1", "rls_1", DplActivity::Deployed)
            .await;
        f.write_release("rls_1", &["upl_rule_1", "upl_rule_2"])
            .await;
        f.write_rule("upl_rule_1").await;
        f.write_rule("upl_rule_2").await;

        let ids = resolved_rule_ids(&f).await;
        assert_eq!(ids, vec!["upl_rule_1", "upl_rule_2"]);
    }

    #[tokio::test]
    async fn skips_rules_from_non_deployed_deployments() {
        let f = Fixture::new("traversal-stale-skip").await;
        // an archived deployment from a since-replaced release: its rules stay
        // in the append-only store but are no longer reachable
        f.write_deployment("dpl_old", "rls_old", DplActivity::Archived)
            .await;
        f.write_release("rls_old", &["upl_rule_old"]).await;
        f.write_rule("upl_rule_old").await;
        // a queued deployment is not deployed yet
        f.write_deployment("dpl_next", "rls_next", DplActivity::Queued)
            .await;
        f.write_release("rls_next", &["upl_rule_next"]).await;
        f.write_rule("upl_rule_next").await;
        // the deployed deployment
        f.write_deployment("dpl_1", "rls_1", DplActivity::Deployed)
            .await;
        f.write_release("rls_1", &["upl_rule_1"]).await;
        f.write_rule("upl_rule_1").await;

        let ids = resolved_rule_ids(&f).await;
        assert_eq!(ids, vec!["upl_rule_1"]);
    }

    #[tokio::test]
    async fn idles_with_no_deployed_deployment() {
        let f = Fixture::new("traversal-idle").await;
        f.write_deployment("dpl_old", "rls_old", DplActivity::Archived)
            .await;
        f.write_release("rls_old", &["upl_rule_old"]).await;
        f.write_rule("upl_rule_old").await;

        let ids = resolved_rule_ids(&f).await;
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn skips_a_deployment_whose_release_is_missing() {
        let f = Fixture::new("traversal-missing-release").await;
        f.write_deployment("dpl_1", "rls_missing", DplActivity::Deployed)
            .await;

        let ids = resolved_rule_ids(&f).await;
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn skips_a_rule_id_whose_body_is_missing() {
        let f = Fixture::new("traversal-missing-id").await;
        f.write_deployment("dpl_1", "rls_1", DplActivity::Deployed)
            .await;
        f.write_release("rls_1", &["upl_rule_missing", "upl_rule_1"])
            .await;
        f.write_rule("upl_rule_1").await;

        let ids = resolved_rule_ids(&f).await;
        assert_eq!(ids, vec!["upl_rule_1"]);
    }

    #[tokio::test]
    async fn unions_and_dedupes_across_deployed_deployments() {
        let f = Fixture::new("traversal-union").await;
        // two deployed deployments mid-redeploy: the outgoing and incoming
        // releases share a rule
        f.write_deployment("dpl_1", "rls_1", DplActivity::Deployed)
            .await;
        f.write_release("rls_1", &["upl_rule_1", "upl_rule_shared"])
            .await;
        f.write_deployment("dpl_2", "rls_2", DplActivity::Deployed)
            .await;
        f.write_release("rls_2", &["upl_rule_shared", "upl_rule_2"])
            .await;
        f.write_rule("upl_rule_1").await;
        f.write_rule("upl_rule_2").await;
        f.write_rule("upl_rule_shared").await;

        let ids = resolved_rule_ids(&f).await;
        assert_eq!(ids, vec!["upl_rule_1", "upl_rule_2", "upl_rule_shared"]);
    }

    #[tokio::test]
    async fn propagates_cache_errors() {
        let f = Fixture::new("traversal-cache-error").await;
        f.write_deployment("dpl_1", "rls_1", DplActivity::Deployed)
            .await;

        // kill the release store so the traversal hits a cache error
        f.release_stor.shutdown().await.unwrap();

        let err = active_upload_rules(&f.storage()).await.unwrap_err();
        assert!(matches!(err, SyncErr::CacheErr(_)));
    }
}
