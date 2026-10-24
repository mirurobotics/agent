// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// internal crates
use crate::mocks::{
    checkpointer::RecordingCheckpointer,
    http_client::{Call, MockClient},
    object_transfer::MockObjectTransfer,
    stub_token_manager::StubTokenManager,
    upload_executor::{MockStep, MockUploadExecutor},
};
use crate::test_utils::upload::{
    destination, make_job, pending_response, response_metadata, response_with_status,
    s3_credentials, token_manager,
};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus, UploadWithCredentials};
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::data_uploads::upload::executor::new_upl_request;
use miru_agent::data_uploads::upload::{
    Checkpoint, Job, LiveExecutor, MultipartState, NoopCheckpointer, TransferState, UploadErr,
    UploadExecutor, UploadRef,
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
        transfer: None,
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
        transfer: None,
    };
    let est = FixedOffset::west_opt(5 * 3600).unwrap();

    let req = new_upl_request(&job, &est);

    assert_eq!(req.source.mtime, "2026-01-14T20:30:00-05:00");
    assert_eq!(req.source.first_observed_at, "2026-01-15T01:31:00+00:00");
}

// ===== reusing the upload across attempts =====

fn upload_ref() -> UploadRef {
    UploadRef {
        id: "upl_1".to_string(),
        bucket_id: "bkt_1".to_string(),
        bucket_name: "my-bucket".to_string(),
        object_key: "logs/a.log".to_string(),
        metadata: response_metadata(),
        transferred: false,
    }
}

fn multipart_state() -> TransferState {
    TransferState::Multipart(MultipartState {
        multipart_upload_id: "mpu_1".to_string(),
        part_size: 8 * 1024 * 1024,
        size: 42,
        mtime: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
    })
}

/// A job a previous attempt already created an upload for.
fn job_with_upload(upload: UploadRef, transfer: Option<TransferState>) -> Job {
    let mut job = make_job("a.log");
    job.upload = Some(upload);
    job.transfer = transfer;
    job
}

fn calls(client: &MockClient) -> Vec<Call> {
    client.requests().iter().map(|r| r.call.clone()).collect()
}

// The upload id is persisted before any bytes move, and the transfer is
// marked done before the confirm.
#[tokio::test]
async fn first_attempt_checkpoints_the_upload_before_transferring() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    // saved during the transfer, to pin where the transfer ran
    transfer.push_checkpoints(vec![Checkpoint::TransferCleared]);
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor
        .upload(&make_job("a.log"), &checkpointer)
        .await
        .unwrap();

    assert_eq!(
        checkpointer.saved(),
        vec![
            Checkpoint::Created(upload_ref()),
            Checkpoint::TransferCleared,
            Checkpoint::Transferred,
        ]
    );
    assert_eq!(transfer.recorded_states(), vec![None]);
}

#[tokio::test]
async fn failed_transfer_keeps_the_upload_and_is_not_marked_transferred() {
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

    assert_eq!(
        checkpointer.saved(),
        vec![Checkpoint::Created(upload_ref())]
    );
    assert_eq!(client.call_count(Call::ConfirmUpload), 0);
}

// A retry gets fresh credentials for the persisted upload instead of creating
// a new one, and hands the persisted transfer state to the transfer.
#[tokio::test]
async fn retry_vends_credentials_for_the_persisted_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Ok(s3_credentials()));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let job = job_with_upload(upload_ref(), Some(multipart_state()));

    executor.upload(&job, &checkpointer).await.unwrap();

    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
    assert_eq!(client.requests()[0].path, "/uploads/upl_1/credentials");
    assert_eq!(
        transfer.recorded_calls(),
        vec![(
            s3_credentials(),
            destination(),
            job.file.clone(),
            response_metadata()
        )]
    );
    assert_eq!(transfer.recorded_states(), vec![Some(multipart_state())]);
    assert_eq!(checkpointer.saved(), vec![Checkpoint::Transferred]);
}

