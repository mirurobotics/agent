// internal crates
use miru_agent::filesys::PathExt;
use miru_agent::installer::init_installer_logging;

#[tokio::test]
async fn creates_temp_log_dir_and_returns_guard() {
    let (guard, tmp_dir) = init_installer_logging()
        .await
        .expect("init_installer_logging should succeed in a writable env");

    assert!(
        tmp_dir.path().exists(),
        "temp dir should exist after init_installer_logging"
    );
    assert!(
        tmp_dir
            .path()
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.starts_with("miru-agent-installer-logs"))
            .unwrap_or(false),
        "temp dir name should match the installer-log prefix"
    );

    // Drop the guard explicitly so the worker thread cleans up before the
    // tmp_dir is awaited for deletion below.
    drop(guard);
    tmp_dir.delete().await.unwrap();
}
