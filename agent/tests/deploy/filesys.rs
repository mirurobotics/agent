// standard crates
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

// internal crates
use miru_agent::deploy::filesys::{deploy, remove};
use miru_agent::deploy::DeployErr;
use miru_agent::filesys::{self, FileSysErr, Overwrite, WriteOptions};
use miru_agent::models::{ConfigInstance, Deployment, DplActivity, DplTarget};
use miru_agent::storage;

// external crates
use serde_json::json;

struct Fixture {
    cfg_inst_meta: storage::CfgInsts,
    cfg_inst_content: storage::CfgInstContent,
    pub(super) temp_dir: filesys::Dir,
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

        Self {
            cfg_inst_meta,
            cfg_inst_content,
            temp_dir,
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

    fn new_removal_deployment(&self, cfg_insts: &[ConfigInstance]) -> Deployment {
        Deployment {
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Removing,
            config_instance_ids: cfg_insts.iter().map(|c| c.id.clone()).collect(),
            ..Default::default()
        }
    }

    fn storage_ref(&self) -> storage::CfgInstRef<'_> {
        storage::CfgInstRef {
            meta: &self.cfg_inst_meta,
            content: &self.cfg_inst_content,
        }
    }

    async fn deploy(&self, deployment: &Deployment) -> Result<(), DeployErr> {
        deploy(&self.storage_ref(), deployment).await
    }

    async fn remove(
        &self,
        deployment: &Deployment,
        keeps: &[filesys::File],
    ) -> Result<(), DeployErr> {
        remove(&self.storage_ref(), deployment, keeps).await
    }
}

pub mod deploy_func {
    use super::*;
    use miru_agent::filesys::PathExt;

    fn abs_path(f: &Fixture, rel: &str) -> String {
        f.temp_dir.path().join(rel).display().to_string()
    }

    #[tokio::test]
    async fn creates_new_file() {
        let f = Fixture::new().await;
        let filepath = abs_path(&f, "test/filepath.json");
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let f = Fixture::new().await;
        let filepath = abs_path(&f, "test/filepath");
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let new_content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, new_content.clone()).await;

