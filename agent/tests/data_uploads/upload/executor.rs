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
    make_job, pending_response, response_with_status, s3_credentials, token_manager,
    upload_checkpoint,
};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus, UploadWithCredentials};
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::data_uploads::upload::executor::new_upl_request;
use miru_agent::data_uploads::upload::{
    Job, LiveExecutor, MultipartCheckpoint, PrepareOutcome, PreparedUpload, UploadCheckpoint,
    UploadErr, UploadExecutor,
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

/// Runs an attempt as the actor does: prepare, then upload unless the upload
/// was already confirmed.
async fn attempt<E: UploadExecutor>(executor: &E, job: &Job) -> Result<(), UploadErr> {
    match executor.prepare(job).await? {
        PrepareOutcome::Ready(prepared) => executor.upload(job, &prepared).await,
        PrepareOutcome::AlreadyUploaded => Ok(()),
    }
}

// ============================ MockUploadExecutor ================================= //

#[tokio::test]
async fn mock_follows_script() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::Ok);
    mock.push_step(MockStep::Err);
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    attempt(mock.as_ref(), &job_a).await.unwrap();
    let result = attempt(mock.as_ref(), &job_b).await;
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

    attempt(mock.as_ref(), &job).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job]);
}

// ============================== LiveExecutor =================================== //

#[tokio::test]
async fn happy_path_creates_transfers_confirms() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let transfer = MockObjectTransfer::new();
    let job = Job {
        crc32c: Some(42),
        ..make_job("a.log")
    };
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    attempt(&executor, &job).await.unwrap();

    // Exactly one transfer, carrying the create response's credentials, its
    // upload (the physical `bucket_name`, not the Miru `bucket_id`, plus key and
    // metadata) and the job's file and CRC32C.
    assert_eq!(
        transfer.recorded_calls(),
        vec![(
            s3_credentials(),
            upload_checkpoint(),
            job.file.clone(),
            Some(42)
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

    attempt(&executor, &make_job("a.log")).await.unwrap();

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

    let result = attempt(&executor, &make_job("a.log")).await;

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

    let err = attempt(&executor, &make_job("a.log")).await.unwrap_err();

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

    let result = attempt(&executor, &make_job("a.log")).await;

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

    let result = attempt(&executor, &make_job("a.log")).await;

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

    let result = attempt(&executor, &make_job("a.log")).await;

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

    let err = attempt(&executor, &make_job("a.log")).await.unwrap_err();

    assert!(err.is_terminal());
}

#[tokio::test]
async fn create_transient_statuses_are_not_terminal() {
    for status in [401u16, 408, 429, 500] {
        let client = Arc::new(MockClient::default());
        client.set_create_upload(move || Err(request_failed_err(status)));
        let transfer = MockObjectTransfer::new();
        let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

        let err = attempt(&executor, &make_job("a.log")).await.unwrap_err();

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

    let err = attempt(&executor, &make_job("a.log")).await.unwrap_err();

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
        crc32c: None,
        mtime: Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
        first_observed_at: Utc.with_ymd_and_hms(2021, 1, 1, 0, 1, 0).unwrap(),
        last_observed_at: Utc.with_ymd_and_hms(2021, 1, 1, 0, 2, 0).unwrap(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
        checkpoint: None,
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
        crc32c: None,
        // 01:30 UTC is 20:30 the evening before in New York (EST)
        mtime: Utc.with_ymd_and_hms(2026, 1, 15, 1, 30, 0).unwrap(),
        first_observed_at: Utc.with_ymd_and_hms(2026, 1, 15, 1, 31, 0).unwrap(),
        last_observed_at: Utc.with_ymd_and_hms(2026, 1, 15, 1, 32, 0).unwrap(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
        checkpoint: None,
    };
    let est = FixedOffset::west_opt(5 * 3600).unwrap();

    let req = new_upl_request(&job, &est);

    assert_eq!(req.source.mtime, "2026-01-14T20:30:00-05:00");
    assert_eq!(req.source.first_observed_at, "2026-01-15T01:31:00+00:00");
}

// =========================== Reusing the upload ================================= //

/// A job an earlier attempt already saved `checkpoint` on.
fn job_with(checkpoint: UploadCheckpoint) -> Job {
    let mut job = make_job("a.log");
    job.checkpoint = Some(checkpoint);
    job
}

/// The checkpoint of an upload with a multipart upload in progress.
fn upload_mid_transfer() -> UploadCheckpoint {
    UploadCheckpoint {
        multipart: Some(MultipartCheckpoint {
            multipart_upload_id: "mpu_1".to_string(),
            size: 42,
            part_size: 32,
        }),
        ..upload_checkpoint()
    }
}

/// What `prepare` returns for `checkpoint`, with the fixtures' credentials.
fn ready(checkpoint: UploadCheckpoint) -> PrepareOutcome {
    PrepareOutcome::Ready(PreparedUpload {
        checkpoint,
        credentials: s3_credentials(),
    })
}

fn calls(client: &MockClient) -> Vec<Call> {
    client.requests().iter().map(|r| r.call.clone()).collect()
}

// A new upload is prepared without transferring anything, so the actor can
// save it before the transfer starts.
#[tokio::test]
async fn new_upload_is_prepared_before_the_transfer() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let prepared = executor.prepare(&make_job("a.log")).await.unwrap();

    assert_eq!(prepared, ready(upload_checkpoint()));
    assert_eq!(calls(&client), vec![Call::CreateUpload]);
    assert!(transfer.recorded_calls().is_empty());
}

// The prepared checkpoint carries the multipart upload the transfer started,
// so the actor saves it before any part is sent.
#[tokio::test]
async fn prepared_checkpoint_carries_the_multipart_upload() {
    let client = Arc::new(MockClient::default());
    client.set_create_upload(|| Ok(pending_response()));
    let transfer = MockObjectTransfer::new();
    transfer.set_multipart(upload_mid_transfer().multipart.unwrap());
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let prepared = executor.prepare(&make_job("a.log")).await.unwrap();

    assert_eq!(prepared, ready(upload_mid_transfer()));
}

// Failing to prepare the multipart upload doesn't fail the attempt: the
// checkpoint keeps its saved multipart upload, which the transfer checks again.
#[tokio::test]
async fn multipart_failure_keeps_the_saved_multipart_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Ok(s3_credentials()));
    let transfer = MockObjectTransfer::new();
    transfer.fail_multipart();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let prepared = executor
        .prepare(&job_with(upload_mid_transfer()))
        .await
        .unwrap();

    assert_eq!(prepared, ready(upload_mid_transfer()));
}

// A retry gets fresh credentials for the saved upload instead of creating a
// new one, and hands its multipart upload to the transfer.
#[tokio::test]
async fn retry_reuses_the_saved_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Ok(s3_credentials()));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());
    let job = job_with(upload_mid_transfer());

    attempt(&executor, &job).await.unwrap();

    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
    assert_eq!(client.requests()[0].path, "/uploads/upl_1/credentials");
    assert_eq!(
        transfer.recorded_calls(),
        vec![(
            s3_credentials(),
            upload_mid_transfer(),
            job.file.clone(),
            job.crc32c
        )]
    );
}

