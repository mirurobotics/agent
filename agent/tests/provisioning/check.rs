// standard crates
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

// internal crates
use miru_agent::disk::{DiskErr, Layout};
use miru_agent::errors::Trace;
use miru_agent::filesys::errors::{FileSysErr, PathExistenceErr};
use miru_agent::filesys::{dirs, files, PathExt, WriteOptions};
use miru_agent::models::Device;
use miru_agent::provisioning::check::{
    self, Report, EXIT_ERROR, EXIT_NOT_PROVISIONED, EXIT_PROVISIONED,
};

/// A layout rooted in a fresh temp directory. The auth directory is created
/// but left empty, mirroring the `fresh_layout` helper in the disk tests.
async fn fresh_layout() -> (Layout, dirs::TempDir) {
    let dir = dirs::temp("testing").unwrap();
    let layout = Layout::new(dir.to_dir());
    dirs::create_if_absent(&layout.auth().root).await.unwrap();
    (layout, dir)
}

/// A layout rooted in a temp directory with nothing under it at all — the
/// state of a machine that has never run the agent.
fn empty_layout() -> (Layout, dirs::TempDir) {
    let dir = dirs::temp("testing").unwrap();
    let layout = Layout::new(dir.to_dir());
    (layout, dir)
}

pub mod reports {
    use super::*;

    #[tokio::test]
    async fn fresh_install_is_not_provisioned() {
        let (layout, _tmp) = empty_layout();

        let report = check::check(&layout);

        assert!(matches!(report, Report::NotProvisioned));
        assert_eq!(EXIT_NOT_PROVISIONED, report.exit_code());
        assert_eq!(Some("not provisioned"), report.stdout_line());
        assert!(report.stderr_line().is_none());
    }

    #[tokio::test]
    async fn both_keys_present_is_provisioned() {
        let (layout, _tmp) = fresh_layout().await;
        let auth = layout.auth();
        files::seed(&auth.private_key(), "private").await;
        files::seed(&auth.public_key(), "public").await;

        let report = check::check(&layout);

        assert!(matches!(report, Report::Provisioned));
        assert_eq!(EXIT_PROVISIONED, report.exit_code());
        assert_eq!(Some("provisioned"), report.stdout_line());
        assert!(report.stderr_line().is_none());
    }

    #[tokio::test]
    async fn private_key_only_is_not_provisioned() {
        let (layout, _tmp) = fresh_layout().await;
        files::seed(&layout.auth().private_key(), "private").await;

        let report = check::check(&layout);

        assert!(matches!(report, Report::NotProvisioned));
        assert_eq!(EXIT_NOT_PROVISIONED, report.exit_code());
    }

    #[tokio::test]
    async fn public_key_only_is_not_provisioned() {
        let (layout, _tmp) = fresh_layout().await;
        files::seed(&layout.auth().public_key(), "public").await;

        let report = check::check(&layout);

        assert!(matches!(report, Report::NotProvisioned));
        assert_eq!(EXIT_NOT_PROVISIONED, report.exit_code());
    }

    #[tokio::test]
    async fn device_file_without_keys_is_not_provisioned() {
        let (layout, _tmp) = fresh_layout().await;
        let device = Device {
            id: "dvc_partial".to_string(),
            ..Device::default()
        };
        files::write_json(&layout.device(), &device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let report = check::check(&layout);

        assert!(matches!(report, Report::NotProvisioned));
        assert_eq!(EXIT_NOT_PROVISIONED, report.exit_code());
    }

    /// Pins the error-case contract without relying on filesystem
    /// permissions, which root bypasses.
    #[test]
    fn undeterminable_reports_error_on_stderr_only() {
        let report = Report::Undeterminable(DiskErr::FileSysErr(FileSysErr::PathExistenceErr(
            PathExistenceErr {
                path: PathBuf::from("/var/lib/miru/auth/private_key.pem"),
                source: Box::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                trace: Box::new(Trace {
                    file: file!(),
                    line: line!(),
                    backtrace: std::backtrace::Backtrace::disabled(),
                }),
            },
        )));

        assert_eq!(EXIT_ERROR, report.exit_code());
        assert!(
            report.stdout_line().is_none(),
            "stdout must stay clean in the error case"
        );
        let stderr = report
            .stderr_line()
            .expect("error case must explain itself");
        assert!(stderr.starts_with("miru-agent: "), "got {stderr}");
    }

    #[tokio::test]
    async fn unreadable_auth_dir_is_undeterminable() {
        // root ignores permission bits, so the error cannot be provoked
        if nix::unistd::geteuid().is_root() {
            eprintln!("skipping: running as root, permission bits are bypassed");
            return;
        }

        let (layout, _tmp) = fresh_layout().await;
        let auth = layout.auth();
        files::seed(&auth.private_key(), "private").await;
        files::seed(&auth.public_key(), "public").await;

        dirs::set_permissions(&auth.root, std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();
        let report = check::check(&layout);
        // restore before asserting so a failure cannot leak an unreadable dir
        dirs::set_permissions(&auth.root, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        assert!(
            matches!(report, Report::Undeterminable(_)),
            "expected Undeterminable, got {report:?}"
        );
        assert_eq!(EXIT_ERROR, report.exit_code());
        assert!(
            report.stdout_line().is_none(),
            "stdout must stay clean in the error case"
        );
        let stderr = report
            .stderr_line()
            .expect("error case must explain itself");
        assert!(!stderr.is_empty());
    }
}

pub mod read_only {
    use super::*;

    #[tokio::test]
    async fn creates_no_directories() {
        let (layout, _tmp) = empty_layout();
        assert!(!layout.root().exists(), "precondition: root must not exist");

        let report = check::check(&layout);

        assert!(matches!(report, Report::NotProvisioned));
        assert!(
            !layout.root().exists(),
            "check must not create the layout root"
        );
        assert!(
            !layout.temp_dir().exists(),
            "check must not create a temp directory"
        );
    }

    #[tokio::test]
    async fn leaves_provisioned_layout_untouched() {
        let (layout, _tmp) = fresh_layout().await;
        let auth = layout.auth();
        files::seed(&auth.private_key(), "private").await;
        files::seed(&auth.public_key(), "public").await;

        let root = layout.root();
        let subdirs_before = paths(dirs::subdirs(&root).await.unwrap().iter());
        let files_before = paths(dirs::files(&root).await.unwrap().iter());

        let report = check::check(&layout);
        assert!(matches!(report, Report::Provisioned));

        assert_eq!(
            subdirs_before,
            paths(dirs::subdirs(&root).await.unwrap().iter())
        );
        assert_eq!(
            files_before,
            paths(dirs::files(&root).await.unwrap().iter())
        );
    }

    fn paths<'a, T: PathExt + 'a>(entries: impl Iterator<Item = &'a T>) -> Vec<PathBuf> {
        let mut paths: Vec<_> = entries.map(|entry| entry.path().clone()).collect();
        paths.sort();
        paths
    }
}
