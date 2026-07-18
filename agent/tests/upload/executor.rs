// standard crates
use std::collections::HashMap;
use std::sync::Arc;

// internal crates
use crate::mocks::{
    http_client::{Call, MockClient},
    object_transfer::MockObjectTransfer,
    stub_token_manager::StubTokenManager,
    token_manager::MockTokenManager,
    upload_executor::{MockStep, MockUploadExecutor},
};
use backend_api::models::{
    CreateUploadRequest, Upload, UploadCredentials, UploadDestination, UploadSource, UploadStatus,
    UploadWithCredentials,
};
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::{AuthnErr, Token};
use miru_agent::filesys::{files, File, PathExt, WriteOptions};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr};
use miru_agent::models::DeletePolicy;
use miru_agent::upload::executor::new_upl_request;
use miru_agent::upload::{Job, LiveExecutor, SdkTransfer, UploadErr, UploadExecutor};

// external crates
use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_types::body::SdkBody;
use chrono::{TimeZone, Utc};
use serde_json::json;

// ================================ HELPERS ======================================== //

fn make_job(name: &str) -> Job {
    let now = Utc::now();
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: now,
        first_observed_at: now,
        last_observed_at: now,
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        delete_policy: DeletePolicy::Never,
    }
}

fn destination() -> UploadDestination {
    UploadDestination {
        bucket_id: "bkt_1".to_string(),
        bucket_name: "my-bucket".to_string(),
        object_key: "logs/a.log".to_string(),
    }
}

fn s3_credentials() -> UploadCredentials {
    serde_json::from_value(json!({
        "scheme": "s3",
        "s3_credentials": {
            "scheme": "s3",
            "access_key_id": "AKIA_TEST",
            "secret_access_key": "secret",
            "session_token": "session",
            "region": "us-east-1",
            "expires_at": "2021-01-01T01:00:00Z"
        },
        "gcs_credentials": null,
        "expires_at": "2021-01-01T01:00:00Z"
    }))
    .unwrap()
}

fn response_metadata() -> HashMap<String, String> {
    HashMap::from([("device_id".to_string(), "dvc_1".to_string())])
}

/// A `POST /uploads` response for upload `upl_1` (destination
/// `my-bucket`/`logs/a.log`, s3 credentials) in the given `status`.
fn response_with_status(status: UploadStatus) -> UploadWithCredentials {
    UploadWithCredentials {
        upload: Box::new(Upload {
            id: "upl_1".to_string(),
            status,
            destination: Box::new(destination()),
            ..Default::default()
        }),
        credentials: Box::new(s3_credentials()),
        metadata: response_metadata(),
    }
}

fn pending_response() -> UploadWithCredentials {
    response_with_status(UploadStatus::UPLOAD_STATUS_PENDING)
}

fn uploaded_response() -> UploadWithCredentials {
    response_with_status(UploadStatus::UPLOAD_STATUS_UPLOADED)
}

fn token_manager() -> Arc<MockTokenManager> {
    Arc::new(MockTokenManager::new(Token {
        token: "test-token".to_string(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    }))
}

fn non_network_http_err() -> HTTPErr {
    HTTPErr::MockErr(HttpMockErr {
        is_network_conn_err: false,
    })
}

// ============================ MockUploadExecutor ================================= //

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

// ============================== LiveExecutor =================================== //

#[tokio::test]
async fn happy_path_creates_transfers_confirms() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let transfer = MockObjectTransfer::new();
    let job = make_job("a.log");
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor.upload(&job).await.unwrap();

    // Exactly one transfer, carrying the create response's credentials,
    // destination, and metadata plus the job's file.
    assert_eq!(
        transfer.recorded_calls(),
        vec![(
            s3_credentials(),
            destination(),
            job.file.clone(),
            response_metadata()
        )]
    );
    // The captured HTTP exchange: create (with the job's payload and the token
    // manager's bearer string) then confirm for the created upload's id.
    let requests = client.requests();
    assert_eq!(
        requests.iter().map(|r| r.call.clone()).collect::<Vec<_>>(),
        vec![Call::CreateUpload, Call::ConfirmUpload]
    );
    let sent: CreateUploadRequest =
        serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(sent, new_upl_request(&job));
    assert_eq!(requests[0].token.as_deref(), Some("test-token"));
    assert_eq!(requests[1].path, "/uploads/upl_1/confirm");
    assert_eq!(requests[1].token.as_deref(), Some("test-token"));
}

