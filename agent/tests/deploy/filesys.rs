// internal crates
use crate::deploy::observer::HistoryObserver;
use miru_agent::cache::file::FileCache;
use miru_agent::deploy::{filesys::deploy, fsm::Settings, observer::Observer};
use miru_agent::filesys::dir::Dir;
use miru_agent::filesys::path::PathExt;
use miru_agent::models::config_instance::{
    ActivityStatus, ConfigInstance, ErrorStatus, TargetStatus,
};
use miru_agent::utils::calc_exp_backoff;

// external crates
use chrono::{TimeDelta, Utc};
use serde_json::json;

pub mod deploy {
    use super::*;

    #[tokio::test]
    async fn deploy_1_failed_missing_instance_data() {
        // define the config instance
        let deploy_filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            relative_filepath: deploy_filepath.clone(),
            // target status must be deployed to increment failure attempts
            target_status: TargetStatus::Deployed,
            ..Default::default()
        };

        // create the cache but omit the config instance content
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();

        // deploy the config instance
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("config_instances");
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            vec![],
            vec![cfg_inst.clone()],
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected config instance
        let expected = ConfigInstance {
            activity_status: ActivityStatus::Removed,
            error_status: ErrorStatus::Retrying,
            attempts: 1,
            cooldown_ends_at: deploy_results.to_deploy[0].cooldown_ends_at,
            ..cfg_inst
        };
        let cooldown = calc_exp_backoff(
            settings.exp_backoff_base_secs,
            2,
            expected.attempts,
            settings.max_cooldown_secs,
        );
        let approx_cooldown_ends_at = Utc::now() + TimeDelta::seconds(cooldown);
        assert!(expected.cooldown_ends_at <= approx_cooldown_ends_at);
        assert!(expected.cooldown_ends_at >= approx_cooldown_ends_at - TimeDelta::seconds(1));

