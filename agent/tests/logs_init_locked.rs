// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.

// internal crates
use miru_agent::logs::{self, LogLevel, Options};

#[tokio::test]
async fn test_reload_level_no_op_when_env_filter_locked() {
    // SAFETY: this is the only test in this binary, and environment setup
    // precedes subscriber initialization and any work that may observe it.
    unsafe {
        std::env::set_var("RUST_LOG", "off");
    }
    // The contract here is that env_filter_locked() reports the lock;
    // emission semantics are exercised in tests/logs/mod.rs.
    let dir = tempfile::Builder::new()
        .prefix("miru_test_logs_locked")
        .tempdir()
        .unwrap();
    let options = Options {
        stdout: false,
        log_level: LogLevel::Info,
        log_dir: dir.path().to_path_buf(),
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
