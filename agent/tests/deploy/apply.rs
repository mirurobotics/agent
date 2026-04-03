// internal crates
use miru_agent::deploy::apply::{self, apply, Outcome};
use miru_agent::deploy::fsm::RetryPolicy;
use miru_agent::deploy::DeployErr;
use miru_agent::filesys::{self, Overwrite, PathExt};
use miru_agent::models::{ConfigInstance, Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage;

// external crates
use chrono::TimeDelta;

// ================================= FIXTURE ===================================== //

struct Fixture {
    deployments: storage::Deployments,
    cfg_insts: storage::CfgInsts,
    cfg_inst_content: storage::CfgInstContent,
    filesys_root: filesys::Dir,
    _temp_dir: filesys::Dir,
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

        let filesys_root = temp_dir.clone();

        Self {
            deployments,
            cfg_insts,
            cfg_inst_content,
            filesys_root,
            _temp_dir: temp_dir,
        }
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
            filesys_root: self.filesys_root.clone(),
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
        let opts = apply::DeployOpts {
            filesys_root: self.filesys_root.clone(),
            retry_policy,
        };
        let args = apply::Args {
            storage: &storage,
            opts: &opts,
        };
        apply(&args).await
    }
}

// ================================= HELPERS ===================================== //

fn make_cfg_inst(filepath: &str) -> ConfigInstance {
    ConfigInstance {
        filepath: filepath.to_string(),
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

        let ci1 = make_cfg_inst("/a.json");
        let ci2 = make_cfg_inst("/b.json");
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

        let ci = make_cfg_inst("/healthy.json");
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

        let file = f.filesys_root.file("/healthy.json");
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

        let ci = make_cfg_inst("/test/filepath.json");
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
        let file = f.filesys_root.file("/test/filepath.json");
        assert!(file.exists(), "deployed file should exist on disk");
        let content = file.read_string().await.unwrap();
        assert_eq!(content, r#"{"speed": 4}"#);
    }

    #[tokio::test]
    async fn multiple_config_instances() {
        let f = Fixture::new().await;

        let ci1 = make_cfg_inst("/a.json");
        let ci2 = make_cfg_inst("/b.yaml");
        let ci3 = make_cfg_inst("/nested/c.toml");
        f.seed_cfg_inst(&ci1, "content-a".into()).await;
        f.seed_cfg_inst(&ci2, "content-b".into()).await;
        f.seed_cfg_inst(&ci3, "content-c".into()).await;

        let dpl = make_deployment(
            "dpl-multi",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci1.id.clone(), ci2.id.clone(), ci3.id.clone()],
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
            f.filesys_root.file("/a.json").read_string().await.unwrap(),
            "content-a"
        );
        assert_eq!(
            f.filesys_root.file("/b.yaml").read_string().await.unwrap(),
            "content-b"
        );
        assert_eq!(
            f.filesys_root
                .file("/nested/c.toml")
                .read_string()
                .await
                .unwrap(),
            "content-c"
        );
    }

    #[tokio::test]
    async fn from_archived_activity() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst("/re-deploy.json");
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
}

mod deploy_errors {
    use super::*;

