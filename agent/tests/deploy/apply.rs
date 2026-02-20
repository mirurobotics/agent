// internal crates
use crate::deploy::observer::HistoryObserver;
use miru_agent::cache::file::FileCache;
use miru_agent::deploy::observer::Observer;
use miru_agent::deploy::{
    apply::{apply, Args},
    fsm::RetryPolicy,
};
use miru_agent::filesys::{dir::Dir, Overwrite};
use miru_agent::models::config_instance::ConfigInstance;
use miru_agent::models::deployment::{Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage::Deployments;

// external crates
use serde_json::json;

pub mod apply_func {
    use super::*;

    #[tokio::test]
    async fn no_config_instances() {
        let temp_dir = Dir::create_temp_dir("apply").await.unwrap();
        let staging_dir = Dir::create_temp_dir("apply_staging").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (deployment_cache, _) =
            Deployments::spawn(16, cache_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: vec![],
            ..Default::default()
        };
        deployment_cache
            .write(
                deployment.id.clone(),
                deployment.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let target_dir = temp_dir.subdir("target");
        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &deployment_cache,
            cfg_insts: &ci_meta_cache,
            contents: &content_cache,
            target_dir: &target_dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };

        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let outcomes = apply(&args, &mut observers).await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_some());
    }

    #[tokio::test]
    async fn deploy_1() {
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };

        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let staging_dir = Dir::create_temp_dir("deploy_staging").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (deployment_cache, _) =
            Deployments::spawn(16, cache_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        ci_meta_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        content_cache
            .write(
                cfg_inst.id.clone(),
                json!({"speed": 4}),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };
        deployment_cache
            .write(
                deployment.id.clone(),
                deployment.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let target_dir = temp_dir.subdir("target");
        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &deployment_cache,
            cfg_insts: &ci_meta_cache,
            contents: &content_cache,
            target_dir: &target_dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };

        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let outcomes = apply(&args, &mut observers).await.unwrap();

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
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };

        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let staging_dir = Dir::create_temp_dir("deploy_staging").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (deployment_cache, _) =
            Deployments::spawn(16, cache_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        // metadata present but NO content
        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        ci_meta_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };
        deployment_cache
            .write(
                deployment.id.clone(),
                deployment.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &deployment_cache,
            cfg_insts: &ci_meta_cache,
            contents: &content_cache,
            target_dir: &temp_dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };

        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let outcomes = apply(&args, &mut observers).await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_some());
        assert_eq!(outcomes[0].deployment.id, deployment.id);
        assert_eq!(outcomes[0].deployment.error_status, DplErrStatus::Retrying);
    }

    #[tokio::test]
    async fn remove_deployment() {
        let temp_dir = Dir::create_temp_dir("remove").await.unwrap();
        let staging_dir = Dir::create_temp_dir("remove_staging").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (deployment_cache, _) =
            Deployments::spawn(16, cache_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        // deployment targeting removal (currently deployed, target archived)
        // FSM: target=Archived, activity=Deployed -> Remove
        let deployment = Deployment {
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Deployed,
            config_instance_ids: vec![],
            ..Default::default()
        };
        deployment_cache
            .write(
                deployment.id.clone(),
                deployment.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &deployment_cache,
            cfg_insts: &ci_meta_cache,
            contents: &content_cache,
            target_dir: &temp_dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };

        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let outcomes = apply(&args, &mut observers).await.unwrap();

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
        let temp_dir = Dir::create_temp_dir("archive").await.unwrap();
        let staging_dir = Dir::create_temp_dir("archive_staging").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (deployment_cache, _) =
            Deployments::spawn(16, cache_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        // FSM: target=Staged, activity=Queued -> Archive
        let deployment = Deployment {
            target_status: DplTarget::Staged,
            activity_status: DplActivity::Queued,
            config_instance_ids: vec![],
            ..Default::default()
        };
        deployment_cache
            .write(
                deployment.id.clone(),
                deployment.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &deployment_cache,
            cfg_insts: &ci_meta_cache,
            contents: &content_cache,
            target_dir: &temp_dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };

        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let outcomes = apply(&args, &mut observers).await.unwrap();

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
        let temp_dir = Dir::create_temp_dir("stale").await.unwrap();
        let staging_dir = Dir::create_temp_dir("stale_staging").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (deployment_cache, _) =
            Deployments::spawn(16, cache_dir.file("deployments.json"), 1000)
                .await
                .unwrap();
        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        // Active deployment: target=Deployed, activity=Queued -> Deploy
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        ci_meta_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        content_cache
            .write(
                cfg_inst.id.clone(),
                serde_json::json!({"key": "value"}),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
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

        // Write all deployments to cache
        for d in [&active, &stale1, &stale2] {
            deployment_cache
                .write(d.id.clone(), d.clone(), |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
        }

        let target_dir = temp_dir.subdir("target");
        let retry_policy = RetryPolicy::default();
        let args = Args {
            deployments: &deployment_cache,
            cfg_insts: &ci_meta_cache,
            contents: &content_cache,
            target_dir: &target_dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };

        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let outcomes = apply(&args, &mut observers).await.unwrap();

        // All 3 deployments should be processed
        assert_eq!(outcomes.len(), 3);

        // Find outcomes by id
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
