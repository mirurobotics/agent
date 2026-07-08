// internal crates
use crate::mocks::upload_executor::{MockStep, MockUploadExecutor};
use miru_agent::filesys::File;
use miru_agent::upload::{LogExecutor, UploadErr, UploadExecutor, UploadJob};

// external crates
use chrono::Utc;

fn make_job(name: &str) -> UploadJob {
    UploadJob {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: Utc::now(),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
    }
}

#[tokio::test]
async fn log_executor_returns_ok() {
    let job = make_job("a.log");
    LogExecutor.upload(&job).await.unwrap();
}

#[tokio::test]
async fn mock_follows_script() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::Ok);
    mock.push_step(MockStep::Err);
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    mock.upload(&job_a).await.unwrap();
    let result = mock.upload(&job_b).await;
    assert!(
        matches!(result, Err(UploadErr::ExecutorErr(_))),
        "expected ExecutorErr, got: {result:?}"
    );
    assert_eq!(mock.recorded_calls(), vec![job_a, job_b]);
}

#[tokio::test]
async fn mock_empty_script_defaults_to_ok() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let job = make_job("a.log");

    mock.upload(&job).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job]);
}
