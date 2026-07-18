// standard crates
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// internal crates
use crate::mocks::http_client::run_server;
use backend_api::models::{S3UploadCredentials, UploadCredentials, UploadDestination};
use miru_agent::filesys::{files, File, WriteOptions};
use miru_agent::upload::transfer::s3_config;
use miru_agent::upload::{ObjectTransfer, SdkTransfer, UploadErr};

// external crates
use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_types::body::SdkBody;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};

fn destination() -> UploadDestination {
    UploadDestination {
        bucket_id: "bkt_1".to_string(),
        bucket_name: "my-bucket".to_string(),
        object_key: "logs/a.log".to_string(),
    }
}

fn metadata() -> HashMap<String, String> {
    HashMap::from([("device_id".to_string(), "dvc_1".to_string())])
}

fn s3_credentials_json() -> Value {
    json!({
        "scheme": "s3",
        "access_key_id": "AKIA_TEST",
        "secret_access_key": "secret",
        "session_token": "session",
        "region": "us-east-1",
        "expires_at": "2021-01-01T01:00:00Z"
    })
}

fn gcs_credentials_json(access_token: &str) -> Value {
    json!({
        "scheme": "gcs",
        "access_token": access_token,
        "expires_at": "2021-01-01T01:00:00Z"
    })
}

/// A full `UploadCredentials` for `scheme` with the given credential arms.
fn credentials(scheme: &str, s3: Value, gcs: Value) -> UploadCredentials {
    serde_json::from_value(json!({
        "scheme": scheme,
        "s3_credentials": s3,
        "gcs_credentials": gcs,
        "expires_at": "2021-01-01T01:00:00Z"
    }))
    .unwrap()
}

/// Writes `bytes` to a fresh temp file and returns the guard (kept alive so
/// the file is not deleted until the test drops it).
async fn temp_file_with(bytes: &[u8]) -> files::TempFile {
    let tf = files::temp("upload-transfer-test").unwrap();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    tf
}

/// The path-style PutObject URI the S3 SDK emits for [`destination`]'s bucket
/// and key with the vended `us-east-1` region: `bucket_name` (never the Miru
/// `bucket_id`) is the path's first segment.
const S3_PUT_URI: &str = "https://s3.us-east-1.amazonaws.com/my-bucket/logs/a.log?x-id=PutObject";

/// Builds an expected request (method + path-style URI, empty body) to match a
/// `ReplayEvent` against. Only the method and URI are meaningful; the streamed
/// body is not compared.
fn s3_req(method: &str, uri: &str) -> http::Request<SdkBody> {
    http::Request::builder()
        .method(method)
        .uri(uri)
        .body(SdkBody::empty())
        .unwrap()
}

/// Builds a canned response with `status` and `body`, setting `content-length`
/// to match. Pass `&[]` for a bodyless 200 OK.
fn s3_resp(status: u16, body: &[u8]) -> http::Response<SdkBody> {
    http::Response::builder()
        .status(status)
        .header("content-length", body.len().to_string())
        .body(SdkBody::from(body.to_vec()))
        .unwrap()
}

const ACCESS_DENIED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message><RequestId>REQ403</RequestId></Error>"#;

/// Canned S3 403 AccessDenied response carrying the XML `<Error>` document S3
/// returns for authorization failures.
fn s3_access_denied_resp() -> http::Response<SdkBody> {
    http::Response::builder()
        .status(403)
        .header("content-type", "application/xml")
        .body(SdkBody::from(ACCESS_DENIED_XML.to_string()))
        .unwrap()
}

/// Shared state recording what the GCS upload mock server observed.
#[derive(Clone, Default)]
struct GcsRecorder {
    inner: Arc<Mutex<GcsRecord>>,
}

#[derive(Default)]
struct GcsRecord {
    upload_hits: usize,
    /// Value of the `Authorization` header on the last upload request.
    authorization: String,
    /// Path-and-query of the last upload request.
    uri: String,
    /// Raw body of the last upload request (multipart framing included).
    body: Vec<u8>,
    /// If set, uploads respond with this status + GCS error JSON instead of
    /// 200 + Object JSON.
    status: Option<StatusCode>,
}