// The object already landed on an earlier attempt: only the confirm is left.
#[tokio::test]
async fn transferred_upload_only_confirms() {
    let client = Arc::new(MockClient::default());
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let mut upload = upload_ref();
    upload.transferred = true;

    executor
        .upload(&job_with_upload(upload, None), &checkpointer)
        .await
        .unwrap();

    assert_eq!(calls(&client), vec![Call::ConfirmUpload]);
    assert!(transfer.recorded_calls().is_empty());
    assert!(checkpointer.saved().is_empty());
}

#[tokio::test]
async fn deleted_upload_is_replaced_with_a_new_one() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(404)));
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let job = job_with_upload(upload_ref(), Some(multipart_state()));

    executor.upload(&job, &checkpointer).await.unwrap();

    assert_eq!(
        calls(&client),
        vec![
            Call::VendUploadCredentials,
            Call::CreateUpload,
            Call::ConfirmUpload
        ]
    );
    assert_eq!(
        checkpointer.saved(),
        vec![
            Checkpoint::UploadCleared,
            Checkpoint::Created(upload_ref()),
            Checkpoint::Transferred,
        ]
    );
    // the stale transfer state is not handed to the new upload
    assert_eq!(transfer.recorded_states(), vec![None]);
}

// A 409 on credentials means the upload is no longer pending; a confirm that
// succeeds shows it was already uploaded.
#[tokio::test]
async fn conflict_on_an_already_uploaded_upload_is_success() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(409)));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor
        .upload(&job_with_upload(upload_ref(), None), &checkpointer)
        .await
        .unwrap();

    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
    assert!(transfer.recorded_calls().is_empty());
    assert!(checkpointer.saved().is_empty());
}

// A 409 on credentials and on confirm: the upload failed on the backend, so
// the job starts over with a new upload.
#[tokio::test]
async fn conflict_on_a_failed_upload_creates_a_new_one() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(409)));
    let confirms = Arc::new(AtomicUsize::new(0));
    let seen = confirms.clone();
    client.set_confirm_upload(move || match seen.fetch_add(1, Ordering::SeqCst) {
        0 => Err(request_failed_err(409)),
        _ => Ok(*uploaded_response().upload),
    });
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    executor
        .upload(&job_with_upload(upload_ref(), None), &checkpointer)
        .await
        .unwrap();

    assert_eq!(
        calls(&client),
        vec![
            Call::VendUploadCredentials,
            Call::ConfirmUpload,
            Call::CreateUpload,
            Call::ConfirmUpload
        ]
    );
    assert_eq!(transfer.recorded_calls().len(), 1);
    assert_eq!(
        checkpointer.saved(),
        vec![
            Checkpoint::UploadCleared,
            Checkpoint::Created(upload_ref()),
            Checkpoint::Transferred,
        ]
    );
}

// Any other credentials failure keeps the persisted upload for the next try.
#[tokio::test]
async fn credentials_failure_keeps_the_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(500)));
    let transfer = MockObjectTransfer::new();
    let checkpointer = RecordingCheckpointer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let err = executor
        .upload(&job_with_upload(upload_ref(), None), &checkpointer)
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)));
    assert!(!err.is_terminal());
    assert_eq!(client.call_count(Call::CreateUpload), 0);
    assert!(transfer.recorded_calls().is_empty());
    assert!(checkpointer.saved().is_empty());
}

#[tokio::test]
async fn credentials_token_failure_maps_to_executor_err() {
    let client = Arc::new(MockClient::default());
    let token_mngr = Arc::new(StubTokenManager::err(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    })));
    let executor = LiveExecutor::new(client.clone(), token_mngr, MockObjectTransfer::new());

    let err = executor
        .upload(&job_with_upload(upload_ref(), None), &NoopCheckpointer)
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)));
    assert!(client.requests().is_empty());
}
