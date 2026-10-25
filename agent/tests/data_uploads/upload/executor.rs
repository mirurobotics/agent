// standard crates
use std::sync::Arc;

// internal crates
use crate::mocks::{
    http_client::{Call, MockClient},
    object_transfer::MockObjectTransfer,
    stub_token_manager::StubTokenManager,
    upload_executor::{MockStep, MockUploadExecutor},
};
use crate::test_utils::upload::{
    idempotency_key, job_on_disk, make_job, pending_response, response_with_status, token_manager,
};
use backend_api::models::{
    CreateUploadRequest, MultipartUpload, UpdateUploadRequest, UploadSource, UploadStatus,
    UploadWithCredentials,
};
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::data_uploads::upload::executor::new_upl_request;
use miru_agent::data_uploads::upload::{Job, LiveExecutor, UploadErr, UploadExecutor};
use miru_agent::errors::Error;
use miru_agent::filesys::File;
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params;

// external crates
use chrono::{FixedOffset, Local, TimeDelta, TimeZone, Utc};

// ================================ HELPERS ======================================== //

fn uploaded_response() -> UploadWithCredentials {
    response_with_status(UploadStatus::UPLOAD_STATUS_UPLOADED)
}

/// [`pending_response`] with the multipart upload `id` recorded on it.
fn recorded_response(id: &str) -> UploadWithCredentials {
    let mut resp = pending_response();
    resp.upload.multipart_upload = Some(Box::new(multipart(id)));
    resp
}

fn multipart(id: &str) -> MultipartUpload {
    MultipartUpload {
        id: id.to_string(),
        part_size: 8 * 1024 * 1024,
    }
}

fn non_network_http_err() -> HTTPErr {
    HTTPErr::MockErr(HttpMockErr {
        is_network_conn_err: false,
    })
}

fn request_failed_err(status: u16) -> HTTPErr {
    HTTPErr::RequestFailed(RequestFailed {
        request: Params::get("http://test/uploads").meta().unwrap(),
        status: reqwest::StatusCode::from_u16(status).unwrap(),
        error: None,
        trace: miru_agent::trace!(),
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
    let (_file, job) = job_on_disk("a.log", b"hello world").await;
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor.upload(&job).await.unwrap();

    // Exactly one transfer, of the job's file to the created upload, in one
    // part.
    assert_eq!(
        transfer.recorded_calls(),
        vec![(pending_response(), job.file.clone(), None)]
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
    assert_eq!(sent, new_upl_request(&job, &Local));
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
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    executor.upload(&job).await.unwrap();

    assert_eq!(transfer.recorded_calls().len(), 1);
    assert_eq!(client.call_count(Call::ConfirmUpload), 1);
}

#[tokio::test]
async fn already_confirmed_upload_skips_the_transfer() {
    let client = Arc::new(MockClient::default());
    // an earlier attempt confirmed it, so create returns it uploaded
    client.set_create_upload(|| Ok(uploaded_response()));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    // the file is gone, which doesn't matter for an uploaded upload
    executor.upload(&make_job("a.log")).await.unwrap();

    assert_eq!(transfer.recorded_calls(), vec![]);
    assert_eq!(client.call_count(Call::ConfirmUpload), 0);
}

#[tokio::test]
async fn recorded_multipart_upload_is_resumed() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(recorded_response("mpu_1")));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    executor.upload(&job).await.unwrap();

    assert_eq!(
        transfer.recorded_calls(),
        vec![(
            recorded_response("mpu_1"),
            job.file.clone(),
            Some(multipart("mpu_1"))
        )]
    );
    assert_eq!(client.call_count(Call::UpdateUpload), 0);
    assert_eq!(client.call_count(Call::ConfirmUpload), 1);
}

#[tokio::test]
async fn new_multipart_upload_is_recorded_before_the_transfer() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(recorded_response("mpu_gone")));
    let transfer = MockObjectTransfer::new();
    transfer.start_multipart_upload(multipart("mpu_2"));
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    executor.upload(&job).await.unwrap();

    let requests = client.requests();
    assert_eq!(
        requests.iter().map(|r| r.call.clone()).collect::<Vec<_>>(),
        vec![Call::CreateUpload, Call::UpdateUpload, Call::ConfirmUpload]
    );
    assert_eq!(requests[1].path, "/uploads/upl_1");
    let sent: UpdateUploadRequest =
        serde_json::from_str(requests[1].body.as_deref().unwrap()).unwrap();
    assert_eq!(sent, UpdateUploadRequest::new(multipart("mpu_2")));
    assert_eq!(requests[1].token.as_deref(), Some("test-token"));
    assert_eq!(transfer.recorded_calls()[0].2, Some(multipart("mpu_2")));
}

#[tokio::test]
async fn record_failure_skips_the_transfer() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    client.set_update_upload(|| Err(non_network_http_err()));
    let transfer = MockObjectTransfer::new();
    transfer.start_multipart_upload(multipart("mpu_1"));
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    let err = executor.upload(&job).await.unwrap_err();

    assert!(!err.is_terminal());
    assert_eq!(transfer.recorded_calls(), vec![]);
    assert_eq!(client.call_count(Call::ConfirmUpload), 0);
}

// ===== changed files =====