/// Handles the single-shot upload `POST /upload/storage/v1/b/{bucket}/o`,
/// recording the request and returning a minimal `Object` JSON that the
/// client decodes to finalize (or the configured error status).
async fn gcs_upload_handler(
    State(rec): State<GcsRecorder>,
    uri: axum::extract::OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> (
    StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    String,
) {
    let mut r = rec.inner.lock().unwrap();
    r.upload_hits += 1;
    r.authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    r.uri = uri.0.to_string();
    r.body = body.to_vec();
    let json_ct = [(axum::http::header::CONTENT_TYPE, "application/json")];
    if let Some(status) = r.status {
        return (
            status,
            json_ct,
            "{\"error\":{\"code\":403,\"message\":\"denied\"}}".to_string(),
        );
    }
    let object = json!({ "name": "logs/a.log", "bucket": "my-bucket" }).to_string();
    (StatusCode::OK, json_ct, object)
}

/// Builds the GCS upload router wired to the recorder.
fn gcs_router(rec: GcsRecorder) -> Router {
    Router::new()
        .route("/upload/storage/v1/b/{*rest}", post(gcs_upload_handler))
        .with_state(rec)
}

#[tokio::test]
async fn unknown_scheme_is_unsupported() {
    let creds = credentials("something-new", s3_credentials_json(), Value::Null);
    let err = SdkTransfer::default()
        .transfer(
            &creds,
            &destination(),
            &File::new("/data/a.log"),
            &HashMap::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(err.to_string().contains("unrecognized"), "message: {err}");
}

#[tokio::test]
async fn s3_scheme_without_credentials_errs() {
    let creds = credentials("s3", Value::Null, Value::Null);
    let err = SdkTransfer::default()
        .transfer(
            &creds,
            &destination(),
            &File::new("/data/a.log"),
            &HashMap::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(err.to_string().contains("s3_credentials"), "message: {err}");
}

#[tokio::test]
async fn s3_transfer_puts_object_to_bucket_name_and_key() {
    let replay = StaticReplayClient::new(vec![ReplayEvent::new(
        s3_req("PUT", S3_PUT_URI),
        s3_resp(200, &[]),
    )]);
    let src = temp_file_with(b"hello world").await;
    let creds = credentials("s3", s3_credentials_json(), Value::Null);

    SdkTransfer::with_s3_http_client(replay.clone())
        .transfer(&creds, &destination(), src.file(), &metadata())
        .await
        .unwrap();

    // The URI is the load-bearing assertion: `my-bucket` in the path proves
    // `bucket_name` reached the SDK (a `bkt_1` would mean `bucket_id` regressed
    // in), `logs/a.log` proves `object_key`, and the host proves the vended
    // region flows through. The streamed body can't be byte-compared — the
    // replay client records it as an unbuffered `SdkBody` (see
    // `tests/s3` `put_streams_file_body_bytes`).
    let requests = replay.actual_requests().collect::<Vec<_>>();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), "PUT");
    assert_eq!(requests[0].uri().to_string(), S3_PUT_URI);
    // The vended metadata map is stamped as user-defined object metadata.
    assert_eq!(
        requests[0].headers().get("x-amz-meta-device_id"),
        Some("dvc_1")
    );
}

#[tokio::test]
async fn s3_put_failure_maps_to_executor_err() {
    let replay = StaticReplayClient::new(vec![ReplayEvent::new(
        s3_req("PUT", S3_PUT_URI),
        s3_access_denied_resp(),
    )]);
    let src = temp_file_with(b"hello world").await;
    let creds = credentials("s3", s3_credentials_json(), Value::Null);

    let err = SdkTransfer::with_s3_http_client(replay.clone())
        .transfer(&creds, &destination(), src.file(), &HashMap::new())
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[tokio::test]
async fn s3_default_store_fails_offline_on_missing_file() {
    // No override: the production `s3::Store::new` arm runs. It does no I/O at
    // construction and `put` stats the file before dispatching any request, so
    // a missing local file fails fast offline.
    let creds = credentials("s3", s3_credentials_json(), Value::Null);
    let missing = File::new("/nonexistent/definitely/not/here.log");

    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &missing, &HashMap::new())
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[tokio::test]
async fn gcs_scheme_without_credentials_errs() {
    let creds = credentials("gcs", Value::Null, Value::Null);
    let err = SdkTransfer::default()
        .transfer(
            &creds,
            &destination(),
            &File::new("/data/a.log"),
            &HashMap::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(
        err.to_string().contains("gcs_credentials"),
        "message: {err}"
    );
}

#[tokio::test]
async fn gcs_invalid_token_surfaces_executor_err() {
    // A newline is not a valid HTTP header byte, so the GCS client build fails
    // offline — exercising the gcs arm's error mapping without a network.
    let creds = credentials("gcs", Value::Null, gcs_credentials_json("bad\ntoken"));
    let err = SdkTransfer::default()
        .transfer(
            &creds,
            &destination(),
            &File::new("/data/a.log"),
            &HashMap::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[tokio::test]
async fn gcs_transfer_puts_object_to_bucket_name_and_key() {
    let rec = GcsRecorder::default();
    let server = run_server(gcs_router(rec.clone())).await;
    let src = temp_file_with(b"hello world").await;
    let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

    SdkTransfer::with_gcs_endpoint(server.base_url)
        .transfer(&creds, &destination(), src.file(), &metadata())
        .await
        .unwrap();

    let r = rec.inner.lock().unwrap();
    // `my-bucket` in the upload path proves `bucket_name` reached the SDK (not
    // the Miru `bucket_id`), `name=logs%2Fa.log` proves `object_key`, and the
    // bearer token proves the vended credential flows through.
    assert_eq!(
        (r.upload_hits, r.authorization.as_str(), r.uri.as_str()),
        (
            1,
            "Bearer vended-token",
            "/upload/storage/v1/b/my-bucket/o?uploadType=multipart&name=logs%2Fa.log"
        )
    );
    assert!(
        r.body
            .windows(b"hello world".len())
            .any(|w| w == b"hello world"),
        "uploaded multipart body must contain the file bytes"
    );
    // The vended metadata map lands in the object-resource JSON part.
    let contains = |needle: &[u8]| r.body.windows(needle.len()).any(|w| w == needle);
    assert!(
        contains(b"device_id") && contains(b"dvc_1"),
        "uploaded multipart body must contain the metadata map"
    );
}

#[tokio::test]
async fn gcs_put_failure_maps_to_executor_err() {
    let rec = GcsRecorder::default();
    rec.inner.lock().unwrap().status = Some(StatusCode::FORBIDDEN);
    let server = run_server(gcs_router(rec)).await;
    let src = temp_file_with(b"hello world").await;
    let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

    let err = SdkTransfer::with_gcs_endpoint(server.base_url)
        .transfer(&creds, &destination(), src.file(), &HashMap::new())
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[tokio::test]
async fn gcs_default_store_fails_offline_on_missing_file() {
    // No override and a valid token: the production `gcs::Store::new` arm
    // builds successfully offline (the invalid-token test covers its build
    // error path) and `put` stats the file before dispatching any request, so
    // a missing local file fails fast offline.
    let creds = credentials("gcs", Value::Null, gcs_credentials_json("valid-token"));
    let missing = File::new("/nonexistent/definitely/not/here.log");

    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &missing, &HashMap::new())
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[test]
fn s3_config_maps_credentials() {
    let creds: S3UploadCredentials = serde_json::from_value(s3_credentials_json()).unwrap();
    let cfg = s3_config(&creds);
    assert_eq!(
        (
            cfg.region.as_str(),
            cfg.creds.access_key_id.as_str(),
            cfg.creds.secret_access_key.as_str(),
            cfg.creds.session_token.as_str(),
        ),
        ("us-east-1", "AKIA_TEST", "secret", "session")
    );
}
