// standard crates
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

// internal crates
use miru_agent::deploy::filesys::{deploy, remove, BACKUP_FILE_PREFIX};
use miru_agent::deploy::DeployErr;
use miru_agent::filesys::{self, Overwrite, PathExt, WriteOptions};
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

    async fn fixture_path(&self, rel: &str) -> String {
        self.temp_dir.path().join(rel).display().to_string()
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

    fn new_queued(&self, cfg_insts: &[ConfigInstance]) -> Deployment {
        Deployment {
            target_status: DplTarget::Deployed,
            activity_status: DplActivity::Queued,
            config_instance_ids: cfg_insts.iter().map(|c| c.id.clone()).collect(),
            ..Default::default()
        }
    }

    fn new_staged(&self, cfg_insts: &[ConfigInstance]) -> Deployment {
        Deployment {
            target_status: DplTarget::Staged,
            activity_status: DplActivity::Staged,
            config_instance_ids: cfg_insts.iter().map(|c| c.id.clone()).collect(),
            ..Default::default()
        }
    }

    fn new_removing(&self, cfg_insts: &[ConfigInstance]) -> Deployment {
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

fn read_only() -> std::fs::Permissions {
    std::fs::Permissions::from_mode(0o555)
}

fn writeable() -> std::fs::Permissions {
    std::fs::Permissions::from_mode(0o755)
}

/// Returns the entries in `dir` whose filename starts with the literal
/// `miru.backup.` prefix emitted by `backup_location`.
fn detect_backup_files(dir: &filesys::Dir) -> Vec<filesys::File> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(BACKUP_FILE_PREFIX) {
            out.push(filesys::File::new(entry.path()));
        }
    }
    out
}

pub mod deploy_func_success {
    use super::*;

    #[tokio::test]
    async fn creates_new_file() {
        let f = Fixture::new().await;
        let filepath = f.fixture_path("test/filepath.json").await;
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let f = Fixture::new().await;
        let filepath = f.fixture_path("test/filepath").await;
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

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
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
            let filepath = f.fixture_path(&format!("test/filepath{i}")).await;
            let cfg_inst = ConfigInstance {
                filepath,
                ..Default::default()
            };
            let content = format!("{{\"index\": {i}}}");
            f.seed_cfg_inst(&cfg_inst, content.clone()).await;
            cfg_insts.push(cfg_inst);
            contents.push(content);
        }

