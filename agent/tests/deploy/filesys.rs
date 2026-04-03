// standard crates
use std::collections::HashSet;

// internal crates
use miru_agent::deploy::filesys::{deploy, filepaths, remove};
use miru_agent::deploy::DeployErr;
use miru_agent::filesys::{self, Overwrite, WriteOptions};
use miru_agent::models::{ConfigInstance, Deployment, DplActivity, DplTarget};
use miru_agent::storage;

// external crates
use serde_json::json;

struct Fixture {
    cfg_inst_meta: storage::CfgInsts,
    cfg_inst_content: storage::CfgInstContent,
    filesys_root: filesys::Dir,
    _temp_dir: filesys::Dir,
}

impl Fixture {
    async fn new() -> Self {
        let temp_dir = filesys::Dir::create_temp_dir("deploy-filesys-test")
            .await
            .unwrap();
        let resources_dir = temp_dir.subdir("resources");

        let (cfg_inst_meta, _) =
            storage::CfgInsts::spawn(16, resources_dir.file("ci_meta.json"), 1000)
                .await
                .unwrap();
        let (cfg_inst_content, _) =
            storage::CfgInstContent::spawn(16, resources_dir.subdir("content"), 1000)
                .await
                .unwrap();

        // Use the temp dir as the filesystem root so absolute paths like
        // "/test/filepath.json" resolve to "<temp_dir>/test/filepath.json".
        let filesys_root = temp_dir.clone();

        Self {
            cfg_inst_meta,
            cfg_inst_content,
            filesys_root,
            _temp_dir: temp_dir,
        }
    }

    async fn seed_cfg_inst(&self, cfg_inst: &ConfigInstance, content: String) {
        self.cfg_inst_meta
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        self.cfg_inst_content
            .write(
                cfg_inst.id.clone(),
                content.clone(),
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

    async fn deploy(&self, deployment: &Deployment) -> Result<(), DeployErr> {
        let stor = storage::CfgInstRef {
            meta: &self.cfg_inst_meta,
            content: &self.cfg_inst_content,
        };
        deploy(&stor, &self.filesys_root, deployment).await
    }

    async fn remove(&self, deployment: &Deployment, keep: &HashSet<String>) {
        let stor = storage::CfgInstRef {
            meta: &self.cfg_inst_meta,
            content: &self.cfg_inst_content,
        };
        remove(&stor, &self.filesys_root, deployment, keep).await
    }
}

pub mod deploy_func {
    use super::*;
    use miru_agent::filesys::PathExt;

    #[tokio::test]
    async fn creates_new_file() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath.json".to_string(),
            ..Default::default()
        };
        let content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .filesys_root
            .file(&cfg_inst.filepath)
            .read_string()
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
        let new_content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, new_content.clone()).await;

        // pre-populate the target with old content
        let file = f.filesys_root.file(&filepath);
        file.write_json(&json!({"old": true}), WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f.filesys_root.file(&filepath).read_string().await.unwrap();
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
            let content = format!("{{\"index\": {i}}}");
            f.seed_cfg_inst(&cfg_inst, content.clone()).await;
            cfg_insts.push(cfg_inst);
            contents.push(content);
        }

        let deployment = f.new_deployment(&cfg_insts);
        f.deploy(&deployment).await.unwrap();

        for (i, cfg_inst) in cfg_insts.iter().enumerate() {
            let actual = f
                .filesys_root
                .file(&cfg_inst.filepath)
                .read_string()
                .await
                .unwrap();
            assert_eq!(actual, contents[i]);
        }
    }

    #[tokio::test]
    async fn empty_config_instance_ids_returns_error() {
        let f = Fixture::new().await;
        let deployment = f.new_deployment(&[]);

        let result = f.deploy(&deployment).await;
        assert!(matches!(result, Err(DeployErr::EmptyConfigInstances(_))));
    }

    #[tokio::test]
    async fn wrong_target_status_returns_error() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"ok\": true}".to_string())
            .await;

        let mut deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        deployment.target_status = DplTarget::Staged;

        let result = f.deploy(&deployment).await;
        match result {
            Err(DeployErr::InvalidDeploymentTarget(err)) => {
                assert_eq!(err.deployment_id, deployment.id);
                assert_eq!(err.target_status, DplTarget::Staged);
            }
            _ => panic!("expected InvalidDeploymentTarget error"),
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
        f.cfg_inst_meta
            .write(
                cfg_inst.id.clone(),
                cfg_inst.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let deployment = f.new_deployment(&[cfg_inst]);
        let result = f.deploy(&deployment).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn partial_write_on_failure() {
        let f = Fixture::new().await;

        // seed one valid config instance
        let good_cfg = ConfigInstance {
            filepath: "/test/good".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&good_cfg, "{\"good\": true}".to_string())
            .await;

        // create a deployment with a valid and an invalid config instance
        let bad_cfg = ConfigInstance {
            filepath: "/test/bad".to_string(),
            ..Default::default()
        };
        // seed metadata for bad_cfg but NOT content
        f.cfg_inst_meta
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

        // with per-file writes, the successfully-written file exists on disk
        assert!(
            f.filesys_root.file("/test/good").exists(),
            "successfully written files should exist after partial failure",
        );
    }

    #[tokio::test]
    async fn parent_dirs_created_if_absent() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/filepath".to_string(),
            ..Default::default()
        };
        let content = "{\"new\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .filesys_root
            .file(&cfg_inst.filepath)
            .read_string()
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
        let content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));

        // deploy twice
        f.deploy(&deployment).await.unwrap();
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .filesys_root
            .file(&cfg_inst.filepath)
            .read_string()
            .await
            .unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn nested_filepath() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/deeply/nested/path/config.json".to_string(),
            ..Default::default()
        };
        let content = "{\"nested\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = f
            .filesys_root
            .file(&cfg_inst.filepath)
            .read_string()
            .await
            .unwrap();
        assert_eq!(actual, content);
    }
}

