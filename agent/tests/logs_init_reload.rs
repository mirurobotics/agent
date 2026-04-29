// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.
// This binary clears RUST_LOG so init's "not env-locked" branch is taken,
// allowing us to exercise LoggingGuard::reload_level's happy path.

// internal crates
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogLevel, Options};

#[tokio::test]
async fn test_reload_level_succeeds_when_not_locked() {
    // SAFETY: this is the only test in this binary, single-threaded entry,
    // and the env mutation precedes any thread spawn that would observe it.
    unsafe {
        std::env::remove_var("RUST_LOG");
    }

    let dir = Dir::create_temp_dir("miru_test_logs_reload").await.unwrap();
    let options = Options {
        stdout: false,
        log_level: LogLevel::Info,
        log_dir: dir.path().clone(),
    };
    let guard = logs::init(options).expect("init should succeed");
    assert!(
        !guard.env_filter_locked(),
        "RUST_LOG was cleared, env filter should not be locked"
    );

    // Happy path: reload_level should succeed and not error.
    guard
        .reload_level(LogLevel::Debug)
        .expect("reload_level should succeed when not locked");
    guard
        .reload_level(LogLevel::Warn)
        .expect("reload_level should succeed for a second reload as well");

    drop(guard);
}