        let deployment = f.new_queued(&cfg_insts);
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
    async fn backup_files_not_leaked() {
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

        let deployment = f.new_queued(&[a_cfg, b_cfg]);
        f.deploy(&deployment).await.unwrap();

        let a_actual = filesys::File::new(&a_path).read_string().await.unwrap();
        assert_eq!(a_actual, "new_a");
        let b_actual = filesys::File::new(&b_path).read_string().await.unwrap();
        assert_eq!(b_actual, "new_b");

        let leftover = detect_backup_files(&f.temp_dir);
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn deploy_is_idempotent() {
        let f = Fixture::new().await;
        let filepath = f.fixture_path("test/filepath").await;
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"speed\": 4}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));

        // deploy twice
        f.deploy(&deployment).await.unwrap();
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn nested_filepath() {
        let f = Fixture::new().await;
        let filepath = f.fixture_path("deeply/nested/path/config.json").await;
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"nested\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn write_files_creates_parent_directories() {
        let f = Fixture::new().await;
        let filepath = f.fixture_path("a/b/c/config.json").await;
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"nested\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content.clone()).await;

        // parent directories do not yet exist
        assert!(!f.temp_dir.path().join("a").exists());

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
        f.deploy(&deployment).await.unwrap();

        let actual = filesys::File::new(&filepath).read_string().await.unwrap();
        assert_eq!(actual, content);
    }

    #[tokio::test]
    async fn stale_backup_overwritten() {
        let f = Fixture::new().await;

        // Pre-populate a.json with old content
        let a_path = f.temp_dir.path().join("a.json").display().to_string();
        filesys::File::new(&a_path)
            .write_string("old_a", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // Simulate a stale backup left by a prior interrupted deploy
        let stale_backup = f.temp_dir.path().join("miru.backup.a.json");
        std::fs::write(&stale_backup, "stale_backup").unwrap();
        assert!(stale_backup.exists());

        let a_cfg = ConfigInstance {
            filepath: a_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&a_cfg, "new_a".to_string()).await;

        let deployment = f.new_queued(std::slice::from_ref(&a_cfg));
        f.deploy(&deployment).await.unwrap();

        // a.json should have new content
        let actual = filesys::File::new(&a_path).read_string().await.unwrap();
        assert_eq!(actual, "new_a");

        // stale backup overwritten by snapshot then removed by cleanup_backups
        let leftover = detect_backup_files(&f.temp_dir);
        assert!(
            leftover.is_empty(),
            "expected stale backup to be cleaned up, found {leftover:?}"
        );
    }
}

pub mod deploy_func_validation_errs {
    use super::*;
    use miru_agent::filesys::PathExt;

    #[tokio::test]
    async fn wrong_target_status_returns_error() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: f.fixture_path("test/filepath").await,
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, "{\"ok\": true}".to_string())
            .await;

        let deployment = f.new_staged(std::slice::from_ref(&cfg_inst));

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
    async fn empty_config_instance_ids_returns_error() {
        let f = Fixture::new().await;
        let deployment = f.new_queued(&[]);

        let result = f.deploy(&deployment).await;
        assert!(matches!(result, Err(DeployErr::EmptyConfigInstances(_))));
    }

    #[tokio::test]
    async fn missing_config_instance_returns_error() {
        let f = Fixture::new().await;
        // deployment references a config instance ID that doesn't exist in the cache
        let deployment = Deployment {
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["nonexistent-id".to_string()],
            ..Default::default()
        };
        let result = f.deploy(&deployment).await;
        assert!(
            matches!(result, Err(DeployErr::CacheErr(_))),
            "expected CacheErr for missing config instance, got {result:?}"
        );
    }

    #[tokio::test]
    async fn missing_content_returns_error() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: f.fixture_path("test/filepath").await,
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

        let deployment = f.new_queued(&[cfg_inst]);
        let result = f.deploy(&deployment).await;
        assert!(
            matches!(result, Err(DeployErr::CacheErr(_))),
            "expected CacheErr for missing config content, got {result:?}"
        );
    }

    #[tokio::test]
    async fn duplicate_cfg_inst_filepaths() {
        let f = Fixture::new().await;

        let filepath = f.temp_dir.path().join("config.json").display().to_string();

        let cfg_inst_1 = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let cfg_inst_2 = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst_1, "content_1".to_string()).await;
        f.seed_cfg_inst(&cfg_inst_2, "content_2".to_string()).await;

        let deployment = f.new_queued(&[cfg_inst_1, cfg_inst_2]);
        let result = f.deploy(&deployment).await;

        assert!(
            matches!(result, Err(DeployErr::DuplicateFilepath(_))),
            "expected DuplicateFilepath error, got {result:?}"
        );
    }

    #[tokio::test]
    async fn duplicate_cfg_inst_filepaths_after_normalization() {
        let f = Fixture::new().await;

        let base = f.temp_dir.path().display().to_string();
        // These differ as raw strings but normalize to the same path
        let raw_a = format!("{base}/./config.json");
        let raw_b = format!("{base}/config.json");
        assert_ne!(raw_a, raw_b, "raw strings must differ for this test");

        let cfg_inst_1 = ConfigInstance {
            filepath: raw_a,
            ..Default::default()
        };
        let cfg_inst_2 = ConfigInstance {
            filepath: raw_b,
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst_1, "content_1".to_string()).await;
        f.seed_cfg_inst(&cfg_inst_2, "content_2".to_string()).await;

        let deployment = f.new_queued(&[cfg_inst_1, cfg_inst_2]);
        let result = f.deploy(&deployment).await;

        assert!(
            matches!(result, Err(DeployErr::DuplicateFilepath(_))),
            "expected DuplicateFilepath error for normalized-duplicate paths, got {result:?}"
        );
    }

    #[tokio::test]
    async fn relative_filepath_rejected() {
        let f = Fixture::new().await;
        let cfg_inst = ConfigInstance {
            filepath: "relative/config.json".to_string(),
            ..Default::default()
        };
        let content = "{\"relative\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content).await;

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
        let result = f.deploy(&deployment).await;

        assert!(
            matches!(result, Err(DeployErr::PathNotAllowed(_))),
            "expected PathNotAllowed, got {result:?}"
        );

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

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
        let result = f.deploy(&deployment).await;

        assert!(
            matches!(result, Err(DeployErr::PathNotAllowed(_))),
            "expected PathNotAllowed, got {result:?}"
        );

        assert!(
            !filesys::File::new(Path::new("/etc/myapp/../passwd")).exists(),
            "parent traversal path file should not exist",
        );
    }

