// Deployment-centric sync tests
// These replace the old config_instances sync tests.

use miru_agent::crud::prelude::*;
use miru_agent::deploy::{filesys, fsm};
use miru_agent::filesys::dir::Dir;
use miru_agent::http::errors::*;
use miru_agent::models::deployment::{DeploymentActivityStatus, DeploymentTargetStatus};
use miru_agent::storage::config_instances::{ConfigInstanceCache, ConfigInstanceContentCache};
use miru_agent::storage::deployments::DeploymentCache;
use miru_agent::sync::deployments::sync;

use crate::http::mock::MockClient;

use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus as BackendActivityStatus,
    DeploymentErrorStatus as BackendErrorStatus, DeploymentStatus as BackendStatus,
    DeploymentTargetStatus as BackendTargetStatus,
};

// external crates
use serde_json::json;

fn make_backend_deployment(id: &str) -> BackendDeployment {
    BackendDeployment {
        object: openapi_client::models::deployment::Object::Deployment,
        id: id.to_string(),
        description: "test".to_string(),
        status: BackendStatus::DEPLOYMENT_STATUS_QUEUED,
        activity_status: BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        error_status: BackendErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        target_status: BackendTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        device_id: "dev_1".to_string(),
        release_id: "rls_1".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        release: None,
        config_instances: Some(vec![openapi_client::models::ConfigInstance {
            object: openapi_client::models::config_instance::Object::ConfigInstance,
            id: format!("{id}_ci_1"),
            config_type_name: "test_type".to_string(),
            filepath: "test/config.json".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            config_schema_id: "schema_1".to_string(),
            config_type_id: "ct_1".to_string(),
            config_type: None,
            content: Some(json!({"key": "value"})),
        }]),
    }
}

pub mod sync_tests {
    use super::*;

    #[tokio::test]
    async fn pull_and_push_empty() {
        let dir = Dir::create_temp_dir("sync_empty").await.unwrap();

        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content"), 1000)
                .await
                .unwrap();

        let http_client = MockClient::default();
        let dpl_retry_policy = fsm::RetryPolicy::default();
        let ctx = filesys::DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir.subdir("deployments"),
            staging_dir: &dir.subdir("staging"),
            retry_policy: &dpl_retry_policy,
        };

        let result = sync(
            &deployment_cache,
            &cfg_inst_cache,
            &cfg_inst_content_cache,
            &http_client,
            &ctx,
            "test_token",
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn pull_stores_deployments_and_config_instances() {
        let dir = Dir::create_temp_dir("sync_pull").await.unwrap();

        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content"), 1000)
                .await
                .unwrap();

        let http_client = MockClient::default();
        let backend_dep = make_backend_deployment("dpl_1");
        http_client.set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        let dpl_retry_policy = fsm::RetryPolicy::default();
        let ctx = filesys::DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir.subdir("deployments"),
            staging_dir: &dir.subdir("staging"),
            retry_policy: &dpl_retry_policy,
        };

        let _result = sync(
            &deployment_cache,
            &cfg_inst_cache,
            &cfg_inst_content_cache,
            &http_client,
            &ctx,
            "test_token",
        )
        .await;

        // Check that the deployment was cached (apply runs, so status changes)
        let cached = deployment_cache
            .read_optional("dpl_1".to_string())
            .await
            .unwrap();
        assert!(cached.is_some(), "deployment should be cached");
        let cached = cached.unwrap();
        assert_eq!(cached.id, "dpl_1");
        // After sync (pull + apply + push), the deployment should be deployed
        // because FSM: target=Deployed, activity=Queued → Deploy → Deployed
        assert_eq!(cached.activity_status, DeploymentActivityStatus::Deployed);
        assert_eq!(cached.target_status, DeploymentTargetStatus::Deployed);
        assert_eq!(cached.config_instance_ids, vec!["dpl_1_ci_1"]);

        // Check that config instance was cached
        let ci = cfg_inst_cache
            .read_optional("dpl_1_ci_1".to_string())
            .await
            .unwrap();
        assert!(ci.is_some(), "config instance should be cached");
        let ci = ci.unwrap();
        assert_eq!(ci.filepath, "test/config.json");
    }

    #[tokio::test]
    async fn pull_failure_returns_error() {
        let dir = Dir::create_temp_dir("sync_fail").await.unwrap();

        let (deployment_cache, _) =
            DeploymentCache::spawn(16, dir.file("deployment_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_cache, _) =
            ConfigInstanceCache::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content_cache, _) =
            ConfigInstanceContentCache::spawn(16, dir.subdir("cfg_inst_content"), 1000)
                .await
                .unwrap();

        let http_client = MockClient::default();
        http_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_connection_error: true,
            }))
        });

        let dpl_retry_policy = fsm::RetryPolicy::default();
        let ctx = filesys::DeployContext {
            content_reader: &cfg_inst_content_cache,
            deployment_dir: &dir.subdir("deployments"),
            staging_dir: &dir.subdir("staging"),
            retry_policy: &dpl_retry_policy,
        };

        let result = sync(
            &deployment_cache,
            &cfg_inst_cache,
            &cfg_inst_content_cache,
            &http_client,
            &ctx,
            "test_token",
        )
        .await;

        assert!(result.is_err());
    }
}
