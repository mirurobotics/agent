// Dedicated integration-test binary so the global-subscriber install in this
// test cannot collide with subscribers installed by other integration tests.

// internal crates
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogLevel, Options};

#[tokio::test]
async fn test_init_file_only() {
    let dir = Dir::create_temp_dir("miru_test_logs_file").await.unwrap();
    let options = Options {
        stdout: false,
        log_level: LogLevel::Warn,
        log_dir: dir.path().clone(),
    };
    let guard = logs::init(options).expect("init should succeed");
    drop(guard);
}
