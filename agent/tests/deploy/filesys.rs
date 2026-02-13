// internal crates
use crate::deploy::observer::HistoryObserver;
use miru_agent::cache::file::FileCache;
use miru_agent::deploy::filesys::{deploy, DeployContext};
use miru_agent::deploy::fsm::Settings;
use miru_agent::deploy::observer::Observer;
use miru_agent::filesys::dir::Dir;
use miru_agent::models::config_instance::ConfigInstance;
use miru_agent::models::deployment::{
    Deployment, DeploymentActivityStatus, DeploymentErrorStatus, DeploymentTargetStatus,
};
use miru_agent::utils::calc_exp_backoff;

// external crates
use chrono::{TimeDelta, Utc};
use serde_json::json;

pub mod deploy {
    use super::*;

    #[tokio::test]
    async fn deploy_failed_missing_instance_data() {
        // define the config instance and deployment
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        let deployment = Deployment {
            // target status must be deployed to increment failure attempts
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // create the cache but omit the config instance content
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();

        // build the deploy context
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("deployments");
        let staging_dir = temp_dir.subdir("staging");
        staging_dir.create(true).await.unwrap();
        let ctx = DeployContext {
            content_reader: &cache,
            deployment_dir: &deployment_dir,
            staging_dir: &staging_dir,
            settings: &settings,
        };

        // deploy
        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let (result_deployment, result) = deploy(
            &ctx,
            deployment.clone(),
            vec![cfg_inst.clone()],
            &mut observers,
        )
        .await;
        assert!(result.is_err());

        // the deployment should have transitioned to error state
        assert_eq!(
            result_deployment.error_status,
            DeploymentErrorStatus::Retrying
        );
        assert_eq!(result_deployment.attempts, 1);

        // verify cooldown was set
        let cooldown = calc_exp_backoff(
            settings.exp_backoff_base_secs,
            2,
            result_deployment.attempts,
            settings.max_cooldown_secs,
        );
        let approx_cooldown_ends_at = Utc::now() + TimeDelta::seconds(cooldown);
        let actual_cooldown = result_deployment.cooldown_ends_at.unwrap();
        assert!(actual_cooldown <= approx_cooldown_ends_at);
        assert!(actual_cooldown >= approx_cooldown_ends_at - TimeDelta::seconds(1));

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], result_deployment);
    }

    #[tokio::test]
    async fn deploy_success_filepath_specified_overwrite_existing() {
        // define the config instance and deployment
        let filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // create the config instance in the cache
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();
        let cfg_inst_content = json!({"speed": 4});
        cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst_content.clone(),
                |_, _| false,
                true,
            )
            .await
            .unwrap();

        // create the file in the deployment directory (simulating pre-existing file)
        let deployment_dir = temp_dir.subdir("deployments");
        let file = deployment_dir.file(filepath.as_str());
        file.write_json(&cfg_inst_content, true, true)
            .await
            .unwrap();

        // build the deploy context
        let settings = Settings::default();
        let staging_dir = temp_dir.subdir("staging");
        staging_dir.create(true).await.unwrap();
        let ctx = DeployContext {
            content_reader: &cache,
            deployment_dir: &deployment_dir,
            staging_dir: &staging_dir,
            settings: &settings,
        };

        // deploy
        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let (result_deployment, result) = deploy(
            &ctx,
            deployment.clone(),
            vec![cfg_inst.clone()],
            &mut observers,
        )
        .await;
        result.unwrap();

        // the deployment should have transitioned to deployed
        assert_eq!(
            result_deployment.activity_status,
            DeploymentActivityStatus::Deployed
        );
        assert_eq!(result_deployment.error_status, DeploymentErrorStatus::None);

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], result_deployment);

        // check that the file was written
        let file = deployment_dir.file(filepath.as_str());
        let actual = file.read_json::<serde_json::Value>().await.unwrap();
        assert_eq!(actual, cfg_inst_content);
    }

    #[tokio::test]
    async fn deploy_success_filepath_specified_no_existing() {
        // define the config instance and deployment
        let filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // create the config instance in the cache
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();
        let cfg_inst_content = json!({"speed": 4});
        cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst_content.clone(),
                |_, _| false,
                true,
            )
            .await
            .unwrap();

        // build the deploy context
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("deployments");
        let staging_dir = temp_dir.subdir("staging");
        staging_dir.create(true).await.unwrap();
        let ctx = DeployContext {
            content_reader: &cache,
            deployment_dir: &deployment_dir,
            staging_dir: &staging_dir,
            settings: &settings,
        };

        // deploy
        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let (result_deployment, result) = deploy(
            &ctx,
            deployment.clone(),
            vec![cfg_inst.clone()],
            &mut observers,
        )
        .await;
        result.unwrap();

        // the deployment should have transitioned to deployed
        assert_eq!(
            result_deployment.activity_status,
            DeploymentActivityStatus::Deployed
        );
        assert_eq!(result_deployment.error_status, DeploymentErrorStatus::None);

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], result_deployment);

        // check that the file was created
        let file = deployment_dir.file(filepath.as_str());
        let actual = file.read_json::<serde_json::Value>().await.unwrap();
        assert_eq!(actual, cfg_inst_content);
    }

    #[tokio::test]
    async fn deploy_multiple_config_instances() {
        // define config instances and deployment
        let n = 3;
        let mut cfg_insts = Vec::new();
        for i in 0..n {
            cfg_insts.push(ConfigInstance {
                filepath: format!("/test/filepath{i}"),
                ..Default::default()
            });
        }
        let deployment = Deployment {
            target_status: DeploymentTargetStatus::Deployed,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: cfg_insts.iter().map(|c| c.id.clone()).collect(),
            ..Default::default()
        };

        // create the config instances in the cache
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();
        let mut contents = Vec::new();
        for cfg_inst in &cfg_insts {
            let content = json!({"filepath": cfg_inst.filepath.clone()});
            cache
                .write(cfg_inst.id.clone(), content.clone(), |_, _| false, true)
                .await
                .unwrap();
            contents.push(content);
        }

        // build the deploy context
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("deployments");
        let staging_dir = temp_dir.subdir("staging");
        staging_dir.create(true).await.unwrap();
        let ctx = DeployContext {
            content_reader: &cache,
            deployment_dir: &deployment_dir,
            staging_dir: &staging_dir,
            settings: &settings,
        };

        // deploy
        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let (result_deployment, result) =
            deploy(&ctx, deployment.clone(), cfg_insts.clone(), &mut observers).await;
        result.unwrap();

        // the deployment should have transitioned to deployed
        assert_eq!(
            result_deployment.activity_status,
            DeploymentActivityStatus::Deployed
        );

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], result_deployment);

        // check that all files were created
        for (i, cfg_inst) in cfg_insts.iter().enumerate() {
            let file = deployment_dir.file(&cfg_inst.filepath);
            let actual = file.read_json::<serde_json::Value>().await.unwrap();
            assert_eq!(actual, contents[i]);
        }
    }

    #[tokio::test]
    async fn deploy_failed_does_not_increment_attempts_when_target_not_deployed() {
        // define the config instance and deployment
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        let deployment = Deployment {
            // target status is staged, so attempts should NOT be incremented
            target_status: DeploymentTargetStatus::Staged,
            activity_status: DeploymentActivityStatus::Queued,
            config_instance_ids: vec![cfg_inst.id.clone()],
            ..Default::default()
        };

        // create the cache but omit the config instance content
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();

        // build the deploy context
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("deployments");
        let staging_dir = temp_dir.subdir("staging");
        staging_dir.create(true).await.unwrap();
        let ctx = DeployContext {
            content_reader: &cache,
            deployment_dir: &deployment_dir,
            staging_dir: &staging_dir,
            settings: &settings,
        };

        // deploy
        let mut observer = HistoryObserver::new();
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];
        let (result_deployment, result) = deploy(
            &ctx,
            deployment.clone(),
            vec![cfg_inst.clone()],
            &mut observers,
        )
        .await;
        assert!(result.is_err());

        // attempts should not be incremented since target_status != Deployed
        assert_eq!(result_deployment.attempts, 0);
        assert_eq!(
            result_deployment.error_status,
            DeploymentErrorStatus::Retrying
        );
    }
}