    #[tokio::test]
    async fn empty_config_instances() {
        let f = Fixture::new().await;

        let dpl = make_deployment(
            "dpl-empty",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![],
        );
        f.seed_deployment(&dpl).await;

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

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            ComparableOutcome::from(&outcomes[0]),
            ComparableOutcome {
                id: "dpl-missing-meta".into(),
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
    async fn missing_content() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst("/test/filepath.json");
        // seed metadata but NOT content
        f.seed_cfg_inst_meta_only(&ci).await;

        let dpl = make_deployment(
            "dpl-no-content",
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
}

mod remove_action {
    use super::*;

    #[tokio::test]
    async fn archived_target_deployed_activity() {
        let f = Fixture::new().await;

        // target=Archived, activity=Deployed -> FSM: Remove
        let dpl = make_deployment(
            "dpl-remove",
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
                id: "dpl-remove".into(),
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
    async fn staged_target_deployed_activity() {
        let f = Fixture::new().await;

        // target=Staged, activity=Deployed -> FSM: Remove
        let dpl = make_deployment(
            "dpl-staged-remove",
            DplTarget::Staged,
            DplActivity::Deployed,
            vec![],
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
    }

    #[tokio::test]
    async fn removes_config_files_from_disk() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst("/test/stale.json");
        f.seed_cfg_inst(&ci, "stale-content".into()).await;

        // First deploy the old deployment so files exist on disk
        let old_dpl = make_deployment(
            "dpl-old",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&old_dpl).await;
        f.apply().await.unwrap();
        assert!(f.filesys_root.file("/test/stale.json").exists());

        // Now set it to archived so it gets removed
        let old_dpl = make_deployment(
            "dpl-old",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&old_dpl).await;

        let outcomes = f.apply().await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].deployment.activity_status,
            DplActivity::Archived
        );

        // File should be deleted since no deployment wants it
        assert!(
            !f.filesys_root.file("/test/stale.json").exists(),
            "stale file should be removed when deployment is archived"
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

        let ci = make_cfg_inst("/already-deployed.json");
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

        let ci = make_cfg_inst("/active.json");
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

        let ci = make_cfg_inst("/active.json");
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

mod stale_file_cleanup {
    use super::*;

    #[tokio::test]
    async fn new_deploy_removes_old_files() {
        let f = Fixture::new().await;

        let ci_old = make_cfg_inst("/test/old.json");
        let ci_shared = make_cfg_inst("/test/shared.json");
        let ci_new = make_cfg_inst("/test/new.json");
        f.seed_cfg_inst(&ci_old, "old".into()).await;
        f.seed_cfg_inst(&ci_shared, "shared-v1".into()).await;
        f.seed_cfg_inst(&ci_new, "new".into()).await;

        // Deploy old deployment with {old, shared}
        let old_dpl = make_deployment(
            "dpl-old",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_old.id.clone(), ci_shared.id.clone()],
        );
        f.seed_deployment(&old_dpl).await;
        f.apply().await.unwrap();

        assert!(f.filesys_root.file("/test/old.json").exists());
        assert!(f.filesys_root.file("/test/shared.json").exists());

        // Now deploy new deployment with {shared, new} and archive old
        let new_dpl = make_deployment(
            "dpl-new",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci_shared.id.clone(), ci_new.id.clone()],
        );
        let old_dpl_archived = make_deployment(
            "dpl-old",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci_old.id.clone(), ci_shared.id.clone()],
        );
        f.seed_deployment(&new_dpl).await;
        f.seed_deployment(&old_dpl_archived).await;

        // Update shared content for new deployment
        f.cfg_inst_content
            .write(
                ci_shared.id.clone(),
                "shared-v2".into(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        f.apply().await.unwrap();

        // old.json should be deleted (was only in old deployment)
        assert!(
            !f.filesys_root.file("/test/old.json").exists(),
            "old file should be removed"
        );
        // shared.json should still exist (in keep set from new deployment)
        assert!(
            f.filesys_root.file("/test/shared.json").exists(),
            "shared file should be preserved"
        );
        // new.json should exist
        assert!(
            f.filesys_root.file("/test/new.json").exists(),
            "new file should be deployed"
        );
    }

    #[tokio::test]
    async fn remove_without_new_deploy_deletes_all_files() {
        let f = Fixture::new().await;

        let ci = make_cfg_inst("/test/remove-all.json");
        f.seed_cfg_inst(&ci, "content".into()).await;

        // Deploy
        let dpl = make_deployment(
            "dpl-remove-all",
            DplTarget::Deployed,
            DplActivity::Queued,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;
        f.apply().await.unwrap();
        assert!(f.filesys_root.file("/test/remove-all.json").exists());

        // Archive (no new deployment)
        let dpl = make_deployment(
            "dpl-remove-all",
            DplTarget::Archived,
            DplActivity::Deployed,
            vec![ci.id.clone()],
        );
        f.seed_deployment(&dpl).await;
        f.apply().await.unwrap();

        assert!(
            !f.filesys_root.file("/test/remove-all.json").exists(),
            "all files should be removed when no new deployment exists"
        );
    }
}
