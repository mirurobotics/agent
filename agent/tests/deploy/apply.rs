// internal crates
use miru_agent::cache::entry::CacheEntry;
use miru_agent::deploy::{
    apply::{apply, is_dirty},
    filesys::DeployContext,
    fsm::RetryPolicy,
};
use miru_agent::filesys::{dir::Dir, Overwrite};
use miru_agent::models::config_instance::ConfigInstance;
use miru_agent::models::deployment::{
    Deployment, DeploymentActivityStatus, DeploymentErrorStatus, DeploymentTargetStatus,
};
use miru_agent::storage::config_instances::{ConfigInstanceCache, ConfigInstanceContentCache};
use miru_agent::storage::deployments::DeploymentCache;

// external crates
use chrono::Utc;
use serde_json::json;

pub mod is_dirty_func {
    use super::*;

    #[tokio::test]
    async fn no_changes() {
        let deployment = Deployment {
            ..Default::default()
        };
        let entry = CacheEntry {
            key: deployment.id.clone(),
            value: deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(!is_dirty(old, &deployment));
    }

    #[tokio::test]
    async fn previous_is_none() {
        let deployment = Deployment {
            ..Default::default()
        };
        assert!(is_dirty(None, &deployment));
    }

    #[tokio::test]
    async fn previously_dirty() {
        let deployment = Deployment {
            ..Default::default()
        };
        let entry = CacheEntry {
            key: deployment.id.clone(),
            value: deployment.clone(),
            is_dirty: true,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &deployment));
    }

    #[tokio::test]
    async fn activity_status_changed() {
        let old_deployment = Deployment {
            activity_status: DeploymentActivityStatus::Queued,
            ..Default::default()
        };
        let new_deployment = Deployment {
            id: old_deployment.id.clone(),
            activity_status: DeploymentActivityStatus::Deployed,
            ..Default::default()
        };
        let entry = CacheEntry {
            key: old_deployment.id.clone(),
            value: old_deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &new_deployment));
    }

    #[tokio::test]
    async fn error_status_changed() {
        let old_deployment = Deployment {
            error_status: DeploymentErrorStatus::None,
            ..Default::default()
        };
        let new_deployment = Deployment {
            id: old_deployment.id.clone(),
            error_status: DeploymentErrorStatus::Retrying,
            ..Default::default()
        };
        let entry = CacheEntry {
            key: old_deployment.id.clone(),
            value: old_deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &new_deployment));
    }
}

pub mod apply_func {
    use super::*;

    #[tokio::test]
    async fn no_config_instances() {
        let dir = Dir::create_temp_dir("apply").await.unwrap();
        let staging_dir = Dir::create_temp_dir("apply_staging").await.unwrap();
        let (deployment_cache, _) = DeploymentCache::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_cache, _) = ConfigInstanceCache::spawn(16, dir.file("metadata.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content_cache, _) = ConfigInstanceContentCache::spawn(16, dir.clone(), 1000)
            .await
            .unwrap();

        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![],
            ..Default::default()
        };

        let retry_policy = RetryPolicy::default();
        let ctx = DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };
        let result = apply(&deployment, &deployment_cache, &cfg_inst_cache, &ctx)
            .await
            .unwrap();

