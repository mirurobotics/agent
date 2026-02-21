// internal crates
use miru_agent::cache::file::FileCache;
use miru_agent::deploy::filesys::deploy;
use miru_agent::filesys::path::PathExt;
use miru_agent::filesys::{dir::Dir, Overwrite, WriteOptions};
use miru_agent::models::config_instance::ConfigInstance;
use miru_agent::models::deployment::{Deployment, DplActivity, DplTarget};

// external crates
use serde_json::json;

struct Fixture {
    ci_meta_cache: FileCache<String, ConfigInstance>,
    content_cache: FileCache<String, serde_json::Value>,
    staging_dir: Dir,
    deployment_dir: Dir,
    _temp_dir: Dir,
}

impl Fixture {
    async fn new() -> Self {
        let temp_dir = Dir::create_temp_dir("deploy-filesys-test").await.unwrap();
        let cache_dir = temp_dir.subdir("caches");

        let (ci_meta_cache, _) =
            FileCache::<String, ConfigInstance>::spawn(16, cache_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (content_cache, _) =
            FileCache::<String, serde_json::Value>::spawn(16, cache_dir.file("content.json"), 1000)
                .await
                .unwrap();

        let staging_dir = temp_dir.subdir("staging");
        staging_dir.create().await.unwrap();
        let deployment_dir = temp_dir.subdir("deployments");

        Self {
            ci_meta_cache,
            content_cache,
            staging_dir,
            deployment_dir,
            _temp_dir: temp_dir,
        }
    }

    async fn seed_config_instance(&self, cfg_inst: &ConfigInstance, content: &serde_json::Value) {
        self.ci_meta_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        self.content_cache
            .write(
                cfg_inst.id.clone(),
                content.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
    }

    fn make_deployment(&self, cfg_insts: &[ConfigInstance]) -> Deployment {
        Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: cfg_insts.iter().map(|c| c.id.clone()).collect(),
            ..Default::default()
        }
    }

    async fn deploy(
        &self,
        deployment: &Deployment,
    ) -> Result<(), miru_agent::deploy::errors::DeployErr> {
        deploy(
            &self.ci_meta_cache,
            &self.content_cache,
            &self.staging_dir,
            &self.deployment_dir,
            deployment,
        )
        .await
    }
}

pub mod deploy_func {
    use super::*;

    #[tokio::test]
    async fn creates_new_file() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        let content = json!({"speed": 4});
        f.seed_config_instance(&cfg_inst, &content).await;

        let deployment = f.make_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .deployment_dir
            .file(&cfg_inst.filepath)
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let f = Fixture::new().await;
        let filepath = "/test/filepath".to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let new_content = json!({"speed": 4});
        f.seed_config_instance(&cfg_inst, &new_content).await;

        // pre-populate the target with old content
        let file = f.deployment_dir.file(&filepath);
        file.write_json(&json!({"old": true}), WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let deployment = f.make_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .deployment_dir
            .file(&filepath)
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, new_content);
    }

    #[tokio::test]
    async fn multiple_config_instances() {
        let f = Fixture::new().await;
        let mut cfg_insts = Vec::new();
        let mut contents = Vec::new();
        for i in 0..3 {
            let cfg_inst = ConfigInstance {
                filepath: format!("/test/filepath{i}"),
                ..Default::default()
            };
            let content = json!({"index": i});
            f.seed_config_instance(&cfg_inst, &content).await;
            cfg_insts.push(cfg_inst);
            contents.push(content);
        }

        let deployment = f.make_deployment(&cfg_insts);
        f.deploy(&deployment).await.unwrap();

        for (i, cfg_inst) in cfg_insts.iter().enumerate() {
            let actual = f
                .deployment_dir
                .file(&cfg_inst.filepath)
                .read_json::<serde_json::Value>()
                .await
                .unwrap();
            assert_eq!(actual, contents[i]);
        }
    }

    #[tokio::test]
    async fn missing_config_instance_metadata_returns_error() {
        let f = Fixture::new().await;
        // deployment references a config instance ID that doesn't exist in the cache
        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["nonexistent-id".to_string()],
            ..Default::default()
        };
        let result = f.deploy(&deployment).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_content_returns_error() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        // seed metadata but not content
        f.ci_meta_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = f.make_deployment(&[cfg_inst]);
        let result = f.deploy(&deployment).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn staging_dir_is_cleaned_up_on_success() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        f.seed_config_instance(&cfg_inst, &json!({"ok": true}))
            .await;

