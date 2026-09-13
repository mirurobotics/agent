// standard crates
use std::sync::Arc;

// internal crates
use crate::test_utils::{
    filesys::files as test_files,
    http_client::{Call, MockClient},
    upload::{make_job, pending_response, token_manager},
};
use miru_agent::data_uploads::upload::{LiveExecutor, SdkTransfer, UploadExecutor};
use miru_agent::filesys::{files, WriteOptions};

// external crates
use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_types::body::SdkBody;

/// The path-style PutObject URI for the upload fixture's bucket
/// and key with the vended `us-east-1` region (see the transfer tests).
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
    let src = test_files::temp("upload-executor-test").unwrap();
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