// The backend no longer accepts the upload (409 on credentials), but confirm
// succeeds: an earlier attempt already uploaded it.
#[tokio::test]
async fn already_uploaded_upload_is_done() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(409)));
    client.set_confirm_upload(|| Ok(*uploaded_response().upload));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let prepared = executor
        .prepare(&job_with(upload_checkpoint()))
        .await
        .unwrap();

    assert_eq!(prepared, PrepareOutcome::AlreadyUploaded);
    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
    assert!(transfer.recorded_calls().is_empty());
}

// A deleted (404) or failed (409) upload is replaced with a new one, which
// starts without the old multipart upload.
#[tokio::test]
async fn gone_upload_is_replaced_with_a_new_one() {
    for status in [404, 409] {
        let client = Arc::new(MockClient::default());
        client.set_vend_upload_credentials(move || Err(request_failed_err(status)));
        client.set_confirm_upload(move || Err(request_failed_err(status)));
        client.set_create_upload(|| Ok(pending_response()));
        let transfer = MockObjectTransfer::new();
        let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

        let prepared = executor
            .prepare(&job_with(upload_mid_transfer()))
            .await
            .unwrap();

        assert_eq!(
            calls(&client),
            vec![
                Call::VendUploadCredentials,
                Call::ConfirmUpload,
                Call::CreateUpload
            ],
            "status {status}"
        );
        assert_eq!(prepared, ready(upload_checkpoint()), "status {status}");
    }
}

// A transient confirm failure while checking a gone upload fails the attempt
// instead of creating a new upload.
#[tokio::test]
async fn failed_check_of_a_gone_upload_keeps_it() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(409)));
    client.set_confirm_upload(|| Err(request_failed_err(500)));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let err = executor
        .prepare(&job_with(upload_checkpoint()))
        .await
        .unwrap_err();

    assert!(!err.is_terminal());
    assert_eq!(
        calls(&client),
        vec![Call::VendUploadCredentials, Call::ConfirmUpload]
    );
}

// Any other credentials failure fails the attempt, so the job keeps its saved
// upload for the next one.
#[tokio::test]
async fn credentials_failure_keeps_the_upload() {
    let client = Arc::new(MockClient::default());
    client.set_vend_upload_credentials(|| Err(request_failed_err(500)));
    let transfer = MockObjectTransfer::new();
    let executor = LiveExecutor::new(client.clone(), token_manager(), transfer.clone());

    let err = executor
        .prepare(&job_with(upload_checkpoint()))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)));
    assert!(!err.is_terminal());
    assert_eq!(calls(&client), vec![Call::VendUploadCredentials]);
    assert!(transfer.recorded_calls().is_empty());
}