        let deployment = f.make_deployment(&[cfg_inst]);
        f.deploy(&deployment).await.unwrap();

        // staging_dir itself should still exist but have no subdirectories
        // (the temp UUID dir was moved away, then delete() was called on the
        // already-absent path, which is a no-op)
        let subdirs = f.staging_dir.subdirs().await.unwrap();
        assert!(
            subdirs.is_empty(),
            "staging dir should have no leftover temp directories, found: {subdirs:?}",
        );
    }

    #[tokio::test]
    async fn staging_dir_is_cleaned_up_on_write_failure() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        // seed metadata but NOT content — write_file will fail when reading content
        f.ci_meta_cache
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = f.make_deployment(&[cfg_inst]);
        let result = f.deploy(&deployment).await;
        assert!(result.is_err());

        // the temp directory should have been cleaned up despite the error
        let subdirs = f.staging_dir.subdirs().await.unwrap();
        assert!(
            subdirs.is_empty(),
            "staging dir should be cleaned up after failure, found: {subdirs:?}",
        );
    }

    #[tokio::test]
    async fn target_dir_unchanged_on_write_failure() {
        let f = Fixture::new().await;

        // seed one valid config instance
        let good_cfg = ConfigInstance {
            filepath: "/test/good".to_string(),
            ..Default::default()
        };
        f.seed_config_instance(&good_cfg, &json!({"good": true}))
            .await;

        // pre-populate the target directory with existing content
        let existing_file = f.deployment_dir.file("/test/existing");
        let existing_content = json!({"existing": true});
        existing_file
            .write_json(&existing_content, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // create a deployment with a valid and an invalid config instance
        let bad_cfg = ConfigInstance {
            filepath: "/test/bad".to_string(),
            ..Default::default()
        };
        // seed metadata for bad_cfg but NOT content
        f.ci_meta_cache
            .write(
                bad_cfg.id.clone(),
                bad_cfg.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            config_instance_ids: vec![good_cfg.id.clone(), bad_cfg.id.clone()],
            ..Default::default()
        };
        let result = f.deploy(&deployment).await;
        assert!(result.is_err());

        // the existing target dir content should be untouched
        let actual = existing_file
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, existing_content);
    }

    #[tokio::test]
    async fn target_dir_created_if_absent() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        let content = json!({"new": true});
        f.seed_config_instance(&cfg_inst, &content).await;

        // deployment_dir does not exist yet
        assert!(!f.deployment_dir.exists());

        let deployment = f.make_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .deployment_dir
            .file(&cfg_inst.filepath)
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn deploy_is_idempotent() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        let content = json!({"speed": 4});
        f.seed_config_instance(&cfg_inst, &content).await;

        let deployment = f.make_deployment(std::slice::from_ref(&cfg_inst));

        // deploy twice
        f.deploy(&deployment).await.unwrap();
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .deployment_dir
            .file(&cfg_inst.filepath)
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn deploy_removes_stale_files_from_target() {
        let f = Fixture::new().await;

        // first deploy writes file_a
        let cfg_a = ConfigInstance {
            filepath: "/test/file_a".to_string(),
            ..Default::default()
        };
        f.seed_config_instance(&cfg_a, &json!({"a": true})).await;
        let deployment_a = f.make_deployment(std::slice::from_ref(&cfg_a));
        f.deploy(&deployment_a).await.unwrap();
        assert!(f.deployment_dir.file("/test/file_a").exists());

        // second deploy writes only file_b (different config instance set)
        let cfg_b = ConfigInstance {
            filepath: "/test/file_b".to_string(),
            ..Default::default()
        };
        f.seed_config_instance(&cfg_b, &json!({"b": true})).await;
        let deployment_b = f.make_deployment(std::slice::from_ref(&cfg_b));
        f.deploy(&deployment_b).await.unwrap();

        // file_a should be gone since the staging dir replaced the entire target
        assert!(
            !f.deployment_dir.file("/test/file_a").exists(),
            "stale files from previous deploy should be removed",
        );
        assert!(f.deployment_dir.file("/test/file_b").exists());
    }

    #[tokio::test]
    async fn nested_filepath() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/deeply/nested/path/config.json".to_string(),
            ..Default::default()
        };
        let content = json!({"nested": true});
        f.seed_config_instance(&cfg_inst, &content).await;

        let deployment = f.make_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .deployment_dir
            .file(&cfg_inst.filepath)
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, content);
    }
}
