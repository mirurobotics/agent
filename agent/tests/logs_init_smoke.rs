// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.
// Also covers the SetGlobalDefault error path: a second init in the same
// process must fail with LogsErr::SetGlobalDefault.

// internal crates
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogsErr, Options};

// external crates
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_init_installs_globally_and_rejects_double_install() {
    let dir = Dir::create_temp_dir("miru_test_logs_smoke").await.unwrap();

    let options = Options {
        stdout: false,
        log_dir: dir.path().clone(),
        ..Default::default()
    };
    let guard = logs::init(options).expect("first init should succeed");

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