pub mod remove_func {
    use super::*;
    use miru_agent::filesys::PathExt;

    #[tokio::test]
    async fn deletes_config_instance_files() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/remove-me.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"data\": true}".to_string())
            .await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();
        assert!(f.filesys_root.file("/test/remove-me.json").exists());

        f.remove(&deployment, &HashSet::new()).await;
        assert!(
            !f.filesys_root.file("/test/remove-me.json").exists(),
            "file should be deleted after remove"
        );
    }

    #[tokio::test]
    async fn respects_keep_set() {
        let f = Fixture::new().await;
        let cfg_a = ConfigInstance {
            filepath: "/test/keep-me.json".to_string(),
            ..Default::default()
        };
        let cfg_b = ConfigInstance {
            filepath: "/test/delete-me.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_a, "{\"a\": true}".to_string()).await;
        f.seed_cfg_inst(&cfg_b, "{\"b\": true}".to_string()).await;

        let deployment = f.new_deployment(&[cfg_a.clone(), cfg_b.clone()]);
        f.deploy(&deployment).await.unwrap();

        let keep: HashSet<String> = ["/test/keep-me.json".to_string()].into();
        f.remove(&deployment, &keep).await;

        assert!(
            f.filesys_root.file("/test/keep-me.json").exists(),
            "file in keep set should be preserved"
        );
        assert!(
            !f.filesys_root.file("/test/delete-me.json").exists(),
            "file not in keep set should be deleted"
        );
    }

    #[tokio::test]
    async fn best_effort_on_missing_files() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/test/nonexistent.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "content".to_string()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        // Don't deploy first -- file doesn't exist on disk
        // remove should not panic or error
        f.remove(&deployment, &HashSet::new()).await;
    }

    #[tokio::test]
    async fn best_effort_on_missing_metadata() {
        let f = Fixture::new().await;
        // deployment references a config instance ID that doesn't exist in the cache
        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Deployed,
            config_instance_ids: vec!["evicted-id".to_string()],
            ..Default::default()
        };
        // remove should not panic -- it logs a warning and returns
        f.remove(&deployment, &HashSet::new()).await;
    }

    #[tokio::test]
    async fn removes_multiple_files() {
        let f = Fixture::new().await;
        let cfg_a = ConfigInstance {
            filepath: "/test/a.json".to_string(),
            ..Default::default()
        };
        let cfg_b = ConfigInstance {
            filepath: "/test/b.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_a, "a".to_string()).await;
        f.seed_cfg_inst(&cfg_b, "b".to_string()).await;

        let deployment = f.new_deployment(&[cfg_a, cfg_b]);
        f.deploy(&deployment).await.unwrap();

        f.remove(&deployment, &HashSet::new()).await;

        assert!(!f.filesys_root.file("/test/a.json").exists());
        assert!(!f.filesys_root.file("/test/b.json").exists());
    }
}

pub mod filepaths_func {
    use super::*;

    #[tokio::test]
    async fn collects_filepaths() {
        let f = Fixture::new().await;
        let cfg_a = ConfigInstance {
            filepath: "/srv/a.json".to_string(),
            ..Default::default()
        };
        let cfg_b = ConfigInstance {
            filepath: "/srv/b.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_a, "a".to_string()).await;
        f.seed_cfg_inst(&cfg_b, "b".to_string()).await;

        let ids = vec![cfg_a.id.clone(), cfg_b.id.clone()];
        let result = filepaths(&f.cfg_inst_meta, &ids).await.unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains("/srv/a.json"));
        assert!(result.contains("/srv/b.json"));
    }

    #[tokio::test]
    async fn missing_metadata_returns_error() {
        let f = Fixture::new().await;
        let ids = vec!["nonexistent".to_string()];
        let result = filepaths(&f.cfg_inst_meta, &ids).await;
        assert!(result.is_err());
    }
}
