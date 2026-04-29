// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.

// internal crates
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogLevel, Options};

#[tokio::test]
async fn test_reload_level_no_op_when_env_filter_locked() {
    // scripts/test.sh sets RUST_LOG=off process-wide, so init's env-filter
    // branch is taken and reload_level is a no-op. The contract we assert
    // here is that env_filter_locked() reports the lock; emission semantics
    // are exercised by test_reload_level_changes_filter in tests/logs/mod.rs.
    let dir = Dir::create_temp_dir("miru_test_logs_locked").await.unwrap();
    let options = Options {
        stdout: false,
        log_level: LogLevel::Info,
        log_dir: dir.path().clone(),
    };
    let guard = logs::init(options).expect("init should succeed");
    assert!(
        guard.env_filter_locked(),
        "RUST_LOG=off should lock the env filter"
    );
    // reload_level should be a no-op (Ok(()) without touching the handle).
    guard
        .reload_level(LogLevel::Trace)
        .expect("reload_level should be a no-op when locked");
    drop(guard);
}
