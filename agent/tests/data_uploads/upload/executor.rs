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
    destination, make_job, pending_response, response_metadata, response_with_status,
    s3_credentials, token_manager,
};
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus, UploadWithCredentials};
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::data_uploads::upload::executor::new_upl_request;
use miru_agent::data_uploads::upload::{Job, LiveExecutor, UploadErr, UploadExecutor};
use miru_agent::errors::Error;
use miru_agent::filesys::File;
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params;

// external crates
use chrono::{TimeZone, Utc};

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

    let err = executor.upload(&make_job("a.log")).await.unwrap_err();

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
    assert_eq!(new_upl_request(&job), expected);
}