        // pre-populate the file with old content
        let file = filesys::File::new(&filepath);
        file.write_json(&json!({"old": true}), WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, new_content);
    }

    #[tokio::test]
    async fn multiple_config_instances() {
        let f = Fixture::new().await;
        let mut cfg_insts = Vec::new();
        let mut contents = Vec::new();
        for i in 0..3 {
            let filepath = abs_path(&f, &format!("test/filepath{i}"));
            let cfg_inst = ConfigInstance {
                filepath,
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
            let actual = filesys::File::new(&cfg_inst.filepath)
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
            filepath: abs_path(&f, "test/filepath"),
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
            filepath: abs_path(&f, "test/filepath"),
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
    async fn target_dir_unchanged_on_write_failure() {
        let f = Fixture::new().await;

        // seed one valid config instance
        let good_cfg = ConfigInstance {
            filepath: abs_path(&f, "test/good"),
            ..Default::default()
        };
        f.seed_cfg_inst(&good_cfg, "{\"good\": true}".to_string())
            .await;

        // pre-populate an existing file at an absolute path under temp_dir
        let existing_path = abs_path(&f, "test/existing");
        let existing_file = filesys::File::new(&existing_path);
        let existing_content = json!({"existing": true});
        existing_file
            .write_json(&existing_content, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // create a deployment with a valid and an invalid config instance
        let bad_cfg = ConfigInstance {
            filepath: abs_path(&f, "test/bad"),
            ..Default::default()
        };
        // seed metadata for bad_cfg but NOT content -- this triggers a content read
        // failure BEFORE write_all is called, so good_cfg's file must not exist.
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

        // good_cfg's file should not exist because content lookup fails before
        // any writes are attempted
        assert!(
            !filesys::File::new(&good_cfg.filepath).exists(),
            "no files should be written when content lookup fails",
        );

        // the pre-existing file should be untouched
        let actual = existing_file
            .read_json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(actual, existing_content);
    }

    #[tokio::test]
    async fn write_files_creates_parent_directories_for_destinations() {
        let f = Fixture::new().await;
        let filepath = abs_path(&f, "a/b/c/config.json");
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"nested\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        // parent directories do not yet exist
        assert!(!f.temp_dir.path().join("a").exists());

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn deploy_is_idempotent() {
        let f = Fixture::new().await;
        let filepath = abs_path(&f, "test/filepath");
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));

        // deploy twice
        f.deploy(&deployment).await.unwrap();
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn nested_filepath() {
        let f = Fixture::new().await;
        let filepath = abs_path(&f, "deeply/nested/path/config.json");
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"nested\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    // ============================ absolute-config-paths tests ============================

    #[tokio::test]
    async fn writes_absolute_path_to_tempdir() {
        let f = Fixture::new().await;
        let filepath = f.temp_dir.path().join("config.json").display().to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = r#"{"key":"value"}"#.to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn rejects_eacces_with_permission_denied_err() {
        let f = Fixture::new().await;
        let locked_dir = f.temp_dir.path().join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let filepath = locked_dir.join("config.json").display().to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"locked\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        let result = f.deploy(&deployment).await;

        // restore permissions BEFORE assertions so tempdir drop can recurse
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // EACCES on atomic write surfaces as AtomicWriteFileErr.
        assert!(
            matches!(&result, Err(DeployErr::FileSysErr(FileSysErr::AtomicWriteFileErr(_)))),
            "expected FileSysErr(AtomicWriteFileErr), got {result:?}",
        );
        assert!(
            !filesys::File::new(&filepath).exists(),
            "file should not exist in locked dir"
        );
    }

    #[tokio::test]
    async fn write_files_rolls_back_existing_files_on_mid_failure() {
        let f = Fixture::new().await;

        // pre-seed two files with old content via filesys::File::write_string
        let a_path = f.temp_dir.path().join("a.json").display().to_string();
        let b_path = f.temp_dir.path().join("b.json").display().to_string();
        filesys::File::new(&a_path)
            .write_string("old_a", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        filesys::File::new(&b_path)
            .write_string("old_b", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // create locked subdir
        let locked_dir = f.temp_dir.path().join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let c_path = locked_dir.join("c.json").display().to_string();

        let a_cfg = ConfigInstance {
            filepath: a_path.clone(),
            ..Default::default()
        };
        let b_cfg = ConfigInstance {
            filepath: b_path.clone(),
            ..Default::default()
        };
        let c_cfg = ConfigInstance {
            filepath: c_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&a_cfg, "new_a".to_string()).await;
        f.seed_cfg_inst(&b_cfg, "new_b".to_string()).await;
        f.seed_cfg_inst(&c_cfg, "new_c".to_string()).await;

        let deployment = f.new_deployment(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;

        // restore permissions so tempdir drop can recurse
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // EACCES on atomic write surfaces as AtomicWriteFileErr.
        assert!(
            matches!(&result, Err(DeployErr::FileSysErr(FileSysErr::AtomicWriteFileErr(_)))),
            "expected FileSysErr(AtomicWriteFileErr), got {result:?}",
        );

        // a and b should be rolled back to old content
        let a_actual = filesys::File::new(&a_path).read_string().await.unwrap();
        assert_eq!(a_actual, "old_a");
        let b_actual = filesys::File::new(&b_path).read_string().await.unwrap();
        assert_eq!(b_actual, "old_b");

        // c should not exist
        assert!(
            !filesys::File::new(&c_path).exists(),
            "c.json should not exist in locked dir"
        );
    }

    #[tokio::test]
    async fn rejects_relative_filepath_with_pathnotallowed() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "relative/config.json".to_string(),
            ..Default::default()
        };
        let content = "{\"relative\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content).await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        let result = f.deploy(&deployment).await;

        match result {
            Err(DeployErr::PathNotAllowed(e)) => {
                assert!(
                    e.reason.contains("not absolute"),
                    "expected 'not absolute' reason, got: {}",
                    e.reason
                );
            }
            other => panic!("expected PathNotAllowed, got {other:?}"),
        }

        assert!(
            !filesys::File::new(Path::new("relative").join("config.json")).exists(),
            "relative path file should not exist",
        );
    }

    #[tokio::test]
    async fn rejects_parent_traversal_filepath() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "/etc/myapp/../passwd".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"traversal\": true}".to_string())
            .await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        let result = f.deploy(&deployment).await;

        match result {
            Err(DeployErr::PathNotAllowed(e)) => {
                assert!(
                    e.reason.contains("parent traversal"),
                    "expected 'parent traversal' reason, got: {}",
                    e.reason
                );
            }
            other => panic!("expected PathNotAllowed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_deployment_when_any_filepath_is_invalid() {
        let f = Fixture::new().await;

        let good_path = abs_path(&f, "good.json");
        let good_cfg = ConfigInstance {
            filepath: good_path.clone(),
            ..Default::default()
        };
        let bad_cfg = ConfigInstance {
            filepath: "relative/config.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&good_cfg, "good content".to_string()).await;
        f.seed_cfg_inst(&bad_cfg, "bad content".to_string()).await;

        // good_cfg is first; if validation happened in-loop instead of as a
        // pre-pass, good.json would be on disk after this call.
        let deployment = f.new_deployment(&[good_cfg.clone(), bad_cfg.clone()]);
        let result = f.deploy(&deployment).await;

        match &result {
            Err(DeployErr::PathNotAllowed(e)) => {
                assert!(
                    e.reason.contains("not absolute"),
                    "expected 'not absolute' reason, got: {}",
                    e.reason
                );
            }
            other => panic!("expected PathNotAllowed, got {other:?}"),
        }

        // The pre-pass must reject the deployment BEFORE any write happens.
        assert!(
            !filesys::File::new(&good_path).exists(),
            "good.json should not exist — validate_cfg_insts must reject the deployment before any writes"
        );
        assert!(
            !filesys::File::new(PathBuf::from("relative/config.json")).exists(),
            "relative path must not be created"
        );
        let leftover = backup_siblings_in(f.temp_dir.path());
        assert!(
            leftover.is_empty(),
            "expected no miru.backup.* siblings; validate_cfg_insts must run before snapshot, found {leftover:?}"
        );
    }

    // ============================ fourth-pass write_files tests ============================

    /// Returns the entries in `dir` whose filename starts with the literal
    /// `miru.backup.` prefix emitted by `backup_location`. Used to assert
    /// that snapshot backups are not leaked on the success path.
    fn backup_siblings_in(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("miru.backup.") {
                out.push(entry.path());
            }
        }
        out
    }

    #[tokio::test]
    async fn write_files_creates_all_new_files_when_none_existed() {
        let f = Fixture::new().await;

        let a_path = f.temp_dir.path().join("a.json").display().to_string();
        let b_path = f.temp_dir.path().join("b.json").display().to_string();
        let a_cfg = ConfigInstance {
            filepath: a_path.clone(),
            ..Default::default()
        };
        let b_cfg = ConfigInstance {
            filepath: b_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&a_cfg, "new_a".to_string()).await;
        f.seed_cfg_inst(&b_cfg, "new_b".to_string()).await;

        let deployment = f.new_deployment(&[a_cfg, b_cfg]);
        f.deploy(&deployment).await.unwrap();

        let a_actual = filesys::File::new(&a_path).read_string().await.unwrap();
        assert_eq!(a_actual, "new_a");
        let b_actual = filesys::File::new(&b_path).read_string().await.unwrap();
        assert_eq!(b_actual, "new_b");

        let leftover = backup_siblings_in(f.temp_dir.path());
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_overwrites_all_when_all_existed() {
        let f = Fixture::new().await;

        let a_path = f.temp_dir.path().join("a.json").display().to_string();
        let b_path = f.temp_dir.path().join("b.json").display().to_string();

        // pre-populate both destinations with old content via the agent's
        // atomic write helper
        filesys::File::new(PathBuf::from(&a_path))
            .write_string("old_a", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        filesys::File::new(PathBuf::from(&b_path))
            .write_string("old_b", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let a_cfg = ConfigInstance {
            filepath: a_path.clone(),
            ..Default::default()
        };
        let b_cfg = ConfigInstance {
            filepath: b_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&a_cfg, "new_a".to_string()).await;
        f.seed_cfg_inst(&b_cfg, "new_b".to_string()).await;

        let deployment = f.new_deployment(&[a_cfg, b_cfg]);
        f.deploy(&deployment).await.unwrap();

        let a_actual = filesys::File::new(PathBuf::from(&a_path))
            .read_string()
            .await
            .unwrap();
        assert_eq!(a_actual, "new_a");
        let b_actual = filesys::File::new(PathBuf::from(&b_path))
            .read_string()
            .await
            .unwrap();
        assert_eq!(b_actual, "new_b");

        let leftover = backup_siblings_in(f.temp_dir.path());
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_rolls_back_new_files_by_deleting_on_mid_failure() {
        let f = Fixture::new().await;

        // first two destinations are fresh tempdir paths (DidNotExist snapshots)
        let a_path = f.temp_dir.path().join("a.json").display().to_string();
        let b_path = f.temp_dir.path().join("b.json").display().to_string();

        // third destination lives under a locked subdirectory so the write fails
        let locked_dir = f.temp_dir.path().join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let c_path = locked_dir.join("c.json").display().to_string();

        let a_cfg = ConfigInstance {
            filepath: a_path.clone(),
            ..Default::default()
        };
        let b_cfg = ConfigInstance {
            filepath: b_path.clone(),
            ..Default::default()
        };
        let c_cfg = ConfigInstance {
            filepath: c_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&a_cfg, "new_a".to_string()).await;
        f.seed_cfg_inst(&b_cfg, "new_b".to_string()).await;
        f.seed_cfg_inst(&c_cfg, "new_c".to_string()).await;

        let deployment = f.new_deployment(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;

        // restore permissions so tempdir drop can recurse
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // EACCES on atomic write surfaces as AtomicWriteFileErr.
        assert!(
            matches!(&result, Err(DeployErr::FileSysErr(FileSysErr::AtomicWriteFileErr(_)))),
            "expected FileSysErr(AtomicWriteFileErr), got {result:?}",
        );

        // first two destinations were created then rolled back via delete
        assert!(
            !filesys::File::new(&a_path).exists(),
            "a.json should have been removed by DidNotExist rollback"
        );
        assert!(
            !filesys::File::new(&b_path).exists(),
            "b.json should have been removed by DidNotExist rollback"
        );
        // third destination never existed
        assert!(
            !filesys::File::new(&c_path).exists(),
            "c.json should not exist in locked dir"
        );

        // both snapshots were DidNotExist so no backup siblings were created
        let leftover = backup_siblings_in(f.temp_dir.path());
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_rolls_back_mixed_existed_and_did_not_exist_in_same_call() {
        let f = Fixture::new().await;

        // Existed: pre-populate a.json with "old_a"
        let a_path = f.temp_dir.path().join("a.json").display().to_string();
        filesys::File::new(&a_path)
            .write_string("old_a", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // DidNotExist: fresh path b.json
        let b_path = f.temp_dir.path().join("b.json").display().to_string();

        // Failing destination: locked_dir/c.json
        let locked_dir = f.temp_dir.path().join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let c_path = locked_dir.join("c.json").display().to_string();

        let a_cfg = ConfigInstance {
            filepath: a_path.clone(),
            ..Default::default()
        };
        let b_cfg = ConfigInstance {
            filepath: b_path.clone(),
            ..Default::default()
        };
        let c_cfg = ConfigInstance {
            filepath: c_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&a_cfg, "new_a".to_string()).await;
        f.seed_cfg_inst(&b_cfg, "new_b".to_string()).await;
        f.seed_cfg_inst(&c_cfg, "new_c".to_string()).await;

        let deployment = f.new_deployment(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;

        // restore permissions so tempdir drop can recurse
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // EACCES on atomic write surfaces as AtomicWriteFileErr.
        assert!(
            matches!(&result, Err(DeployErr::FileSysErr(FileSysErr::AtomicWriteFileErr(_)))),
            "expected FileSysErr(AtomicWriteFileErr), got {result:?}",
        );

        // Existed snapshot was restored via rename-back
        let a_actual = filesys::File::new(&a_path).read_string().await.unwrap();
        assert_eq!(a_actual, "old_a");

        // DidNotExist snapshot was rolled back via delete
        assert!(
            !filesys::File::new(&b_path).exists(),
            "b.json should have been removed by DidNotExist rollback"
        );

        // failing destination never existed
        assert!(
            !filesys::File::new(&c_path).exists(),
            "c.json should not exist in locked dir"
        );

        // the backup created for a.json's Existed snapshot should have been
        // renamed back over a.json, leaving no .miru-backup-* sibling
        let leftover = backup_siblings_in(f.temp_dir.path());
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_returns_snapshot_failure_when_initial_read_fails() {
        let f = Fixture::new().await;

        // create locked_dir and write c.json with "old" content BEFORE locking
        let locked_dir = f.temp_dir.path().join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        let c_path = locked_dir.join("c.json").display().to_string();
        filesys::File::new(&c_path)
            .write_string("old", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // now lock the parent directory so snapshot_destination's sibling
        // backup copy cannot succeed
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let c_cfg = ConfigInstance {
            filepath: c_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&c_cfg, "new".to_string()).await;

        let deployment = f.new_deployment(std::slice::from_ref(&c_cfg));
        let result = f.deploy(&deployment).await;

        // restore permissions so tempdir drop can recurse
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // EACCES on copy_to (snapshot backup) surfaces as CopyFileErr.
        assert!(
            matches!(&result, Err(DeployErr::FileSysErr(FileSysErr::CopyFileErr(_)))),
            "expected FileSysErr(CopyFileErr), got {result:?}",
        );

        // c.json content must be unchanged
        let c_actual = filesys::File::new(&c_path).read_string().await.unwrap();
        assert_eq!(c_actual, "old");

        // no backup siblings leaked next to c.json
        let leftover = backup_siblings_in(&locked_dir);
        assert!(
            leftover.is_empty(),
            "expected no miru.backup.* siblings near c.json, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_does_not_leak_temp_files_on_success() {
        let f = Fixture::new().await;

        let filepath = f.temp_dir.path().join("config.json").display().to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"ok\": true}".to_string())
            .await;

        let deployment = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        // walk the tempdir root. Filter out the destination file and the
        // Fixture's own `resources` subdir. Assert nothing matches the
        // `.miru-tmp-*` or `.miru-backup-*` sibling naming pattern.
        let mut leaked: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(f.temp_dir.path()).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "config.json" || name == "resources" {
                continue;
            }
            if name.starts_with('.')
                && (name.contains(".miru-tmp-") || name.contains(".miru-backup-"))
            {
                leaked.push(name);
            }
        }
        assert!(
            leaked.is_empty(),
            "expected no .miru-tmp-* or .miru-backup-* siblings in tempdir root, found {leaked:?}"
        );
    }
}

pub mod remove_func {
    use super::*;
    use miru_agent::filesys::PathExt;

    fn abs_path(f: &Fixture, rel: &str) -> String {
        f.temp_dir.path().join(rel).display().to_string()
    }

    /// Helper: seed a cfg_inst, deploy it so the file exists on disk, then return
    /// a removal deployment referencing the same cfg_inst.
    async fn seed_and_deploy(f: &Fixture, rel: &str, content: &str) -> ConfigInstance {
        let filepath = abs_path(f, rel);
        let cfg_inst = ConfigInstance {
            filepath,
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, content.to_string()).await;
        let deploy_dpl = f.new_deployment(std::slice::from_ref(&cfg_inst));
        f.deploy(&deploy_dpl).await.unwrap();
        cfg_inst
    }

    #[tokio::test]
    async fn removes_file() {
        let f = Fixture::new().await;
        let ci = seed_and_deploy(&f, "config.json", r#"{"v": 1}"#).await;
        let dest = filesys::File::new(&ci.filepath);
        assert!(dest.path().exists(), "file should exist before removal");

        let dpl = f.new_removal_deployment(std::slice::from_ref(&ci));
        f.remove(&dpl, &[]).await.unwrap();
        assert!(
            !dest.path().exists(),
            "file should be deleted after removal"
        );
    }

    #[tokio::test]
    async fn removes_multiple_files() {
        let f = Fixture::new().await;
        let ci_a = seed_and_deploy(&f, "a.json", r#"{"a": 1}"#).await;
        let ci_b = seed_and_deploy(&f, "nested/b.json", r#"{"b": 2}"#).await;
        let dest_a = filesys::File::new(&ci_a.filepath);
        let dest_b = filesys::File::new(&ci_b.filepath);

        let dpl = f.new_removal_deployment(&[ci_a, ci_b]);
        f.remove(&dpl, &[]).await.unwrap();
        assert!(!dest_a.path().exists(), "a.json should be deleted");
        assert!(!dest_b.path().exists(), "b.json should be deleted");
    }

    #[tokio::test]
    async fn keeps_protected_files() {
        let f = Fixture::new().await;
        let ci_keep = seed_and_deploy(&f, "keep.json", r#"{"keep": true}"#).await;
        let ci_remove = seed_and_deploy(&f, "remove.json", r#"{"remove": true}"#).await;
        let keep_file = filesys::File::new(&ci_keep.filepath);
        let remove_file = filesys::File::new(&ci_remove.filepath);

        let dpl = f.new_removal_deployment(&[ci_keep, ci_remove]);
        f.remove(&dpl, std::slice::from_ref(&keep_file))
            .await
            .unwrap();

        assert!(keep_file.path().exists(), "protected file should survive");
        assert!(
            !remove_file.path().exists(),
            "unprotected file should be deleted"
        );
    }

    #[tokio::test]
    async fn idempotent_when_file_missing() {
        let f = Fixture::new().await;
        let filepath = abs_path(&f, "nonexistent.json");
        let ci = ConfigInstance {
            filepath,
            ..Default::default()
        };
        // seed metadata only — no file on disk
        f.seed_cfg_inst(&ci, "{}".to_string()).await;

        let dpl = f.new_removal_deployment(std::slice::from_ref(&ci));
        // should succeed even though file doesn't exist (File::delete is idempotent)
        f.remove(&dpl, &[]).await.unwrap();
    }

    #[tokio::test]
    async fn empty_config_instance_ids_is_noop() {
        let f = Fixture::new().await;
        let dpl = Deployment {
            target_status: DplTarget::Archived,
            activity_status: DplActivity::Removing,
            config_instance_ids: vec![],
            ..Default::default()
        };
        // nothing to remove — should succeed as a no-op
        f.remove(&dpl, &[]).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_relative_filepath() {
        let f = Fixture::new().await;
        let ci = ConfigInstance {
            filepath: "relative/path.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&ci, "{}".to_string()).await;

        let dpl = f.new_removal_deployment(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;
        assert!(result.is_err(), "should reject relative filepath");
    }

    #[tokio::test]
    async fn rejects_parent_traversal_filepath() {
        let f = Fixture::new().await;
        let ci = ConfigInstance {
            filepath: "/etc/myapp/../passwd".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&ci, "{}".to_string()).await;

        let dpl = f.new_removal_deployment(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;
        match result {
            Err(DeployErr::PathNotAllowed(e)) => {
                assert!(
                    e.reason.contains("parent traversal"),
                    "expected 'parent traversal' reason, got: {}",
                    e.reason
                );
            }
            other => panic!("expected PathNotAllowed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_config_instance_metadata_returns_error() {
        let f = Fixture::new().await;
        let ci = ConfigInstance {
            id: "nonexistent-ci".to_string(),
            filepath: f.temp_dir.path().join("missing.json").display().to_string(),
            ..Default::default()
        };
        // do NOT seed metadata — only reference the ID
        let dpl = f.new_removal_deployment(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;
        assert!(
            matches!(result, Err(DeployErr::CacheErr(_))),
            "expected CacheErr for missing metadata, got {result:?}"
        );
    }

    #[tokio::test]
    async fn delete_error_is_swallowed() {
        let f = Fixture::new().await;

        // deploy a file to a directory, then lock the directory so delete fails
        let ci = seed_and_deploy(&f, "locked/config.json", r#"{"v": 1}"#).await;
        let dest = filesys::File::new(&ci.filepath);
        assert!(dest.path().exists(), "file should exist before removal");

        // lock the parent directory so remove_file fails with EACCES
        let parent = dest.path().parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o555)).unwrap();

        let dpl = f.new_removal_deployment(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;

        // restore permissions so tempdir drop can recurse
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();

        // deletion errors are logged but swallowed — remove returns Ok
        assert!(
            result.is_ok(),
            "remove should succeed even when deletion fails (errors are logged)"
        );
        assert!(
            dest.path().exists(),
            "file should still exist since delete was blocked"
        );
    }
}