#[tokio::test]
async fn unknown_status_proceeds_like_pending() {
    let client = Arc::new(MockClient::default());
    // The serde catch-all a newer backend status deserializes into: proceed
    // with transfer and confirm rather than stranding the file.
    client.set_create_upload(|| Ok(response_with_status(UploadStatus::UploadStatusUnknown)));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor.upload(&make_job("a.log")).await.unwrap();

    assert_eq!(transfer.recorded_calls().len(), 1);
    assert_eq!(client.call_count(Call::ConfirmUpload), 1);
}

#[tokio::test]
async fn token_failure_maps_to_executor_err() {
    let client = Arc::new(MockClient::default());
    let token_mngr = Arc::new(StubTokenManager::err(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    })));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_mngr, transfer.clone());

    let result = executor.upload(&make_job("a.log")).await;

    assert!(
        matches!(result, Err(UploadErr::ExecutorErr(_))),
        "expected ExecutorErr, got: {result:?}"
    );
    assert!(client.requests().is_empty());
    assert_eq!(transfer.recorded_calls(), vec![]);
}

#[tokio::test]
async fn create_failure_maps_to_executor_err() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Err(non_network_http_err()));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let result = executor.upload(&make_job("a.log")).await;

    assert!(
        matches!(result, Err(UploadErr::ExecutorErr(_))),
        "expected ExecutorErr, got: {result:?}"
    );
    assert_eq!(transfer.recorded_calls(), vec![]);
}

#[tokio::test]
async fn transfer_failure_propagates_and_skips_confirm() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    transfer.push_err();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let result = executor.upload(&make_job("a.log")).await;

    assert!(
        matches!(result, Err(UploadErr::ExecutorErr(_))),
        "expected ExecutorErr, got: {result:?}"
    );
    assert_eq!(client.call_count(Call::ConfirmUpload), 0);
}

#[tokio::test]
async fn confirm_failure_maps_to_executor_err() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    client.set_confirm_upload(|| Err(non_network_http_err()));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let result = executor.upload(&make_job("a.log")).await;

    assert!(
        matches!(result, Err(UploadErr::ExecutorErr(_))),
        "expected ExecutorErr, got: {result:?}"
    );
    assert_eq!(transfer.recorded_calls().len(), 1);
}

/// The path-style PutObject URI the S3 SDK emits for [`destination`]'s bucket
/// and key with the vended `us-east-1` region (see `tests/upload/transfer.rs`).
const S3_PUT_URI: &str = "https://s3.us-east-1.amazonaws.com/my-bucket/logs/a.log?x-id=PutObject";

#[tokio::test]
async fn end_to_end_with_sdk_transfer_over_replayed_s3() {
    // Same composition as production — LiveExecutor over the real
    // SdkTransfer — with the S3 exchange replayed offline.
    let replay = StaticReplayClient::new(vec![ReplayEvent::new(
        http::Request::builder()
            .method("PUT")
            .uri(S3_PUT_URI)
            .body(SdkBody::empty())
            .unwrap(),
        http::Response::builder()
            .status(200)
            .header("content-length", "0")
            .body(SdkBody::empty())
            .unwrap(),
    )]);
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let src = files::temp("upload-executor-test").unwrap();
    files::write_bytes(
        src.file(),
        b"hello world",
        WriteOptions::OVERWRITE_NONATOMIC,
    )
    .await
    .unwrap();
    let mut job = make_job("a.log");
    job.file = src.file().clone();
    let executor = LiveExecutor::new(
        client.clone(),
        token_manager(),
        SdkTransfer::with_s3_http_client(replay.clone()),
    );

    executor.upload(&job).await.unwrap();

    // The replay client saw exactly one PUT at the expected URI — the create
    // response's credentials and destination drove the production transfer —
    // stamped with the create response's metadata map.
    let requests = replay.actual_requests().collect::<Vec<_>>();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), "PUT");
    assert_eq!(requests[0].uri().to_string(), S3_PUT_URI);
    assert_eq!(
        requests[0].headers().get("x-amz-meta-device_id"),
        Some("dvc_1")
    );
    assert_eq!(client.call_count(Call::ConfirmUpload), 1);
}

