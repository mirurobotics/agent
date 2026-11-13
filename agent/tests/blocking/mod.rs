// internal crates
use miru_agent::blocking;

#[tokio::test(flavor = "current_thread")]
async fn run_runs_off_the_runtime_thread() {
    let runtime_thread = std::thread::current().id();
    let op_thread = blocking::run(|| std::thread::current().id()).await;
    assert_ne!(op_thread, runtime_thread);
}

#[tokio::test]
async fn run_returns_the_result() {
    assert_eq!(blocking::run(|| 2 + 2).await, 4);
}

#[tokio::test]
#[should_panic(expected = "boom")]
async fn run_reraises_a_panic() {
    blocking::run(|| -> u8 { panic!("boom") }).await;
}