/// Uploads `job`, whose file changed after the scan, checks that nothing was
/// sent, and returns the error.
async fn upload_changed(job: Job) -> UploadErr {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(recorded_response("mpu_1")));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let err = executor.upload(&job).await.unwrap_err();

    assert_eq!(transfer.recorded_calls(), vec![]);
    assert_eq!(client.call_count(Call::ConfirmUpload), 0);
    err
}

#[tokio::test]
async fn file_of_another_size_is_terminal() {
    let (_file, job) = job_on_disk("a.log", b"hello world").await;
    let job = Job {
        size: job.size + 1,
        ..job
    };

    assert!(upload_changed(job).await.is_terminal());
}

#[tokio::test]
async fn file_with_new_bytes_is_terminal() {
    let (_file, job) = job_on_disk("a.log", b"hello world").await;
    let job = Job {
        mtime: job.mtime + TimeDelta::seconds(1),
        digest: "sha256:other".to_string(),
        ..job
    };

    assert!(upload_changed(job).await.is_terminal());
}

// The scanner doesn't report a file again when only its mtime moved, so the
// upload goes ahead.
#[tokio::test]
async fn file_with_only_a_new_mtime_uploads() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let (_file, job) = job_on_disk("a.log", b"hello world").await;
    let job = Job {
        mtime: job.mtime + TimeDelta::seconds(1),
        ..job
    };

    executor.upload(&job).await.unwrap();

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
async fn token_network_failure_is_network_classified() {
    let client = Arc::new(MockClient::default());
    let token_mngr = Arc::new(StubTokenManager::err(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: true,
        trace: miru_agent::trace!(),
    })));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_mngr, transfer.clone());

    let err = executor.upload(&make_job("a.log")).await.unwrap_err();

    // the authn network classification survives the executor's type erasure
    assert!(err.is_network_conn_err());
    assert!(!err.is_terminal());
    assert!(client.requests().is_empty());
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
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    let result = executor.upload(&job).await;

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
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    let result = executor.upload(&job).await;

    assert!(
        matches!(result, Err(UploadErr::ExecutorErr(_))),
        "expected ExecutorErr, got: {result:?}"
    );
    assert_eq!(transfer.recorded_calls().len(), 1);
}

// ===== terminal classification =====

#[tokio::test]
async fn create_4xx_failure_is_terminal() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Err(request_failed_err(404)));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let err = executor.upload(&make_job("a.log")).await.unwrap_err();

    assert!(err.is_terminal());
}

#[tokio::test]
async fn create_transient_statuses_are_not_terminal() {
    for status in [401u16, 408, 429, 500] {
        let client = Arc::new(MockClient::default());
        client.set_create_upload(move || Err(request_failed_err(status)));
        let transfer = MockObjectTransfer::new();
        let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

        let err = executor.upload(&make_job("a.log")).await.unwrap_err();

        assert!(!err.is_terminal(), "status {status}");
    }
}

#[tokio::test]
async fn confirm_4xx_failure_is_terminal() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    client.set_confirm_upload(|| Err(request_failed_err(422)));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let (_file, job) = job_on_disk("a.log", b"hello world").await;

    let err = executor.upload(&job).await.unwrap_err();

    assert!(err.is_terminal());
}

// ================================ create_request ================================= //

#[test]
fn create_request_maps_job_fields() {
    let file = File::new("/data/a.log");
    let file_path = file.to_string();
    let job = Job {
        file,
        size: 42,
        digest: "sha256:abc".to_string(),
        mtime: Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
        first_observed_at: Utc.with_ymd_and_hms(2021, 1, 1, 0, 1, 0).unwrap(),
        last_observed_at: Utc.with_ymd_and_hms(2021, 1, 1, 0, 2, 0).unwrap(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
        idempotency_key: idempotency_key("a.log"),
    };

    let expected = CreateUploadRequest {
        file_rule_id: "rule_1".to_string(),
        source: Box::new(UploadSource {
            file_path,
            mtime: "2021-01-01T00:00:00+00:00".to_string(),
            first_observed_at: "2021-01-01T00:01:00+00:00".to_string(),
            last_observed_at: "2021-01-01T00:02:00+00:00".to_string(),
        }),
        digest: "sha256:abc".to_string(),
        size: 42,
        incomplete: None,
        deployment_id: "dpl_1".to_string(),
        idempotency_key: Some(idempotency_key("a.log")),
    };
    assert_eq!(new_upl_request(&job, &Utc), expected);
}

#[test]
fn new_upl_request_sends_mtime_with_device_offset() {
    let file = File::new("/data/a.log");
    let job = Job {
        file,
        size: 42,
        digest: "sha256:abc".to_string(),
        // 01:30 UTC is 20:30 the evening before in New York (EST)
        mtime: Utc.with_ymd_and_hms(2026, 1, 15, 1, 30, 0).unwrap(),
        first_observed_at: Utc.with_ymd_and_hms(2026, 1, 15, 1, 31, 0).unwrap(),
        last_observed_at: Utc.with_ymd_and_hms(2026, 1, 15, 1, 32, 0).unwrap(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
        idempotency_key: idempotency_key("a.log"),
    };
    let est = FixedOffset::west_opt(5 * 3600).unwrap();

    let req = new_upl_request(&job, &est);

    assert_eq!(req.source.mtime, "2026-01-14T20:30:00-05:00");
    assert_eq!(req.source.first_observed_at, "2026-01-15T01:31:00+00:00");
}
