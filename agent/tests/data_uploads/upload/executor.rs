// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// internal crates
use crate::mocks::{
    checkpointer::{NoopCheckpointer, RecordingCheckpointer},
    http_client::{Call, MockClient},
    object_transfer::MockObjectTransfer,
    stub_token_manager::StubTokenManager,
    upload_executor::{MockStep, MockUploadExecutor},
};
use crate::test_utils::upload::{
    make_job, pending_response, response_with_status, s3_credentials, token_manager, upload_ref,
};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus, UploadWithCredentials};
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::data_uploads::upload::executor::new_upl_request;
use miru_agent::data_uploads::upload::{
    Job, LiveExecutor, MultipartRef, UploadErr, UploadExecutor, UploadRef,
};
use miru_agent::errors::Error;
use miru_agent::filesys::File;
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params;

// external crates
use chrono::{FixedOffset, Local, TimeZone, Utc};

// ================================ HELPERS ======================================== //

fn uploaded_response() -> UploadWithCredentials {
    response_with_status(UploadStatus::UPLOAD_STATUS_UPLOADED)
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

    mock.upload(&job_a, &NoopCheckpointer).await.unwrap();
    let result = mock.upload(&job_b, &NoopCheckpointer).await;
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

    mock.upload(&job, &NoopCheckpointer).await.unwrap();
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

    executor.upload(&job, &NoopCheckpointer).await.unwrap();

    // Exactly one transfer, carrying the create response's credentials, its
    // upload (the physical `bucket_name`, not the Miru `bucket_id`, plus key and
    // metadata) and the job's file.
    assert_eq!(
        transfer.recorded_calls(),
        vec![(s3_credentials(), upload_ref(), job.file.clone())]
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

    executor
        .upload(&make_job("a.log"), &NoopCheckpointer)
        .await
        .unwrap();

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

    let result = executor.upload(&make_job("a.log"), &NoopCheckpointer).await;

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

    let err = executor
        .upload(&make_job("a.log"), &NoopCheckpointer)
        .await
        .unwrap_err();

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

    let result = executor.upload(&make_job("a.log"), &NoopCheckpointer).await;

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

    let result = executor.upload(&make_job("a.log"), &NoopCheckpointer).await;

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

    let result = executor.upload(&make_job("a.log"), &NoopCheckpointer).await;

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

    let err = executor
        .upload(&make_job("a.log"), &NoopCheckpointer)
        .await
        .unwrap_err();

    assert!(err.is_terminal());
}

#[tokio::test]
async fn create_transient_statuses_are_not_terminal() {
    for status in [401u16, 408, 429, 500] {
        let client = Arc::new(MockClient::default());
        client.set_create_upload(move || Err(request_failed_err(status)));
        let transfer = MockObjectTransfer::new();
        let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

        let err = executor
            .upload(&make_job("a.log"), &NoopCheckpointer)
            .await
            .unwrap_err();

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

    let err = executor
        .upload(&make_job("a.log"), &NoopCheckpointer)
        .await
        .unwrap_err();

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
        upload: None,
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
        upload: None,
    };
    let est = FixedOffset::west_opt(5 * 3600).unwrap();

    let req = new_upl_request(&job, &est);

    assert_eq!(req.source.mtime, "2026-01-14T20:30:00-05:00");
    assert_eq!(req.source.first_observed_at, "2026-01-15T01:31:00+00:00");
}

// =========================== Reusing the upload ================================= //

/// A job an earlier attempt already saved `upload` on.
fn job_with(upload: UploadRef) -> Job {
    let mut job = make_job("a.log");
    job.upload = Some(upload);
    job
}

/// The upload with a multipart upload in progress.
fn upload_mid_transfer() -> UploadRef {
    UploadRef {
        multipart: Some(MultipartRef {
            id: "mpu_1".to_string(),
            size: 42,
            mtime: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            part_size: 32,
        }),
        ..upload_ref()
    }
}

fn calls(client: &MockClient) -> Vec<Call> {
    client.requests().iter().map(|r| r.call.clone()).collect()
}

// The new upload is saved before the transfer starts, so a failed transfer
// leaves it for the next attempt.
#[tokio::test]
async fn new_upload_is_saved_before_the_transfer() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    transfer.push_err();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor
        .upload(&make_job("a.log"), &checkpointer)
        .await
        .unwrap_err();

    assert_eq!(checkpointer.saved(), [upload_ref()]);
}

// A retry gets fresh credentials for the saved upload instead of creating a
// new one, and hands its multipart upload to the transfer.
#[tokio::test]
async fn retry_reuses_the_saved_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Ok(s3_credentials()));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let job = job_with(upload_mid_transfer());

    executor.upload(&job, &checkpointer).await.unwrap();

    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
    assert_eq!(client.requests()[0].path, "/uploads/upl_1/credentials");
    assert_eq!(
        transfer.recorded_calls(),
        vec![(s3_credentials(), upload_mid_transfer(), job.file.clone())]
    );
    assert!(checkpointer.saved().is_empty());
}

// The backend no longer accepts the upload (409 on credentials), but confirm
// succeeds: an earlier attempt already uploaded it.
#[tokio::test]
async fn already_uploaded_upload_is_done() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(409)));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor
        .upload(&job_with(upload_ref()), &checkpointer)
        .await
        .unwrap();

    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
    assert!(transfer.recorded_calls().is_empty());
    assert!(checkpointer.saved().is_empty());
}

// A deleted (404) or failed (409) upload is replaced with a new one, which
// starts without the old multipart upload.
#[tokio::test]
async fn gone_upload_is_replaced_with_a_new_one() {
    for status in [404, 409] {
        let client = Arc::new(MockClient::default());
        client.set_vend_upload_credentials(move || Err(request_failed_err(status)));
        let confirms = AtomicUsize::new(0);
        client.set_confirm_upload(move || match confirms.fetch_add(1, Ordering::SeqCst) {
            0 => Err(request_failed_err(status)),
            _ => Ok(*uploaded_response().upload),
        });
        client.set_create_upload(|| Ok(pending_response()));
        let transfer = MockObjectTransfer::new();
        let checkpointer = RecordingCheckpointer::new();
        let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

        executor
            .upload(&job_with(upload_mid_transfer()), &checkpointer)
            .await
            .unwrap();

        assert_eq!(
            calls(&client),
            vec![
                Call::VendUploadCredentials,
                Call::ConfirmUpload,
                Call::CreateUpload,
                Call::ConfirmUpload
            ],
            "status {status}"
        );
        assert_eq!(checkpointer.saved(), vec![upload_ref()], "status {status}");
        assert_eq!(
            transfer.recorded_calls()[0].1,
            upload_ref(),
            "status {status}"
        );
    }
}

// Any other credentials failure fails the attempt and keeps the saved upload.
#[tokio::test]
async fn credentials_failure_keeps_the_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(500)));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let err = executor
        .upload(&job_with(upload_ref()), &checkpointer)
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)));
    assert!(!err.is_terminal());
    assert_eq!(calls(&client), vec![Call::VendUploadCredentials]);
    assert!(transfer.recorded_calls().is_empty());
    assert!(checkpointer.saved().is_empty());
}
