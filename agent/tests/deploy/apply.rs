// internal crates
use miru_agent::deploy::apply::{self, apply, Outcome};
use miru_agent::deploy::fsm::RetryPolicy;
use miru_agent::deploy::DeployErr;
use miru_agent::filesys::{self, File, Overwrite, PathExt};
use miru_agent::models::{ConfigInstance, Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage;

// external crates
use chrono::TimeDelta;

// ================================= FIXTURE ===================================== //

struct Fixture {
    deployments: storage::Deployments,
    cfg_insts: storage::CfgInsts,
    cfg_inst_content: storage::CfgInstContent,
    temp_dir: filesys::Dir,
}

impl Fixture {
    async fn new() -> Self {
        let temp_dir = filesys::Dir::create_temp_dir("apply-test").await.unwrap();
        let resources_dir = temp_dir.subdir("resources");

        let (deployments, _) =
            storage::Deployments::spawn(16, resources_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        let (cfg_insts, _) = storage::CfgInsts::spawn(16, resources_dir.file("ci_meta.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content, _) =
            storage::CfgInstContent::spawn(16, resources_dir.subdir("content"), 1000)
                .await
                .unwrap();

        Self {
            deployments,
            cfg_insts,
            cfg_inst_content,
            temp_dir,
        }
    }

    /// Build an absolute filepath under the fixture's temp_dir for use as a
    /// `ConfigInstance.filepath` value.
    fn fixture_path(&self, rel: &str) -> String {
        self.temp_dir.path().join(rel).display().to_string()
    }

    async fn seed_cfg_inst(&self, cfg_inst: &ConfigInstance, content: String) {
        self.cfg_insts
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        self.cfg_inst_content
            .write(cfg_inst.id.clone(), content, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
    }

    async fn seed_cfg_inst_meta_only(&self, cfg_inst: &ConfigInstance) {
        self.cfg_insts
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
    }

    async fn seed_cfg_inst_content(&self, cfg_inst: &ConfigInstance, content: String) {
        self.cfg_inst_content
            .write(cfg_inst.id.clone(), content, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
    }

    async fn seed_deployment(&self, deployment: &Deployment) {
        self.deployments
            .write(
                deployment.id.clone(),
                deployment.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
    }

    fn storage(&self) -> apply::Storage<'_> {
        apply::Storage {
            deployments: &self.deployments,
            cfg_insts: storage::CfgInstRef {
                meta: &self.cfg_insts,
                content: &self.cfg_inst_content,
            },
        }
    }

    async fn apply(&self) -> Result<Vec<Outcome>, DeployErr> {
        let storage = self.storage();
        let opts = apply::DeployOpts {
            retry_policy: RetryPolicy::default(),
        };
        let args = apply::Args {
            storage: &storage,
            opts: &opts,
        };
        apply(&args).await
    }

    async fn apply_with_retry_policy(
        &self,
        retry_policy: RetryPolicy,
    ) -> Result<Vec<Outcome>, DeployErr> {
        let storage = self.storage();
        let opts = apply::DeployOpts { retry_policy };
        let args = apply::Args {
            storage: &storage,
            opts: &opts,
        };
        apply(&args).await
    }
}

// ================================= HELPERS ===================================== //

fn make_cfg_inst(filepath: String) -> ConfigInstance {
    ConfigInstance {
        filepath,
        ..Default::default()
    }
}

fn make_deployment(
    id: &str,
    target: DplTarget,
    activity: DplActivity,
    cfg_inst_ids: Vec<String>,
) -> Deployment {
    Deployment {
        id: id.to_string(),
        target_status: target,
        activity_status: activity,
        config_instance_ids: cfg_inst_ids,
        ..Default::default()
    }
}

/// Projects the subset of `Outcome` fields that tests care about into an
/// `assert_eq!`-friendly struct. `Outcome` itself cannot derive `PartialEq`
/// because `DeployErr` has non-comparable variants.
#[derive(Debug, PartialEq, Eq)]
struct ComparableOutcome {
    id: String,
    activity: DplActivity,
    error_status: DplErrStatus,
    attempts: u32,
    has_error: bool,
    has_wait: bool,
    in_cooldown: bool,
    transitioned: bool,
}

impl From<&Outcome> for ComparableOutcome {
    fn from(o: &Outcome) -> Self {
        Self {
            id: o.deployment.id.clone(),
            activity: o.deployment.activity_status,
            error_status: o.deployment.error_status,
            attempts: o.deployment.attempts,
            has_error: o.error.is_some(),
            has_wait: o.wait.is_some(),
            in_cooldown: o.deployment.is_in_cooldown(),
            transitioned: o.transitioned,
        }
    }
}

// ================================= TESTS ======================================= //

mod find_target_deployed {
    use super::*;

    #[tokio::test]
    async fn empty_store() {
        let f = Fixture::new().await;
        let outcomes = f.apply().await.unwrap();
        assert!(outcomes.is_empty());
    }

    #[tokio::test]
    async fn conflicting_deployments() {
        let f = Fixture::new().await;

        let ci1 = make_cfg_inst(f.fixture_path("a.json"));
        let ci2 = make_cfg_inst(f.fixture_path("b.json"));
        f.seed_cfg_inst(&ci1, "{}".into()).await;
        f.seed_cfg_inst(&ci2, "{}".into()).await;

        let d1 = make_deployment(
            "dpl-1",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci1.id.clone()],
        );
        let d2 = make_deployment(
            "dpl-2",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci2.id.clone()],
        );
        f.seed_deployment(&d1).await;
        f.seed_deployment(&d2).await;

        match f.apply().await {
            Ok(_) => panic!("expected ConflictingDeployments error, got Ok"),
            Err(DeployErr::ConflictingDeployments(e)) => {
                assert_eq!(e.ids.len(), 2);
                assert!(e.ids.contains(&"dpl-1".to_string()));
                assert!(e.ids.contains(&"dpl-2".to_string()));
            }
            Err(other) => panic!("expected ConflictingDeployments, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_deployment_excluded() {
        let f = Fixture::new().await;

        // target=Deployed but error_status=Failed -> excluded by find_target_deployed
        let mut dpl = make_deployment(
            "dpl-failed",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        dpl.error_status = DplErrStatus::Failed;
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        // find_target_deployed excludes Failed deployments, so None branch runs.
        // apply_actionables also skips it (FSM next_action == None for failed).
        assert!(outcomes.is_empty());
    }

    #[tokio::test]
    async fn failed_deployment_does_not_conflict_with_healthy() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("healthy.json"));
        f.seed_cfg_inst(&ci, "healthy-content".into()).await;

        // healthy: target=Deployed, activity=Queued -> should be picked and deployed
        let healthy = make_deployment(
            "dpl-healthy",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        // failed: target=Deployed, error_status=Failed -> excluded by find_target_deployed
        let mut failed = make_deployment(
            "dpl-failed",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        failed.error_status = DplErrStatus::Failed;

        f.seed_deployment(&healthy).await;
        f.seed_deployment(&failed).await;

        let outcomes = f.apply().await.unwrap();
        // find_target_deployed sees only the healthy one (no conflict).
        // The failed one is skipped by apply_actionables (FSM next_action == None).
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-healthy".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );

        let file = File::new(&ci.filepath);
        assert!(
            file.exists(),
            "healthy deployment file should exist on disk"
        );
        assert_eq!(file.read_string().await.unwrap(), "healthy-content");
    }
}

mod deploy_success {
    use super::*;

    #[tokio::test]
    async fn single_queued_to_deployed() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("test/filepath.json"));
        f.seed_cfg_inst(&ci, r#"{"speed": 4}"#.into()).await;

        let dpl = make_deployment(
            "dpl-1",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-1".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );

        // verify file on disk
        let file = File::new(&ci.filepath);
        assert!(file.exists(), "deployed file should exist on disk");
        let content = file.read_string().await.unwrap();
        assert_eq!(content, r#"{"speed": 4}"#);
    }

    #[tokio::test]
    async fn multiple_config_instances() {
        let f = Fixture::new().await;

        let ci1 = make_cfg_inst(f.fixture_path("a.json"));
        let ci2 = make_cfg_inst(f.fixture_path("b.yaml"));
        let ci3 = make_cfg_inst(f.fixture_path("nested/c.toml"));
        let ci4 = make_cfg_inst(f.fixture_path("a/super/nested/file.txt"));
        f.seed_cfg_inst(&ci1, "content-a".into()).await;
        f.seed_cfg_inst(&ci2, "content-b".into()).await;
        f.seed_cfg_inst(&ci3, "content-c".into()).await;
        f.seed_cfg_inst(&ci4, "content-d".into()).await;

        let dpl = make_deployment(
            "dpl-multi",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![
                ci1.id.clone(),
                ci2.id.clone(),
                ci3.id.clone(),
                ci4.id.clone(),
            ],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-multi".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );

        assert_eq!(
            File::new(&ci1.filepath).read_string().await.unwrap(),
            "content-a"
        );
        assert_eq!(
            File::new(&ci2.filepath).read_string().await.unwrap(),
            "content-b"
        );
        assert_eq!(
            File::new(&ci3.filepath).read_string().await.unwrap(),
            "content-c"
        );
        assert_eq!(
            File::new(&ci4.filepath).read_string().await.unwrap(),
            "content-d"
        );
    }

    #[tokio::test]
    async fn from_archived_activity() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("re-deploy.json"));
        f.seed_cfg_inst(&ci, "re-deployed".into()).await;

        // target=Deployed, activity=Archived -> FSM: Deploy
        let dpl = make_deployment(
            "dpl-redeploy",
            DplTarget::Deployed,
            DplActivity::Archived,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-redeploy".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    /// Full replacement scenario: deployment A has files [x, y], replaced by
    /// deployment B with files [y, z]. Verifies:
    /// - x (old-only) is deleted
    /// - y (shared) survives with new content
    /// - z (new-only) is created
    #[tokio::test]
    async fn replacement_removes_old_keeps_shared_creates_new() {
        let f = Fixture::new().await;

        let ci_old_only = make_cfg_inst(f.fixture_path("old-only.json"));
        let ci_shared = make_cfg_inst(f.fixture_path("shared.json"));
        let ci_new_only = make_cfg_inst(f.fixture_path("new-only.json"));
        f.seed_cfg_inst(&ci_old_only, r#"{"file": "x"}"#.into())
            .await;
        f.seed_cfg_inst(&ci_shared, r#"{"file": "y-old"}"#.into())
            .await;
        f.seed_cfg_inst(&ci_new_only, r#"{"file": "z"}"#.into())
            .await;

        // deploy A with [old-only, shared]
        let dpl_a = make_deployment(
            "dpl-a",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_old_only.id.clone(), ci_shared.id.clone()],
        );
        f.seed_deployment(&dpl_a).await;
        f.apply().await.unwrap();

        // update shared content for deployment B
        f.seed_cfg_inst_content(&ci_shared, r#"{"file": "y-new"}"#.into())
            .await;

        // deployment B wants [shared, new-only]; deployment A is being removed
        let dpl_b = make_deployment(
            "dpl-b",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_shared.id.clone(), ci_new_only.id.clone()],
        );
        let dpl_a_remove = make_deployment(
            "dpl-a",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci_old_only.id.clone(), ci_shared.id.clone()],
        );
        f.seed_deployment(&dpl_b).await;
        f.seed_deployment(&dpl_a_remove).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 2);

        let old_only = File::new(&ci_old_only.filepath);
        let shared = File::new(&ci_shared.filepath);
        let new_only = File::new(&ci_new_only.filepath);

        // old-only file deleted
        assert!(
            !old_only.path().exists(),
            "old-only file should be deleted after replacement"
        );

        // shared file survives with updated content
        assert!(shared.path().exists(), "shared file should survive");
        let content = shared.read_string().await.unwrap();
        assert_eq!(content, r#"{"file": "y-new"}"#);

        // new-only file created
        assert!(new_only.path().exists(), "new-only file should be created");
        let content = new_only.read_string().await.unwrap();
        assert_eq!(content, r#"{"file": "z"}"#);
    }
}

mod deploy_errors {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn empty_config_instances() {
        let f = Fixture::new().await;

        // to deploy
        let dpl = make_deployment(
            "dpl-empty",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        // to remove -- used to ensure that deployment is not removed in errors
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-empty".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(
            outcomes[0].error,
            Some(DeployErr::EmptyConfigInstances(_))
        ));
    }

    #[tokio::test]
    async fn missing_config_instance_metadata() {
        let f = Fixture::new().await;

        // reference a cfg_inst ID that was never seeded
        let dpl = make_deployment(
            "dpl-missing-meta",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec!["non-existent-ci-id".to_string()],
        );
        f.seed_deployment(&dpl).await;

        // to remove -- used to ensure that deployment is not removed in errors
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        // apply returns Err because read_cfg_insts for the deployed_files list
        // fails before any deployment is attempted
        let result = f.apply().await;
        assert!(matches!(result, Err(DeployErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn missing_content() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("test/filepath.json"));
        // seed metadata but NOT content
        f.seed_cfg_inst_meta_only(&ci).await;

        let dpl = make_deployment(
            "dpl-no-content",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        // to remove -- used to ensure that deployment is not removed in errors
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-no-content".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(outcomes[0].error, Some(DeployErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn deploy_error_preserves_old_deployment_files() {
        let f = Fixture::new().await;

        // Deploy an old deployment with a real config instance file on disk
        let ci_old = make_cfg_inst(f.fixture_path("old-config.json"));
        f.seed_cfg_inst(&ci_old, r#"{"old": true}"#.into()).await;

        let dpl_old = make_deployment(
            "dpl-old",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_old.id.clone()],
        );
        f.seed_deployment(&dpl_old).await;
        f.apply().await.unwrap();

        let old_file = File::new(&ci_old.filepath);
        assert!(
            old_file.exists(),
            "old file should exist after initial deploy"
        );

        // Seed a new deployment that will fail: metadata only, no content -> CacheErr
        let ci_new = make_cfg_inst(f.fixture_path("new-config.json"));
        f.seed_cfg_inst_meta_only(&ci_new).await;

        let dpl_new = make_deployment(
            "dpl-new",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_new.id.clone()],
        );
        f.seed_deployment(&dpl_new).await;

        // Seed the old deployment as target=Archived, activity=Deployed for removal
        let dpl_old_remove = make_deployment(
            "dpl-old",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci_old.id.clone()],
        );
        f.seed_deployment(&dpl_old_remove).await;

        let outcomes = f.apply().await.unwrap();

        // Only the failing new deployment should produce an outcome; the old
        // deployment's removal is skipped because the deploy phase errored.
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-new".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(outcomes[0].error, Some(DeployErr::CacheErr(_))));

        // The old deployment's file must still exist on disk
        assert!(
            old_file.exists(),
            "old deployment file should survive when the new deploy fails"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn config_instance_write_permission_denied() {
        let f = Fixture::new().await;

        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        locked_dir
            .set_permissions(std::fs::Permissions::from_mode(0o555))
            .await
            .unwrap();

        let ci = make_cfg_inst(locked_dir.file("config.json").path().display().to_string());
        f.seed_cfg_inst(&ci, r#"{"locked": true}"#.into()).await;

        let dpl = make_deployment(
            "dpl-inaccessible-dest",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        // to remove -- used to ensure that deployment is not removed in errors
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        let outcomes = f.apply().await.unwrap();

        locked_dir
            .set_permissions(std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-inaccessible-dest".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(
            outcomes[0].error,
            Some(DeployErr::WriteAccessDenied(_))
        ));
    }

    #[tokio::test]
    async fn error_bumps_attempts_and_sets_cooldown() {
        let f = Fixture::new().await;

        let mut dpl = make_deployment(
            "dpl-attempts",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        dpl.attempts = 3;
        f.seed_deployment(&dpl).await;

        // to remove -- used to ensure that deployment is not removed in errors
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-attempts".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 4,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(
            outcomes[0].error,
            Some(DeployErr::EmptyConfigInstances(_))
        ));
    }

    #[tokio::test]
    async fn max_retries_exceeded_enters_failed() {
        let f = Fixture::new().await;

        let dpl = make_deployment(
            "dpl-maxed",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        // to remove -- used to ensure that deployment is not removed in errors
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        let policy = RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        };

        let outcomes = f.apply_with_retry_policy(policy).await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-maxed".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Failed,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(
            outcomes[0].error,
            Some(DeployErr::EmptyConfigInstances(_))
        ));
    }

    #[tokio::test]
    async fn archives_processed_on_deploy_failure() {
        let f = Fixture::new().await;

        // Target deployment that will fail (no config instance content)
        let dpl = make_deployment(
            "dpl-target",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        // Old deployment needing archive: (Archived, Queued) → FSM: Archive
        f.seed_deployment(&make_deployment(
            "dpl-to-archive",
            DplTarget::Archived,
            DplActivity::Queued,
            vec![],
        ))
        .await;

        let outcomes = f.apply().await.unwrap();

        assert_eq!(outcomes.len(), 2);
        // First outcome: the failed target
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-target".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        // Second outcome: the old deployment was archived
        assert_eq!(
            ComparableOutcome::from(&outcomes[1]),
            ComparableOutcome {
                id: "dpl-to-archive".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    #[tokio::test]
    async fn removals_blocked_but_archives_proceed_on_deploy_failure() {
        let f = Fixture::new().await;

        // Target deployment that will fail (no config instance content)
        let dpl = make_deployment(
            "dpl-target",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        // Old deployment needing archive: (Archived, Queued) → FSM: Archive
        f.seed_deployment(&make_deployment(
            "dpl-to-archive",
            DplTarget::Archived,
            DplActivity::Queued,
            vec![],
        ))
        .await;

        // Old deployment needing removal: (Archived, Deployed) → FSM: Remove
        // This should be skipped because removals are unsafe when target failed
        f.seed_deployment(&make_deployment(
            "dpl-to-remove",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        ))
        .await;

        let outcomes = f.apply().await.unwrap();

        // Only 2 outcomes: failed target + archived. Removal is skipped.
        assert_eq!(outcomes.len(), 2);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-target".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert_eq!(
            ComparableOutcome::from(&outcomes[1]),
            ComparableOutcome {
                id: "dpl-to-archive".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }
}

mod remove_action {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn archived_target_deployed_activity() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("remove-me.json"));
        f.seed_cfg_inst(&ci, r#"{"old": true}"#.into()).await;

        // deploy first so file exists on disk
        let deploy_dpl = make_deployment(
            "dpl-deploy-first",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&deploy_dpl).await;
        f.apply().await.unwrap();

        // now mark it for removal: target=Archived, activity=Deployed -> FSM: Remove
        let dpl = make_deployment(
            "dpl-deploy-first",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let dest = File::new(&ci.filepath);
        assert!(dest.path().exists(), "file should exist before removal");

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-deploy-first".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
        assert!(
            !dest.path().exists(),
            "file should be deleted after removal"
        );
    }

    #[tokio::test]
    async fn staged_target_deployed_activity() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("staged-remove.json"));
        f.seed_cfg_inst(&ci, r#"{"staged": true}"#.into()).await;

        // deploy first so file exists on disk
        let deploy_dpl = make_deployment(
            "dpl-staged-remove",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&deploy_dpl).await;
        f.apply().await.unwrap();

        let dest = File::new(&ci.filepath);
        assert!(
            dest.path().exists(),
            "file should exist after initial deploy"
        );

        // Re-seed as target=Staged, activity=Deployed -> FSM: Remove
        let dpl = make_deployment(
            "dpl-staged-remove",
            DplTarget::Staged,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-staged-remove".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
        assert!(
            !dest.path().exists(),
            "file should be deleted after staged removal"
        );
    }

    #[tokio::test]
    async fn empty_config_instances_is_noop() {
        let f = Fixture::new().await;

        // target=Archived, activity=Deployed, no config instances -> FSM: Remove (no-op)
        let dpl = make_deployment(
            "dpl-empty",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-empty".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    #[tokio::test]
    async fn remove_error_enters_retry() {
        let f = Fixture::new().await;

        // cfg_inst with a relative filepath triggers PathNotAllowed in remove
        let ci = ConfigInstance {
            filepath: "relative/path.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst_meta_only(&ci).await;

        // target=Archived, activity=Deployed -> FSM: Remove
        let dpl = make_deployment(
            "dpl-remove-err",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-remove-err".into(),
                activity: DplActivity::Removing,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );
        assert!(matches!(
            outcomes[0].error,
            Some(DeployErr::PathNotAllowed(_))
        ));
    }

    #[tokio::test]
    async fn dont_remove_protects_shared_files() {
        let f = Fixture::new().await;

        // shared config instance used by both old and new deployment
        let ci_shared = make_cfg_inst(f.fixture_path("shared.json"));
        f.seed_cfg_inst(&ci_shared, r#"{"shared": true}"#.into())
            .await;
        // config instance only in the old deployment
        let ci_old = make_cfg_inst(f.fixture_path("old-only.json"));
        f.seed_cfg_inst(&ci_old, r#"{"old": true}"#.into()).await;

        // deploy old deployment first so both files exist on disk
        let old_deploy = make_deployment(
            "dpl-old",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_shared.id.clone(), ci_old.id.clone()],
        );
        f.seed_deployment(&old_deploy).await;
        f.apply().await.unwrap();

        // now: new deployment wants to be deployed (uses shared config),
        // old deployment is being removed
        let new_dpl = make_deployment(
            "dpl-new",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_shared.id.clone()],
        );
        let old_dpl = make_deployment(
            "dpl-old",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci_shared.id.clone(), ci_old.id.clone()],
        );
        f.seed_deployment(&new_dpl).await;
        f.seed_deployment(&old_dpl).await;

        let shared_dest = File::new(&ci_shared.filepath);
        let old_dest = File::new(&ci_old.filepath);

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 2);

        // new deployment should deploy successfully
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-new".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );

        // old deployment should be archived
        assert_eq!(
            ComparableOutcome::from(&outcomes[1]),
            ComparableOutcome {
                id: "dpl-old".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );

        // shared file should survive (protected by dont_remove)
        assert!(
            shared_dest.path().exists(),
            "shared file should be protected from deletion"
        );
        // old-only file should be deleted
        assert!(!old_dest.path().exists(), "old-only file should be deleted");
    }

    #[tokio::test]
    async fn already_removing_transitions_to_archived() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("removing.json"));
        f.seed_cfg_inst(&ci, r#"{"removing": true}"#.into()).await;

        // deploy first so file exists on disk
        let deploy_dpl = make_deployment(
            "dpl-removing",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&deploy_dpl).await;
        f.apply().await.unwrap();

        // now set target=Archived, activity=Removing (already in removing state)
        let dpl = Deployment {
            id: "dpl-removing".to_string(),
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Removing,
            config_instance_ids: vec![ci.id.clone()],
            ..Default::default()
        };
        f.seed_deployment(&dpl).await;

        let dest = File::new(&ci.filepath);
        assert!(dest.path().exists(), "file should exist before removal");

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-removing".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
        assert!(
            !dest.path().exists(),
            "file should be deleted after removal"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_io_error_permission_denied() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("remove-locked/config.json"));
        f.seed_cfg_inst(&ci, r#"{"locked": true}"#.into()).await;

        // deploy first so file exists on disk
        let deploy_dpl = make_deployment(
            "dpl-remove-locked",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&deploy_dpl).await;
        f.apply().await.unwrap();

        let dest = File::new(&ci.filepath);
        assert!(
            dest.path().exists(),
            "file should exist after initial deploy"
        );

        // Lock the parent directory so removal fails with EACCES
        let parent_dir = dest.parent().unwrap();
        parent_dir
            .set_permissions(std::fs::Permissions::from_mode(0o555))
            .await
            .unwrap();

        // Seed deployment as target=Archived, activity=Deployed for removal
        let dpl = make_deployment(
            "dpl-remove-locked",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();

        // Restore permissions BEFORE assertions so tempdir cleanup succeeds
        parent_dir
            .set_permissions(std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-remove-locked".into(),
                activity: DplActivity::Removing,
                error_status: DplErrStatus::Retrying,
                attempts: 1,
                has_error: true,
                has_wait: true,
                in_cooldown: true,
                transitioned: true,
            }
        );

        // File should still exist since removal failed
        assert!(
            dest.path().exists(),
            "file should still exist after permission-denied removal"
        );
    }
}

mod archive_action {
    use super::*;

    #[tokio::test]
    async fn staged_target_queued_activity() {
        let f = Fixture::new().await;

        let dpl = make_deployment(
            "dpl-archive-1",
            DplTarget::Staged,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-archive-1".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    #[tokio::test]
    async fn archived_target_queued_activity() {
        let f = Fixture::new().await;

        let dpl = make_deployment(
            "dpl-archive-2",
            DplTarget::Archived,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-archive-2".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    #[tokio::test]
    async fn archived_target_drifted_activity() {
        let f = Fixture::new().await;

        let dpl = make_deployment(
            "dpl-archive-3",
            DplTarget::Archived,
            DplActivity::Drifted,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-archive-3".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    #[tokio::test]
    async fn archived_target_staged_activity() {
        let f = Fixture::new().await;

        let dpl = make_deployment(
            "dpl-archive-4",
            DplTarget::Archived,
            DplActivity::Staged,
            vec![],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-archive-4".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }
}

mod no_action {
    use super::*;

    #[tokio::test]
    async fn non_actionable_deployments_skipped() {
        let f = Fixture::new().await;

        // target=Staged, activity=Staged -> FSM: None (not actionable)
        let d1 = make_deployment("dpl-noop-1", DplTarget::Staged, DplActivity::Staged, vec![]);
        let d2 = make_deployment("dpl-noop-2", DplTarget::Staged, DplActivity::Staged, vec![]);
        f.seed_deployment(&d1).await;
        f.seed_deployment(&d2).await;

        let outcomes = f.apply().await.unwrap();
        // Neither targets Deployed, so find_target_deployed returns None.
        // apply_actionables filters out NextAction::None deployments.
        assert!(outcomes.is_empty());
    }

    #[tokio::test]
    async fn steady_state_deployed_is_noop() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("already-deployed.json"));
        f.seed_cfg_inst(&ci, "content".into()).await;

        // target=Deployed, activity=Deployed -> FSM: None (already at target)
        let dpl = make_deployment(
            "dpl-steady",
            DplTarget::Deployed,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        // find_target_deployed selects this deployment and passes it directly
        // to apply_one, which returns a no-op outcome (transitioned: false).
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-steady".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: false,
            }
        );
    }
}

mod wait_action {
    use super::*;

    #[tokio::test]
    async fn target_deployed_in_cooldown() {
        let f = Fixture::new().await;

        let mut dpl = make_deployment("dpl-wait", DplTarget::Deployed, DplActivity::Queued, vec![]);
        dpl.set_cooldown(TimeDelta::minutes(60));
        f.seed_deployment(&dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-wait".into(),
                activity: DplActivity::Queued,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: true,
                in_cooldown: true,
                transitioned: false,
            }
        );
        let wait = outcomes[0].wait.unwrap();
        assert!(
            wait.num_seconds() > 3500,
            "wait should be ~60 min, got {}s",
            wait.num_seconds()
        );
    }

    #[tokio::test]
    async fn actionable_in_cooldown() {
        let f = Fixture::new().await;

        // target=Archived, activity=Deployed -> FSM: Remove, but in cooldown -> Wait
        let mut dpl = make_deployment(
            "dpl-wait-act",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        );
        dpl.set_cooldown(TimeDelta::minutes(60));
        f.seed_deployment(&dpl).await;

        // No deployment targets Deployed, so find_target_deployed returns None,
        // and this goes through apply_actionables.
        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-wait-act".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: true,
                in_cooldown: true,
                transitioned: false,
            }
        );
        let wait = outcomes[0].wait.unwrap();
        assert!(wait.num_seconds() > 3500);
    }
}

mod ordering_and_composition {
    use super::*;

    #[tokio::test]
    async fn deployed_target_first_then_actionables() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("active.json"));
        f.seed_cfg_inst(&ci, "active-content".into()).await;

        let dpl_active = make_deployment(
            "dpl-active",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        let dpl_stale = make_deployment(
            "dpl-stale",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        );
        f.seed_deployment(&dpl_active).await;
        f.seed_deployment(&dpl_stale).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 2);
        // Ordering guarantee: deployed target always first
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-active".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
        assert_eq!(
            ComparableOutcome::from(&outcomes[1]),
            ComparableOutcome {
                id: "dpl-stale".into(),
                activity: DplActivity::Archived,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );
    }

    #[tokio::test]
    async fn stale_removed_alongside_active() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst(f.fixture_path("active.json"));
        f.seed_cfg_inst(&ci, "active-content".into()).await;

        let active = make_deployment(
            "active",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        let stale1 = make_deployment(
            "stale-1",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        );
        let stale2 = make_deployment(
            "stale-2",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![],
        );

        for d in [&active, &stale1, &stale2] {
            f.seed_deployment(d).await;
        }

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 3);

        // Active is always first
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "active".into(),
                activity: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                attempts: 0,
                has_error: false,
                has_wait: false,
                in_cooldown: false,
                transitioned: true,
            }
        );

        let stale_expected = ComparableOutcome {
            id: String::new(), // checked separately per stale
            activity: DplActivity::Archived,
            error_status: DplErrStatus::None,
            attempts: 0,
            has_error: false,
            has_wait: false,
            in_cooldown: false,
            transitioned: true,
        };
        let stale_outcomes: Vec<_> = outcomes
            .iter()
            .filter(|o| o.deployment.id.starts_with("stale"))
            .collect();
        assert_eq!(stale_outcomes.len(), 2);
        for o in stale_outcomes {
            let mut actual = ComparableOutcome::from(o);
            actual.id = String::new(); // normalize for comparison
            assert_eq!(actual, stale_expected);
        }
    }
}