        // check that the returned instances' states were correctly updated
        assert!(deploy_results.to_remove.is_empty());
        assert_eq!(deploy_results.to_deploy.len(), 1);
        assert_eq!(deploy_results.to_deploy[0], expected);

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 2);
        assert_eq!(observer.history[1], expected);
    }

    // deploy 1 - filepath specified overwrites existing file
    #[tokio::test]
    async fn deploy_1_filepath_specified_overwrite_existing() {
        // define the config instance
        let filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            relative_filepath: filepath.clone(),
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

        // create the file in the deployment directory
        let deployment_dir = temp_dir.subdir("config_instances");
        let file = deployment_dir.file(filepath.as_str());
        file.write_json(&cfg_inst_content, true, true)
            .await
            .unwrap();

        // deploy the config instance
        let settings = Settings::default();
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            vec![],
            vec![cfg_inst.clone()],
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected config instance
        let expected = ConfigInstance {
            activity_status: ActivityStatus::Deployed,
            ..cfg_inst
        };

        // check that the returned instances' states were correctly updated
        assert!(deploy_results.to_remove.is_empty());
        assert_eq!(deploy_results.to_deploy.len(), 1);
        assert_eq!(deploy_results.to_deploy[0], expected);

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], expected);

        // check that the file was created
        let file = deployment_dir.file(filepath.as_str());
        let actual = file.read_json::<serde_json::Value>().await.unwrap();
        assert_eq!(actual, cfg_inst_content);
    }

    // deploy 1 - filepath specified doesn't overwrite existing file
    #[tokio::test]
    async fn deploy_1_filepath_specified_no_overwrite() {
        // define the config instance
        let filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            relative_filepath: filepath.clone(),
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

        // deploy the config instance
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("config_instances");
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            vec![],
            vec![cfg_inst.clone()],
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected config instance
        let expected = ConfigInstance {
            activity_status: ActivityStatus::Deployed,
            ..cfg_inst
        };

        // check that the returned instances' states were correctly updated
        assert!(deploy_results.to_remove.is_empty());
        assert_eq!(deploy_results.to_deploy.len(), 1);
        assert_eq!(deploy_results.to_deploy[0], expected);

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], expected);

        // check that the file was created
        let file = deployment_dir.file(filepath.as_str());
        let actual = file.read_json::<serde_json::Value>().await.unwrap();
        assert_eq!(actual, cfg_inst_content);
    }

    // remove failures are essentially impossible since removing a file that doesn't exist
    // does not throw an error

    #[tokio::test]
    async fn remove_1_filepath_specified_doesnt_exist() {
        // define the config instance
        let filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            relative_filepath: filepath.clone(),
            ..Default::default()
        };

        // create the config instance in the cache
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();
        cache
            .write(cfg_inst.id.clone(), json!({"speed": 4}), |_, _| false, true)
            .await
            .unwrap();

        // deploy the config instance
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("config_instances");
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            vec![cfg_inst.clone()],
            vec![],
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected config instance
        let expected = ConfigInstance {
            activity_status: ActivityStatus::Removed,
            ..cfg_inst
        };

        // check that the returned instances' states were correctly updated
        assert_eq!(deploy_results.to_remove.len(), 1);
        assert_eq!(deploy_results.to_remove[0], expected);
        assert!(deploy_results.to_deploy.is_empty());

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], expected);
    }

    #[tokio::test]
    async fn remove_1_filepath_specified_exists() {
        // define the config instance
        let filepath = "/test/filepath/config.json".to_string();
        let cfg_inst = ConfigInstance {
            relative_filepath: filepath.clone(),
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

        // create the file in the deployment directory
        let deployment_dir = temp_dir.subdir("config_instances");
        let file = deployment_dir.file(filepath.as_str());
        file.write_json(&cfg_inst_content, true, true)
            .await
            .unwrap();

        // deploy the config instance
        let settings = Settings::default();
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            vec![cfg_inst.clone()],
            vec![],
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected config instance
        let expected = ConfigInstance {
            activity_status: ActivityStatus::Removed,
            ..cfg_inst
        };

        // check that the returned instances' states were correctly updated
        assert_eq!(deploy_results.to_remove.len(), 1);
        assert_eq!(deploy_results.to_remove[0], expected);
        assert!(deploy_results.to_deploy.is_empty());

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 1);
        assert_eq!(observer.history[0], expected);

        // check that the file was removed
        assert!(!file.exists());

        // check that the parent directories were also removed
        assert!(deployment_dir.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn rollback_1_deploy_missing_instance_data() {
        // define the config instance
        let to_deploy_filepath = "/to/deploy/filepath".to_string();
        let to_deploy = ConfigInstance {
            relative_filepath: to_deploy_filepath.clone(),
            // target status must be deployed to increment failure attempts
            target_status: TargetStatus::Deployed,
            ..Default::default()
        };
        let to_remove_filepath = "/to/remove/filepath".to_string();
        let to_remove = ConfigInstance {
            relative_filepath: to_remove_filepath.clone(),
            target_status: TargetStatus::Removed,
            ..Default::default()
        };

        // create the cache but the config instance content for the to_deploy config instance
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();
        let to_remove_data = json!({"speed": 8});
        cache
            .write(
                to_remove.id.clone(),
                to_remove_data.clone(),
                |_, _| false,
                true,
            )
            .await
            .unwrap();

        // deploy the config instance
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("config_instances");
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            vec![to_remove.clone()],
            vec![to_deploy.clone()],
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected instances
        let expected_to_remove = ConfigInstance {
            activity_status: ActivityStatus::Deployed,
            ..to_remove
        };

        let expected_to_deploy = ConfigInstance {
            activity_status: ActivityStatus::Removed,
            error_status: ErrorStatus::Retrying,
            attempts: 1,
            cooldown_ends_at: deploy_results.to_deploy[0].cooldown_ends_at,
            ..to_deploy
        };
        let cooldown = calc_exp_backoff(
            settings.exp_backoff_base_secs,
            2,
            expected_to_deploy.attempts,
            settings.max_cooldown_secs,
        );
        let approx_cooldown_ends_at = Utc::now() + TimeDelta::seconds(cooldown);
        assert!(expected_to_deploy.cooldown_ends_at <= approx_cooldown_ends_at);
        assert!(
            expected_to_deploy.cooldown_ends_at >= approx_cooldown_ends_at - TimeDelta::seconds(1)
        );

        // check that the returned instances' states were correctly updated
        assert_eq!(deploy_results.to_remove.len(), 1);
        assert_eq!(deploy_results.to_remove[0], expected_to_remove);
        assert_eq!(deploy_results.to_deploy.len(), 1);
        assert_eq!(deploy_results.to_deploy[0], expected_to_deploy);

        // check that the observer's history was correctly updated
        assert_eq!(observer.history.len(), 4);
        assert_eq!(observer.history[2], expected_to_deploy);
        assert_eq!(observer.history[3], expected_to_remove);

        // check that the removed config instance is still deployed
        let file = deployment_dir.file(to_remove_filepath.as_str());
        let actual = file.read_json::<serde_json::Value>().await.unwrap();
        assert_eq!(actual, to_remove_data);
    }

    #[tokio::test]
    async fn rollback_n_deploy_missing_instance_data() {
        // define the instances
        let n = 10;
        let mut to_deploy_instances = Vec::new();
        for i in 0..n {
            let filepath = format!("/to/deploy/filepath{i}");
            let cfg_inst = ConfigInstance {
                relative_filepath: filepath.clone(),
                target_status: TargetStatus::Deployed,
                ..Default::default()
            };
            to_deploy_instances.push(cfg_inst);
        }
        let mut to_remove_instances = Vec::new();
        for i in 0..n {
            let filepath = format!("/to/remove/filepath{i}");
            let cfg_inst = ConfigInstance {
                relative_filepath: filepath.clone(),
                target_status: TargetStatus::Removed,
                ..Default::default()
            };
            to_remove_instances.push(cfg_inst);
        }

        // create the cache but the config instance content for the to_deploy config instance
        let temp_dir = Dir::create_temp_dir("deploy").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");
        let (cache, _) = FileCache::spawn(16, cache_dir.file("cache.json"), 1000)
            .await
            .unwrap();
        for cfg_inst in to_remove_instances.iter() {
            cache
                .write(
                    cfg_inst.id.clone(),
                    json!({"relative_filepath": cfg_inst.relative_filepath.clone()}),
                    |_, _| false,
                    true,
                )
                .await
                .unwrap();
        }

        // deploy the config instance
        let settings = Settings::default();
        let deployment_dir = temp_dir.subdir("config_instances");
        let mut observers: Vec<&mut dyn Observer> = Vec::new();
        let mut observer = HistoryObserver::new();
        observers.push(&mut observer);
        let (deploy_results, result) = deploy(
            to_remove_instances.clone(),
            to_deploy_instances.clone(),
            &cache,
            &deployment_dir,
            &settings,
            &mut observers,
        )
        .await;
        result.unwrap();

        // define the expected instances
        let mut expected_to_remove_instances = to_remove_instances.clone();
        for cfg_inst in expected_to_remove_instances.iter_mut() {
            cfg_inst.activity_status = ActivityStatus::Deployed;
        }
        let mut expected_to_deploy_instances = to_deploy_instances.clone();
        for (i, cfg_inst) in expected_to_deploy_instances.iter_mut().enumerate() {
            cfg_inst.activity_status = ActivityStatus::Removed;
            if i == 0 {
                cfg_inst.error_status = ErrorStatus::Retrying;
                cfg_inst.attempts = 1;
                let cooldown = calc_exp_backoff(
                    settings.exp_backoff_base_secs,
                    2,
                    cfg_inst.attempts,
                    settings.max_cooldown_secs,
                );
                let approx_cooldown_ends_at = Utc::now() + TimeDelta::seconds(cooldown);
                cfg_inst.cooldown_ends_at = deploy_results.to_deploy[i].cooldown_ends_at;
                assert!(cfg_inst.cooldown_ends_at <= approx_cooldown_ends_at);
                assert!(
                    cfg_inst.cooldown_ends_at >= approx_cooldown_ends_at - TimeDelta::seconds(1)
                );
            }
        }

        // check that the returned instances' states were correctly updated
        assert_eq!(deploy_results.to_remove.len(), n);
        assert_eq!(
            deploy_results.to_remove.len(),
            expected_to_remove_instances.len()
        );
        for (i, cfg_inst) in deploy_results.to_remove.iter().enumerate() {
            assert_eq!(cfg_inst, &expected_to_remove_instances[i]);
        }
        assert_eq!(deploy_results.to_deploy.len(), n);
        assert_eq!(
            deploy_results.to_deploy.len(),
            expected_to_deploy_instances.len()
        );
        for (i, cfg_inst) in deploy_results.to_deploy.iter().enumerate() {
            assert_eq!(cfg_inst, &expected_to_deploy_instances[i]);
        }

        // check that the observer's history was correctly updated
        for (i, cfg_inst) in expected_to_deploy_instances.iter().enumerate() {
            assert_eq!(&observer.history[n + 1 + i], cfg_inst);
        }
        for (i, cfg_inst) in expected_to_remove_instances.iter().enumerate() {
            assert_eq!(&observer.history[2 * n + 1 + i], cfg_inst);
        }

        // check that the removed instances are still deployed
        for cfg_inst in to_remove_instances {
            let file = deployment_dir.file(&cfg_inst.relative_filepath);
            let actual = file.read_json::<serde_json::Value>().await.unwrap();
            assert_eq!(
                actual,
                json!({"relative_filepath": cfg_inst.relative_filepath})
            );
        }
    }
}