    #[tokio::test]
    async fn rejects_deployment_when_any_filepath_is_invalid() {
        let f = Fixture::new().await;

        let good_path = f.fixture_path("good.json").await;
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
        let deployment = f.new_queued(&[good_cfg.clone(), bad_cfg.clone()]);
        let result = f.deploy(&deployment).await;
        assert!(
            matches!(result, Err(DeployErr::PathNotAllowed(_))),
            "expected PathNotAllowed, got {result:?}"
        );

        // The pre-pass must reject the deployment BEFORE any write happens.
        assert!(
            !filesys::File::new(&good_path).exists(),
            "good.json should not exist — validate_cfg_insts must reject the deployment before any writes"
        );
        assert!(
            !filesys::File::new(PathBuf::from("relative").join("config.json")).exists(),
            "relative path must not be created"
        );
    }
}

pub mod deploy_func_backup_errs {
    use super::*;
    use miru_agent::filesys::PathExt;

    #[tokio::test]
    async fn copy_file_for_backup_permission_denied() {
        let f = Fixture::new().await;

        // create locked_dir and write c.json with "old" content BEFORE locking
        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        let c_path = locked_dir.file("c.json").path().display().to_string();
        filesys::File::new(&c_path)
            .write_string("old", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // now lock the parent directory so snapshot_destination's sibling
        // backup copy cannot succeed
        locked_dir.set_permissions(read_only()).await.unwrap();

        let c_cfg = ConfigInstance {
            filepath: c_path.clone(),
            ..Default::default()
        };
        f.seed_cfg_inst(&c_cfg, "new".to_string()).await;

        let deployment = f.new_queued(std::slice::from_ref(&c_cfg));
        let result = f.deploy(&deployment).await;
        assert!(
            matches!(&result, Err(DeployErr::BackupAccessDenied(_))),
            "expected BackupAccessDenied, got {result:?}"
        );

        // restore permissions
        locked_dir.set_permissions(writeable()).await.unwrap();

        // c.json content must be unchanged
        let c_actual = filesys::File::new(&c_path).read_string().await.unwrap();
        assert_eq!(c_actual, "old");

        // no backup siblings leaked next to c.json
        let leftover = detect_backup_files(&locked_dir);
        assert!(
            leftover.is_empty(),
            "expected no miru.backup.* siblings near c.json, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn copy_backups_failure_retains_original_files() {
        let f = Fixture::new().await;

        // Pre-populate a.json and b.json in writable temp_dir root
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

        // Pre-populate c.json in a subdir, then lock the subdir so snapshot's
        // backup copy cannot create miru.backup.c.json (EACCES)
        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        let c_path = locked_dir.file("c.json").path().display().to_string();
        filesys::File::new(&c_path)
            .write_string("old_c", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        locked_dir.set_permissions(read_only()).await.unwrap();

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

        let deployment = f.new_queued(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;
        assert!(
            matches!(&result, Err(DeployErr::BackupAccessDenied(_))),
            "expected BackupAccessDenied, got {result:?}"
        );

        // restore permissions
        locked_dir.set_permissions(writeable()).await.unwrap();

        // files should be unchanged
        let a_actual = filesys::File::new(&a_path).read_string().await.unwrap();
        assert_eq!(a_actual, "old_a");
        let b_actual = filesys::File::new(&b_path).read_string().await.unwrap();
        assert_eq!(b_actual, "old_b");
        let c_actual = filesys::File::new(&c_path).read_string().await.unwrap();
        assert_eq!(c_actual, "old_c");

        // backup files consumed by rollback (rename-back), none should remain
        let leftover = detect_backup_files(&f.temp_dir);
        assert!(
            leftover.is_empty(),
            "expected no miru.backup.* siblings in temp_dir root, found {leftover:?}"
        );
    }
}

pub mod deploy_func_write_errs {
    use super::*;
    use miru_agent::filesys::PathExt;

    #[tokio::test]
    async fn write_file_permission_denied() {
        let f = Fixture::new().await;
        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        locked_dir.set_permissions(read_only()).await.unwrap();

        let filepath = locked_dir.file("config.json").path().display().to_string();
        let cfg_inst = ConfigInstance {
            filepath: filepath.clone(),
            ..Default::default()
        };
        let content = "{\"locked\": true}".to_string();
        f.seed_cfg_inst(&cfg_inst, content).await;

        let deployment = f.new_queued(std::slice::from_ref(&cfg_inst));
        let result = f.deploy(&deployment).await;
        assert!(matches!(result, Err(DeployErr::WriteAccessDenied(_))));

        assert!(
            !filesys::File::new(&filepath).exists(),
            "file should not exist in locked dir"
        );

        // backups should not be leaked
        let leftover = detect_backup_files(&locked_dir);
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_restores_existing_files_on_mid_failure() {
        let f = Fixture::new().await;

        // pre-seed two files with old content via filesys::File::write_string
        let a_path = f.temp_dir.file("a.json").path().display().to_string();
        let b_path = f.temp_dir.file("b.json").path().display().to_string();
        filesys::File::new(&a_path)
            .write_string("old_a", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        filesys::File::new(&b_path)
            .write_string("old_b", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // create locked subdir
        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        locked_dir.set_permissions(read_only()).await.unwrap();
        let c_path = locked_dir.file("c.json").path().display().to_string();

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

        let deployment = f.new_queued(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;
        assert!(matches!(result, Err(DeployErr::WriteAccessDenied(_))));

        // restore permissions
        locked_dir.set_permissions(writeable()).await.unwrap();

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

        // backups should not be leaked
        let leftover = detect_backup_files(&f.temp_dir);
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_deletes_new_files_on_mid_failure() {
        let f = Fixture::new().await;

        // first two destinations are fresh tempdir paths (DidNotExist snapshots)
        let a_path = f.temp_dir.file("a.json").path().display().to_string();
        let b_path = f.temp_dir.file("b.json").path().display().to_string();

        // third destination lives under a locked subdirectory so the write fails
        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        locked_dir.set_permissions(read_only()).await.unwrap();
        let c_path = locked_dir.file("c.json").path().display().to_string();

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

        let deployment = f.new_queued(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;
        assert!(matches!(&result, Err(DeployErr::WriteAccessDenied(_))));

        // restore permissions
        locked_dir.set_permissions(writeable()).await.unwrap();

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

        // backups should not be leaked
        let leftover = detect_backup_files(&f.temp_dir);
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }

    #[tokio::test]
    async fn write_files_rolls_back_mixed_existed_and_did_not_exist_in_same_call() {
        let f = Fixture::new().await;

        // Existed: pre-populate a.json with "old_a"
        let a_path = f.temp_dir.file("a.json").path().display().to_string();
        filesys::File::new(&a_path)
            .write_string("old_a", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // DidNotExist: fresh path b.json
        let b_path = f.temp_dir.file("b.json").path().display().to_string();

        // Failing destination: locked_dir/c.json
        let locked_dir = f.temp_dir.subdir("locked");
        locked_dir.create().await.unwrap();
        locked_dir.set_permissions(read_only()).await.unwrap();
        let c_path = locked_dir.file("c.json").path().display().to_string();

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

        let deployment = f.new_queued(&[a_cfg, b_cfg, c_cfg]);
        let result = f.deploy(&deployment).await;
        assert!(matches!(&result, Err(DeployErr::WriteAccessDenied(_))));

        // restore permissions so tempdir drop can recurse
        locked_dir.set_permissions(writeable()).await.unwrap();

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
        let leftover = detect_backup_files(&f.temp_dir);
        assert!(
            leftover.is_empty(),
            "expected no .miru-backup-* siblings, found {leftover:?}"
        );
    }
}

pub mod remove_func_success {
    use super::*;
    use miru_agent::filesys::PathExt;

    /// Helper: seed a cfg_inst, deploy it so the file exists on disk, then return
    /// a removal deployment referencing the same cfg_inst.
    async fn seed_and_deploy(f: &Fixture, rel: &str, content: &str) -> ConfigInstance {
        let filepath = f.fixture_path(rel).await;
        let cfg_inst = ConfigInstance {
            filepath,
            ..Default::default()
        };
        f.seed_cfg_inst(&cfg_inst, content.to_string()).await;
        let deploy_dpl = f.new_queued(std::slice::from_ref(&cfg_inst));
        f.deploy(&deploy_dpl).await.unwrap();
        cfg_inst
    }

    #[tokio::test]
    async fn remove_one_file() {
        let f = Fixture::new().await;
        let ci = seed_and_deploy(&f, "config.json", r#"{"v": 1}"#).await;
        let dest = filesys::File::new(&ci.filepath);
        assert!(dest.path().exists(), "file should exist before removal");

        let dpl = f.new_removing(std::slice::from_ref(&ci));
        f.remove(&dpl, &[]).await.unwrap();
        assert!(
            !dest.path().exists(),
            "file should be deleted after removal"
        );
    }

    #[tokio::test]
    async fn remove_multiple_files() {
        let f = Fixture::new().await;
        let ci_a = seed_and_deploy(&f, "a.json", r#"{"a": 1}"#).await;
        let ci_b = seed_and_deploy(&f, "nested/b.json", r#"{"b": 2}"#).await;
        let dest_a = filesys::File::new(&ci_a.filepath);
        let dest_b = filesys::File::new(&ci_b.filepath);

        let dpl = f.new_removing(&[ci_a, ci_b]);
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

        let dpl = f.new_removing(&[ci_keep, ci_remove]);
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
        let filepath = f.fixture_path("nonexistent.json").await;
        let ci = ConfigInstance {
            filepath,
            ..Default::default()
        };
        // seed metadata only — no file on disk
        f.seed_cfg_inst(&ci, "{}".to_string()).await;

        let dpl = f.new_removing(std::slice::from_ref(&ci));
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
    async fn delete_error_is_propagated() {
        let f = Fixture::new().await;

        // deploy a file to a directory, then lock the directory so delete fails
        let ci = seed_and_deploy(&f, "locked/config.json", r#"{"v": 1}"#).await;
        let dest = filesys::File::new(&ci.filepath);
        assert!(dest.path().exists(), "file should exist before removal");

        // lock the parent directory so remove_file fails with EACCES
        let parent = dest.parent().unwrap();
        parent.set_permissions(read_only()).await.unwrap();

        let dpl = f.new_removing(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;

        // restore permissions so tempdir drop can recurse
        parent.set_permissions(writeable()).await.unwrap();

        // deletion errors are now propagated
        assert!(
            result.is_err(),
            "remove should return error when deletion fails"
        );
        assert!(
            dest.path().exists(),
            "file should still exist since delete was blocked"
        );
    }
}

pub mod remove_func_errs {
    use super::*;

    #[tokio::test]
    async fn missing_cfg_inst_returns_error() {
        let f = Fixture::new().await;
        let ci = ConfigInstance {
            id: "nonexistent-ci".to_string(),
            filepath: f.temp_dir.path().join("missing.json").display().to_string(),
            ..Default::default()
        };
        // do NOT seed metadata — only reference the ID
        let dpl = f.new_removing(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;
        assert!(
            matches!(result, Err(DeployErr::CacheErr(_))),
            "expected CacheErr for missing metadata, got {result:?}"
        );
    }

    #[tokio::test]
    async fn relative_filepath_rejected() {
        let f = Fixture::new().await;
        let ci = ConfigInstance {
            filepath: "relative/path.json".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&ci, "{}".to_string()).await;

        let dpl = f.new_removing(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;
        assert!(
            matches!(result, Err(DeployErr::PathNotAllowed(_))),
            "should reject relative filepath with PathNotAllowed, got {result:?}"
        );
    }

    #[tokio::test]
    async fn rejects_parent_traversal_filepath() {
        let f = Fixture::new().await;
        let ci = ConfigInstance {
            filepath: "/etc/myapp/../passwd".to_string(),
            ..Default::default()
        };
        f.seed_cfg_inst(&ci, "{}".to_string()).await;

        let dpl = f.new_removing(std::slice::from_ref(&ci));
        let result = f.remove(&dpl, &[]).await;
        assert!(
            matches!(result, Err(DeployErr::PathNotAllowed(_))),
            "expected PathNotAllowed, got {result:?}"
        );
    }

    #[tokio::test]
    async fn rejects_deployment_when_any_filepath_is_invalid() {
        let f = Fixture::new().await;

        let good_path = f.fixture_path("good.json").await;
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

        // Create the good file up front. If validation happened in-loop rather
        // than as a pre-pass, this file would be removed before the bad path is
        // discovered.
        filesys::File::new(&good_path)
            .write_string("on_disk_before_remove", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let deployment = f.new_removing(&[good_cfg, bad_cfg]);
        let result = f.remove(&deployment, &[]).await;
        assert!(
            matches!(result, Err(DeployErr::PathNotAllowed(_))),
            "expected PathNotAllowed, got {result:?}"
        );

        // The pre-pass must reject the deployment BEFORE any remove happens.
        assert!(
            filesys::File::new(&good_path).exists(),
            "good.json should still exist — validate_cfg_insts must reject before any deletes"
        );
        assert!(
            !filesys::File::new(PathBuf::from("relative").join("config.json")).exists(),
            "relative path must not be created"
        );
    }
}
