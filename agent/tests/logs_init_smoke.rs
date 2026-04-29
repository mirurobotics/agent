// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.
// Covers three things in a single process (because `set_global_default` is
// one-shot per process):
//   1. The successful global install through `logs::init`.
//   2. `LoggingGuard::reload_level` happy path on a non-locked filter
//      (we clear `RUST_LOG` before init so the not-locked branch is taken).
//   3. The `SetGlobalDefault` error path: a second init in the same process
//      must fail with `LogsErr::SetGlobalDefault`.

// internal crates
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogLevel, LogsErr, Options};

// external crates
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_init_installs_globally_and_rejects_double_install() {
    // SAFETY: this is the only test in this binary, single-threaded entry,
    // and the env mutation precedes any thread spawn that would observe it.
    // Clearing `RUST_LOG` lets us exercise `LoggingGuard::reload_level`'s
    // happy path (otherwise `scripts/test.sh`'s `RUST_LOG=off` would lock the
    // filter and `reload_level` would no-op).
    unsafe {
        std::env::remove_var("RUST_LOG");
    }

    let dir = Dir::create_temp_dir("miru_test_logs_smoke").await.unwrap();

    let options = Options {
        stdout: false,
        log_dir: dir.path().clone(),
        ..Default::default()
    };
    let guard = logs::init(options).expect("first init should succeed");
    assert!(
        !guard.env_filter_locked(),
        "RUST_LOG was cleared, env filter should not be locked"
    );

    // Happy path: reload_level should succeed on a non-locked filter.
    guard
        .reload_level(LogLevel::Debug)
        .expect("reload_level should succeed when not locked");
    guard
        .reload_level(LogLevel::Warn)
        .expect("reload_level should succeed for a second reload as well");

    let options_second = Options {
        stdout: false,
        log_dir: dir.path().clone(),
        ..Default::default()
    };
    match logs::init(options_second) {
        Err(LogsErr::SetGlobalDefault(_)) => {}
        Err(other) => panic!("expected LogsErr::SetGlobalDefault on double init, got: {other:?}"),
        Ok(_) => panic!("expected double init to fail, but it succeeded"),
    }

    drop(guard);
}
