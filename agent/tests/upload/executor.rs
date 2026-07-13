// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use crate::mocks::{
    upload_client::{upload_with_credentials_json, MockUploadClient},
    upload_executor::{MockStep, MockUploadExecutor},
};
use backend_api::models::{UploadCredentials, UploadDestination};
use miru_agent::filesys::{File, PathExt};
use miru_agent::upload::errors::ExecutorErr;
use miru_agent::upload::{
    BrokerExecutor, Job, LogExecutor, ObjectTransfer, TokenSource, UploadErr, UploadExecutor,
};

// external crates
use chrono::Utc;

fn make_job(name: &str) -> Job {
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: Utc::now(),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
    }
}

fn boom(msg: &str) -> UploadErr {
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other(msg.to_string())),
        trace: miru_agent::trace!(),
    })
}

// ================================ LOG / MOCK ==================================== //

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

// ============================== BROKER EXECUTOR ================================= //

/// One-shot token source: `Some` yields the token, `None` fails.
struct StubTokenSource(Option<&'static str>);

impl TokenSource for StubTokenSource {
    async fn token(&self) -> Result<String, UploadErr> {
        match self.0 {
            Some(t) => Ok(t.to_string()),
            None => Err(boom("token unavailable")),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct TransferCall {
    scheme_is_s3: bool,
    bucket_name: String,
    object_key: String,
    file: String,
}

/// Records each transfer and returns a fixed outcome, so executor orchestration
/// is testable without a live object store.
struct FakeTransfer {
    fail: bool,
    calls: Arc<Mutex<Vec<TransferCall>>>,
}

impl ObjectTransfer for FakeTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
    ) -> Result<(), UploadErr> {
        use backend_api::models::upload_credentials::Scheme;
        self.calls.lock().unwrap().push(TransferCall {
            scheme_is_s3: matches!(credentials.scheme, Scheme::UPLOAD_SCHEME_S3),
            bucket_name: destination.bucket_name.clone(),
            object_key: destination.object_key.clone(),
            file: file.path().to_string_lossy().into_owned(),
        });
        if self.fail {
            Err(boom("transfer failed"))
        } else {
            Ok(())
        }
    }
}

/// An executor wired over a mock client, canned token, and fake transfer, plus
/// the client and call log for assertions.
type Wired = (
    BrokerExecutor<Arc<MockUploadClient>, StubTokenSource, FakeTransfer>,
    Arc<MockUploadClient>,
    Arc<Mutex<Vec<TransferCall>>>,
);

/// Wires an executor over a fresh mock client, a canned token, and a fake
/// transfer, returning all three.
fn wired(token: Option<&'static str>, fail_transfer: bool) -> Wired {
    let client = Arc::new(MockUploadClient::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let transfer = FakeTransfer {
        fail: fail_transfer,
        calls: calls.clone(),
    };
    let exec = BrokerExecutor::new(client.clone(), StubTokenSource(token), transfer);
    (exec, client, calls)
}

#[tokio::test]
async fn transfers_then_confirms_on_pending() {
    let (exec, client, calls) = wired(Some("tok"), false);

    exec.upload(&make_job("a.log")).await.unwrap();

    // transferred exactly once, to the physical bucket + object key from the
    // create response (not the bucket_id).
    let calls = calls.lock().unwrap();
    assert_eq!(
        *calls,
        vec![TransferCall {
            scheme_is_s3: true,
            bucket_name: "my-bucket".to_string(),
            object_key: "logs/a.log".to_string(),
            file: "/data/a.log".to_string(),
        }]
    );
    // create then confirm, both bearing the token.
    let paths = client.paths();
    assert_eq!(paths, vec!["/uploads", "/uploads/upl_1/confirm"]);
    assert!(client
        .requests()
        .iter()
        .all(|r| r.token.as_deref() == Some("tok")));
}

#[tokio::test]
async fn already_uploaded_skips_transfer_and_confirm() {
    let (exec, client, calls) = wired(Some("tok"), false);
    client.set_create(upload_with_credentials_json("uploaded", "s3").to_string());

    exec.upload(&make_job("a.log")).await.unwrap();

    assert!(
        calls.lock().unwrap().is_empty(),
        "must not transfer a deduped upload"
    );
    assert_eq!(
        client.paths(),
        vec!["/uploads"],
        "must not confirm a deduped upload"
    );
}

#[tokio::test]
async fn create_failure_is_executor_err_without_transfer() {
    let (exec, client, calls) = wired(Some("tok"), false);
    client.set_create("not json{");

    let err = exec.upload(&make_job("a.log")).await.unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(client.paths(), vec!["/uploads"]);
}

#[tokio::test]
async fn transfer_failure_skips_confirm() {
    let (exec, client, _calls) = wired(Some("tok"), true);

    let err = exec.upload(&make_job("a.log")).await.unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert_eq!(
        client.paths(),
        vec!["/uploads"],
        "must not confirm a failed transfer"
    );
}

#[tokio::test]
async fn confirm_failure_is_executor_err() {
    let (exec, client, calls) = wired(Some("tok"), false);
    client.set_confirm("not json{");

    let err = exec.upload(&make_job("a.log")).await.unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "transfer still ran before confirm failed"
    );
}

#[tokio::test]
async fn token_failure_short_circuits_before_create() {
    let (exec, client, calls) = wired(None, false);

    let err = exec.upload(&make_job("a.log")).await.unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(
        client.requests().is_empty(),
        "must not hit the backend without a token"
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn create_request_carries_job_fields() {
    let (exec, client, _calls) = wired(Some("tok"), false);

    exec.upload(&make_job("a.log")).await.unwrap();

    let reqs = client.requests();
    let create = reqs.iter().find(|r| r.path == "/uploads").unwrap();
    let body: serde_json::Value = serde_json::from_str(create.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["upload_rule_id"], "rule_1");
    assert_eq!(body["digest"], "sha256:a.log");
    assert_eq!(body["size"], 42);
    assert_eq!(body["release_id"], "rls_1");
    assert_eq!(body["deployment_id"], "dpl_1");
    assert_eq!(body["source"]["file_path"], "/data/a.log");
}
