// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.

// internal crates
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogsErr, Options};

// external crates
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_init_returns_error_on_double_install() {
    let dir = Dir::create_temp_dir("miru_test_logs_double").await.unwrap();

    let options = Options {
        stdout: false,
        log_dir: dir.path().clone(),
        ..Default::default()
    };

    let _first = logs::init(options).expect("first init should succeed");

    let options_second = Options {
        stdout: false,
        log_dir: dir.path().clone(),
        ..Default::default()
    };
    let result = logs::init(options_second);
    match result {
        Err(LogsErr::SetGlobalDefault(_)) => {}
        Err(other) => panic!("expected LogsErr::SetGlobalDefault on double init, got: {other:?}"),
        Ok(_) => panic!("expected double init to fail, but it succeeded"),
    }
}
