// standard crates
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

// internal crates
use miru_agent::disk::{DiskErr, Layout};
use miru_agent::errors::Trace;
use miru_agent::filesys::errors::{FileSysErr, PathExistenceErr};
use miru_agent::filesys::{dirs, files};
use miru_agent::provisioning::check::{
    self, Report, EXIT_ERROR, EXIT_NOT_PROVISIONED, EXIT_PROVISIONED,
};

async fn fresh_layout() -> (Layout, dirs::TempDir) {
    let dir = dirs::temp("testing").unwrap();
    let layout = Layout::new(dir.to_dir());
    dirs::create_if_absent(&layout.auth().root).await.unwrap();
    (layout, dir)
}

fn empty_layout() -> (Layout, dirs::TempDir) {
    let dir = dirs::temp("testing").unwrap();
    let layout = Layout::new(dir.to_dir());
    (layout, dir)
}

pub mod reports {
    use super::*;

    #[tokio::test]
    async fn not_provisioned() {
        let (layout, _tmp) = empty_layout();

        let report = check::check(&layout);

        assert!(matches!(report, Report::NotProvisioned));
        assert_eq!(EXIT_NOT_PROVISIONED, report.exit_code());
        assert_eq!(Some("not provisioned"), report.stdout_line());
        assert!(report.stderr_line().is_none());
    }

    #[tokio::test]
    async fn is_provisioned() {
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
