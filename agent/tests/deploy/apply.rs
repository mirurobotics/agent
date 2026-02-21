// internal crates
use crate::deploy::observer::HistoryObserver;
use miru_agent::deploy::apply::{apply, Args, Outcome};
use miru_agent::deploy::errors::DeployErr;
use miru_agent::deploy::fsm::RetryPolicy;
use miru_agent::deploy::observer::Observer;
use miru_agent::filesys::{dir::Dir, Overwrite};
use miru_agent::models::config_instance::ConfigInstance;
use miru_agent::models::deployment::{Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage;

struct Fixture {
    deployments: storage::Deployments,
    cfg_insts: storage::CfgInsts,
    cfg_inst_content: storage::CfgInstContent,
    staging_dir: Dir,
    target_dir: Dir,
    _temp_dir: Dir,
}

impl Fixture {
    async fn new() -> Self {
        let temp_dir = Dir::create_temp_dir("apply-test").await.unwrap();
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

        let staging_dir = temp_dir.subdir("staging");
        let target_dir = temp_dir.subdir("target");

        Self {
            deployments,
            cfg_insts,
            cfg_inst_content,
            staging_dir,
            target_dir,
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

    fn new_deployment(&self, cfg_insts: &[ConfigInstance]) -> Deployment {
        Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: cfg_insts.iter().map(|c| c.id.clone()).collect(),
            ..Default::default()
        }
    }

    async fn apply(&self) -> Result<Vec<Outcome>, DeployErr> {
        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &self.deployments,
            cfg_insts: &self.cfg_insts,
            contents: &self.cfg_inst_content,
            target_dir: &self.target_dir,
            staging_dir: &self.staging_dir,
            retry_policy: &retry_policy,
        };
        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        apply(&args, &mut observers).await
    }
}

pub mod apply_func {
    use super::*;

    #[tokio::test]
    async fn no_config_instances() {
        let f = Fixture::new().await;

        let deployment = f.new_deployment(&[]);
        f.seed_deployment(&deployment).await;

        let outcomes = f.apply().await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_some());
    }

    #[tokio::test]
    async fn deploy_1() {
        let f = Fixture::new().await;

        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"speed\": 4}".to_string())
            .await;

        let deployment = f.new_deployment(&[cfg_inst]);
        f.seed_deployment(&deployment).await;

        let outcomes = f.apply().await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_none());
        assert_eq!(
            outcomes[0].deployment.activity_status,
            DplActivity::Deployed
        );
        assert_eq!(outcomes[0].deployment.id, deployment.id);
    }

    #[tokio::test]
    async fn deploy_missing_content_causes_error_state() {
        let f = Fixture::new().await;

        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        // seed metadata but NOT content
        f.cfg_insts
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = f.new_deployment(&[cfg_inst]);
        f.seed_deployment(&deployment).await;

        let outcomes = f.apply().await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_some());
        assert_eq!(outcomes[0].deployment.id, deployment.id);
        assert_eq!(outcomes[0].deployment.error_status, DplErrStatus::Retrying);
    }

    #[tokio::test]
    async fn remove_deployment() {
        let f = Fixture::new().await;

        // target=Archived, activity=Deployed -> Remove
        let deployment = Deployment {
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Deployed,
            config_instance_ids: vec![],
            ..Default::default()
        };
        f.seed_deployment(&deployment).await;

        let outcomes = f.apply().await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_none());
        assert_eq!(
            outcomes[0].deployment.activity_status,
            DplActivity::Archived
        );
        assert_eq!(outcomes[0].deployment.id, deployment.id);
    }

    #[tokio::test]
    async fn archive_deployment() {
        let f = Fixture::new().await;

        // target=Staged, activity=Queued -> Archive
        let deployment = Deployment {
            target_status: DplTarget::Staged,
            activity_status: DplActivity::Queued,
            config_instance_ids: vec![],
            ..Default::default()
        };
        f.seed_deployment(&deployment).await;

        let outcomes = f.apply().await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_none());
        assert_eq!(
            outcomes[0].deployment.activity_status,
            DplActivity::Archived
        );
        assert_eq!(outcomes[0].deployment.id, deployment.id);
    }

    #[tokio::test]
    async fn stale_deployments_are_removed() {
        let f = Fixture::new().await;

        // Active deployment with content
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"key\": \"value\"}".to_string())
            .await;

        let active = Deployment {
            id: "active".to_string(),
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };
        // Stale deployments: target=Archived, activity=Deployed -> Remove
        let stale1 = Deployment {
            id: "stale-1".to_string(),
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Deployed,
            ..Default::default()
        };
        let stale2 = Deployment {
            id: "stale-2".to_string(),
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Deployed,
            ..Default::default()
        };

        for d in [&active, &stale1, &stale2] {
            f.seed_deployment(d).await;
        }

        let outcomes = f.apply().await.unwrap();

        // All 3 deployments should be processed
        assert_eq!(outcomes.len(), 3);

        let active_outcome = outcomes
            .iter()
            .find(|o| o.deployment.id == "active")
            .unwrap();
        let stale1_outcome = outcomes
            .iter()
            .find(|o| o.deployment.id == "stale-1")
            .unwrap();
        let stale2_outcome = outcomes
            .iter()
            .find(|o| o.deployment.id == "stale-2")
            .unwrap();

        assert!(active_outcome.error.is_none());
        assert_eq!(
            active_outcome.deployment.activity_status,
            DplActivity::Deployed
        );
        assert!(stale1_outcome.error.is_none());
        assert_eq!(
            stale1_outcome.deployment.activity_status,
            DplActivity::Archived
        );
        assert!(stale2_outcome.error.is_none());
        assert_eq!(
            stale2_outcome.deployment.activity_status,
            DplActivity::Archived
        );
    }
}