        // With no config instances, the deployment is returned as-is
        assert_eq!(result.id, deployment.id);
    }

    #[tokio::test]
    async fn deploy_1() {
        // define the config instance
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };

        // create caches
        let dir = Dir::create_temp_dir("deploy").await.unwrap();
        let staging_dir = Dir::create_temp_dir("deploy_staging").await.unwrap();
        let (deployment_cache, _) = DeploymentCache::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_cache, _) = ConfigInstanceCache::spawn(16, dir.file("metadata.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content_cache, _) = ConfigInstanceContentCache::spawn(16, dir.clone(), 1000)
            .await
            .unwrap();

        // write config instance to cache
        cfg_inst_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        cfg_inst_content_cache
            .write(
                cfg_inst.id.clone(),
                json!({"speed": 4}),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // define the deployment referencing the config instance
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // apply the deployment
        let retry_policy = RetryPolicy::default();
        let ctx = DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };
        let result = apply(&deployment, &deployment_cache, &cfg_inst_cache, &ctx)
            .await
            .unwrap();

        // check that the deployment's activity status was updated to Deployed
        assert_eq!(result.activity_status, DeploymentActivityStatus::Deployed);
        assert_eq!(result.id, deployment.id);
    }

    #[tokio::test]
    async fn deploy_missing_content_causes_error_state() {
        // define the config instance (content will NOT be in cache)
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };

        // create caches
        let dir = Dir::create_temp_dir("deploy").await.unwrap();
        let staging_dir = Dir::create_temp_dir("deploy_staging").await.unwrap();
        let (deployment_cache, _) = DeploymentCache::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_cache, _) = ConfigInstanceCache::spawn(16, dir.file("metadata.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content_cache, _) = ConfigInstanceContentCache::spawn(16, dir.clone(), 1000)
            .await
            .unwrap();

        // write config instance metadata but NOT content
        cfg_inst_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // define the deployment referencing the config instance
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // apply the deployment - it should still return Ok but with error state
        // because the deploy itself fails (missing content) but the function catches it
        let retry_policy = RetryPolicy::default();
        let ctx = DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };
        let result = apply(&deployment, &deployment_cache, &cfg_inst_cache, &ctx)
            .await
            .unwrap();

        // The deployment should have error state due to missing content
        assert_eq!(result.id, deployment.id);
        assert_eq!(result.error_status, DeploymentErrorStatus::Retrying);
    }

    #[tokio::test]
    async fn remove_deployment() {
        // define the config instance
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };

        // create caches
        let dir = Dir::create_temp_dir("remove").await.unwrap();
        let staging_dir = Dir::create_temp_dir("remove_staging").await.unwrap();
        let (deployment_cache, _) = DeploymentCache::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_cache, _) = ConfigInstanceCache::spawn(16, dir.file("metadata.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content_cache, _) = ConfigInstanceContentCache::spawn(16, dir.clone(), 1000)
            .await
            .unwrap();

        // write config instance to cache
        cfg_inst_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        cfg_inst_content_cache
            .write(
                cfg_inst.id.clone(),
                json!({"speed": 4}),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // define a deployment targeting removal (currently deployed)
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Archived,
            activity_status: DeploymentActivityStatus::Deployed,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // apply the deployment
        let retry_policy = RetryPolicy::default();
        let ctx = DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };
        let result = apply(&deployment, &deployment_cache, &cfg_inst_cache, &ctx)
            .await
            .unwrap();

        // check that the deployment's activity status was updated to Archived (removed)
        assert_eq!(result.activity_status, DeploymentActivityStatus::Archived);
        assert_eq!(result.id, deployment.id);
    }

    #[tokio::test]
    async fn archive_deployment() {
        // define the config instance
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };

        // create caches
        let dir = Dir::create_temp_dir("archive").await.unwrap();
        let staging_dir = Dir::create_temp_dir("archive_staging").await.unwrap();
        let (deployment_cache, _) = DeploymentCache::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_cache, _) = ConfigInstanceCache::spawn(16, dir.file("metadata.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content_cache, _) = ConfigInstanceContentCache::spawn(16, dir.clone(), 1000)
            .await
            .unwrap();

        // write config instance to cache
        cfg_inst_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // define a deployment targeting archive (currently queued, target staged)
        // FSM: target=Staged, activity=Queued -> Archive
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Staged,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // apply the deployment
        let retry_policy = RetryPolicy::default();
        let ctx = DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir,
            staging_dir: &staging_dir,
            retry_policy: &retry_policy,
        };
        let result = apply(&deployment, &deployment_cache, &cfg_inst_cache, &ctx)
            .await
            .unwrap();

        // check that the deployment's activity status was updated to Archived
        assert_eq!(result.activity_status, DeploymentActivityStatus::Archived);
        assert_eq!(result.id, deployment.id);
    }
}
