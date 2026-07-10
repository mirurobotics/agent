// internal crates
use crate::cache;
use crate::models;

pub type UploadRules = cache::FileCache<models::UploadRuleID, models::UploadRule>;

/// Resolve the upload rules referenced by a deployment's release: read the
/// release, then read each rule body it references.
pub async fn upload_rules_for_deployment(
    releases: &super::Releases,
    upload_rules: &UploadRules,
    deployment: &models::Deployment,
) -> Result<Vec<models::UploadRule>, super::DiskErr> {
    let release = releases.read(deployment.release_id.clone()).await?;
    let mut rules = Vec::new();
    for rule_id in &release.upload_rule_ids {
        rules.push(upload_rules.read(rule_id.clone()).await?);
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::BTreeSet;

    // internal crates
    use crate::disk::{self, Layout};
    use crate::filesys;
    use crate::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};

    // =============================== TEST HELPERS ================================= //

    /// Build an UploadRule from Default with only the source fields set.
    fn rule_with(id: &str, glob: &str, stability_window_secs: i32) -> UploadRule {
        UploadRule {
            id: id.to_string(),
            source: UploadRuleSource {
                glob: glob.to_string(),
                stability_window_secs,
            },
            ..Default::default()
        }
    }

    /// The set of resolved rule ids, order-independent.
    fn ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
    }

    struct Stores {
        _dir: filesys::dirs::TempDir,
        deployments: disk::Deployments,
        releases: disk::Releases,
        upload_rules: disk::UploadRules,
    }

    impl Stores {
        async fn new() -> Self {
            let dir = filesys::dirs::temp("testing").unwrap();
            let layout = Layout::new(dir.to_dir());
            let (deployments, _) = disk::Deployments::spawn(64, layout.deployments(), 1000)
                .await
                .unwrap();
            let (releases, _) = disk::Releases::spawn(64, layout.releases(), 1000)
                .await
                .unwrap();
            let (upload_rules, _) = disk::UploadRules::spawn(64, layout.upload_rules(), 1000)
                .await
                .unwrap();
            Self {
                _dir: dir,
                deployments,
                releases,
                upload_rules,
            }
        }

        /// Seed a Deployed deployment -> release -> upload rule bodies so the
        /// deployment-scoped traversal resolves to the given rules.
        async fn seed_deployed(&self, dpl_id: &str, release_id: &str, rules: &[UploadRule]) {
            self.deployments
                .write_if_absent(
                    dpl_id.to_string(),
                    Deployment {
                        id: dpl_id.to_string(),
                        activity_status: DplActivity::Deployed,
                        release_id: release_id.to_string(),
                        ..Default::default()
                    },
                    |_, _| false,
                )
                .await
                .unwrap();
            self.releases
                .write_if_absent(
                    release_id.to_string(),
                    Release {
                        id: release_id.to_string(),
                        upload_rule_ids: rules.iter().map(|r| r.id.clone()).collect(),
                        ..Default::default()
                    },
                    |_, _| false,
                )
                .await
                .unwrap();
            for rule in rules {
                self.upload_rules
                    .write_if_absent(rule.id.clone(), rule.clone(), |_, _| false)
                    .await
                    .unwrap();
            }
        }

        async fn find_deployed(&self) -> Result<Option<Deployment>, disk::DiskErr> {
            crate::disk::deployments::find_deployed(&self.deployments).await
        }

        async fn upload_rules_for_deployment(
            &self,
            deployment: &Deployment,
        ) -> Result<Vec<UploadRule>, disk::DiskErr> {
            super::upload_rules_for_deployment(&self.releases, &self.upload_rules, deployment).await
        }
    }

    // Deployed deployment -> release ids [r1,r2] with both bodies present resolves
    // to both rules.
    #[tokio::test]
    async fn resolves_active_set() {
        let stores = Stores::new().await;
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        let r2 = rule_with("r2", "/none/*.mcap", 0);
        stores.seed_deployed("dpl", "rel", &[r1, r2]).await;

        let deployed = stores.find_deployed().await.unwrap().expect("deployed");
        let rules = stores.upload_rules_for_deployment(&deployed).await.unwrap();
        assert_eq!(
            ids(&rules),
            BTreeSet::from(["r1".to_string(), "r2".to_string()])
        );
    }

    // A rule body present in the store but NOT referenced by the deployed release is
    // not resolved.
    #[tokio::test]
    async fn stale_rule_not_acted_on() {
        let stores = Stores::new().await;
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        // release references only r1, but r2's body also lives in the store.
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;
        stores
            .upload_rules
            .write_if_absent(
                "r2".to_string(),
                rule_with("r2", "/none/*.mcap", 0),
                |_, _| false,
            )
            .await
            .unwrap();

        let deployed = stores.find_deployed().await.unwrap().expect("deployed");
        let rules = stores.upload_rules_for_deployment(&deployed).await.unwrap();
        assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string()]));
    }

    // The release references [r1,r2] but only r1's body is in the store: the missing
    // rule body propagates as an error (NOT silently skipped — the API is a Result).
    #[tokio::test]
    async fn missing_rule_body_errors() {
        let stores = Stores::new().await;
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        let r2 = rule_with("r2", "/none/*.mcap", 0);
        stores
            .deployments
            .write_if_absent(
                "dpl".to_string(),
                Deployment {
                    id: "dpl".to_string(),
                    activity_status: DplActivity::Deployed,
                    release_id: "rel".to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();
        stores
            .releases
            .write_if_absent(
                "rel".to_string(),
                Release {
                    id: "rel".to_string(),
                    upload_rule_ids: vec![r1.id.clone(), r2.id.clone()],
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();
        stores
            .upload_rules
            .write_if_absent(r1.id.clone(), r1, |_, _| false)
            .await
            .unwrap();

        let deployed = stores.find_deployed().await.unwrap().expect("deployed");
        let err = stores
            .upload_rules_for_deployment(&deployed)
            .await
            .unwrap_err();
        // a missing body reads as CacheElementNotFound, wrapped as DiskErr::CacheErr.
        assert!(matches!(err, crate::disk::DiskErr::CacheErr(_)));
    }

    // A Deployed deployment whose release_id is not cached errors.
    #[tokio::test]
    async fn missing_release_errors() {
        let stores = Stores::new().await;
        stores
            .deployments
            .write_if_absent(
                "dpl".to_string(),
                Deployment {
                    id: "dpl".to_string(),
                    activity_status: DplActivity::Deployed,
                    release_id: "absent".to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();

        let deployed = stores.find_deployed().await.unwrap().expect("deployed");
        let err = stores
            .upload_rules_for_deployment(&deployed)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::disk::DiskErr::CacheErr(_)));
    }

    // A shut-down releases store makes upload_rules_for_deployment error (replacing the old
    // "treated as empty" assertion which no longer holds).
    #[tokio::test]
    async fn release_store_shutdown_errors() {
        let stores = Stores::new().await;
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;
        let deployed = stores.find_deployed().await.unwrap().expect("deployed");
        stores.releases.shutdown().await.unwrap();

        let err = stores
            .upload_rules_for_deployment(&deployed)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::disk::DiskErr::CacheErr(_)));
    }

    // A shut-down upload_rules store makes upload_rules_for_deployment error.
    #[tokio::test]
    async fn rule_store_shutdown_errors() {
        let stores = Stores::new().await;
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;
        let deployed = stores.find_deployed().await.unwrap().expect("deployed");
        stores.upload_rules.shutdown().await.unwrap();

        let err = stores
            .upload_rules_for_deployment(&deployed)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::disk::DiskErr::CacheErr(_)));
    }
}