// ============================== delete policy =================================== //

async fn delete_policy_setup(
    client: Arc<MockClient>,
) -> (
    files::TempFile,
    LiveExecutor<MockClient, MockTokenManager, MockObjectTransfer>,
) {
    client.set_create_upload(|| Ok(pending_response()));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let src = files::temp("upload-delete-policy-test").unwrap();
    files::write_bytes(
        src.file(),
        b"hello world",
        WriteOptions::OVERWRITE_NONATOMIC,
    )
    .await
    .unwrap();
    let executor = LiveExecutor::new(client, token_manager(), MockObjectTransfer::new());
    (src, executor)
}

#[tokio::test]
async fn after_upload_deletes_source_after_confirm() {
    let client = Arc::new(MockClient::default());
    let (src, executor) = delete_policy_setup(client).await;
    let mut job = make_job("a.log");
    job.file = src.file().clone();
    job.delete_policy = DeletePolicy::AfterUpload;
    let path = job.file.path().clone();
    assert!(path.exists());

    executor.upload(&job).await.unwrap();

    // AfterUpload deletes the confirmed source file.
    assert!(!path.exists());
}

#[tokio::test]
async fn never_leaves_source_in_place() {
    let client = Arc::new(MockClient::default());
    let (src, executor) = delete_policy_setup(client).await;
    let mut job = make_job("a.log");
    job.file = src.file().clone();
    job.delete_policy = DeletePolicy::Never;
    let path = job.file.path().clone();

    executor.upload(&job).await.unwrap();

    // Never leaves the source file untouched.
    assert!(path.exists());
}

#[tokio::test]
async fn delete_failure_after_confirm_still_succeeds() {
    let client = Arc::new(MockClient::default());
    let (src, executor) = delete_policy_setup(client.clone()).await;
    let mut job = make_job("a.log");
    // Point the source at a child of the written temp file. Its parent is a
    // regular file, not a directory, so `remove_file` fails with a non-NotFound
    // kind. `files::delete` returns Err; the executor logs and swallows it (any
    // delete error other than NotFound is swallowed), so the upload — already
    // durably confirmed — still reports Ok.
    job.file = File::new(src.file().path().join("child"));
    job.delete_policy = DeletePolicy::AfterUpload;

    // Observable outcome: upload is Ok and confirm ran exactly once, proving the
    // swallowed delete error did not re-drive an already-durable upload.
    executor.upload(&job).await.unwrap();
    assert_eq!(client.call_count(Call::ConfirmUpload), 1);
}

#[tokio::test]
async fn missing_source_at_delete_is_success() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let executor = LiveExecutor::new(client, token_manager(), MockObjectTransfer::new());
    let mut job = make_job("never-existed.log");
    // A path that never existed (distinct from the present-then-deleted case):
    // `files::delete` maps NotFound to Ok, so upload succeeds.
    job.file = File::new("/data/does-not-exist/never-existed.log");
    job.delete_policy = DeletePolicy::AfterUpload;

    executor.upload(&job).await.unwrap();
}

// ================================ create_request ================================= //

#[test]
fn create_request_maps_job_fields() {
    let job = Job {
        file: File::new("/data/a.log"),
        size: 42,
        digest: "sha256:abc".to_string(),
        mtime: Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
        first_observed_at: Utc.with_ymd_and_hms(2021, 1, 1, 0, 1, 0).unwrap(),
        last_observed_at: Utc.with_ymd_and_hms(2021, 1, 1, 0, 2, 0).unwrap(),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        delete_policy: DeletePolicy::Never,
    };

    let expected = CreateUploadRequest {
        upload_rule_id: "rule_1".to_string(),
        source: Box::new(UploadSource {
            file_path: "/data/a.log".to_string(),
            mtime: "2021-01-01T00:00:00+00:00".to_string(),
            first_observed_at: "2021-01-01T00:01:00+00:00".to_string(),
            last_observed_at: "2021-01-01T00:02:00+00:00".to_string(),
        }),
        digest: "sha256:abc".to_string(),
        size: 42,
        incomplete: None,
        deployment_id: "dpl_1".to_string(),
    };
    assert_eq!(new_upl_request(&job), expected);
}
