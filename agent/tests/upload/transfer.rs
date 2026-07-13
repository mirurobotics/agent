// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use crate::mocks::http_client::run_server;
use crate::mocks::upload_client::{credentials_json, s3_credentials_json};
use backend_api::models::{
    GcsUploadCredentials, S3UploadCredentials, UploadCredentials, UploadDestination,
};
use miru_agent::filesys::{files, File, WriteOptions};
use miru_agent::upload::transfer::{gcs_credentials, s3_config};
use miru_agent::upload::{ObjectTransfer, SdkTransfer, UploadErr};

// external crates
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::routing::post;
use axum::Router;
use serde_json::json;

fn destination() -> UploadDestination {
    UploadDestination {
        bucket_id: "bkt_1".to_string(),
        bucket_name: "my-bucket".to_string(),
        object_key: "logs/a.log".to_string(),
    }
}

fn creds_from(value: serde_json::Value) -> UploadCredentials {
    serde_json::from_value(value).unwrap()
}

/// A full `UploadCredentials` object as JSON with the GCS arm populated and
/// the S3 arm null, carrying the given access token.
fn gcs_creds_json(token: &str) -> serde_json::Value {
    json!({
        "scheme": "gcs",
        "s3_credentials": null,
        "gcs_credentials": {
            "scheme": "gcs",
            "access_token": token,
            "expires_at": "2021-01-01T01:00:00Z"
        },
        "expires_at": "2021-01-01T01:00:00Z"
    })
}

// ============================ GCS DATA-PATH MOCK ============================ //
//
// Minimal mock of the GCS single-shot upload endpoint, injected into
// `SdkTransfer` via its test-only `gcs_endpoint` seam so the happy path drives
// the real trait dispatch (`transfer` → scheme match → `transfer_gcs`). It
// records the hit count and `Authorization` header; the richer recorder in
// `tests/gcs/mod.rs` stays specific to the gcs suite.

/// Shared state recording what the GCS upload mock observed.
#[derive(Clone, Default)]
struct GcsRecorder {
    inner: Arc<Mutex<GcsRecord>>,
}

#[derive(Default)]
struct GcsRecord {
    upload_hits: usize,
    /// The `Authorization` header of the last upload request.
    auth_header: Option<String>,
    /// If set, uploads respond with this status and a GCS error JSON body.
    upload_status: Option<StatusCode>,
}

/// Handles the single-shot upload `POST /upload/storage/v1/b/{bucket}/o`.
/// Returns a minimal `Object` JSON that the client decodes to finalize.
async fn upload_handler(
    State(rec): State<GcsRecorder>,
    headers: HeaderMap,
) -> (StatusCode, [(HeaderName, &'static str); 1], String) {
    let mut r = rec.inner.lock().unwrap();
    r.upload_hits += 1;
    r.auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let json_ct = [(axum::http::header::CONTENT_TYPE, "application/json")];
    if let Some(status) = r.upload_status {
        return (
            status,
            json_ct,
            "{\"error\":{\"code\":403,\"message\":\"denied\"}}".to_string(),
        );
    }
    let object = json!({
        "name": "logs/a.log",
        "bucket": "my-bucket",
    })
    .to_string();
    (StatusCode::OK, json_ct, object)
}

fn gcs_router(rec: GcsRecorder) -> Router {
    Router::new()
        .route("/upload/storage/v1/b/{*rest}", post(upload_handler))
        .with_state(rec)
}

/// Writes `bytes` to a fresh temp file and returns the guard (kept alive so
/// the file is not deleted until the test drops it).
async fn temp_file_with(bytes: &[u8]) -> files::TempFile {
    let tf = files::temp("transfer-gcs").unwrap();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    tf
}

#[tokio::test]
async fn gcs_scheme_uploads_file_to_store() {
    let src = temp_file_with(b"hello gcs").await;
    let rec = GcsRecorder::default();
    let server = run_server(gcs_router(rec.clone())).await;
    let transfer = SdkTransfer {
        gcs_endpoint: Some(server.base_url),
    };
    let creds = creds_from(gcs_creds_json("gcs-token-test"));

    transfer
        .transfer(&creds, &destination(), &src.to_file())
        .await
        .unwrap();

    let r = rec.inner.lock().unwrap();
    assert_eq!(r.upload_hits, 1);
    // The vended token flowed through gcs_credentials into the request.
    assert_eq!(r.auth_header.as_deref(), Some("Bearer gcs-token-test"));
}

#[tokio::test]
async fn gcs_transfer_error_maps_to_executor_err() {
    let src = temp_file_with(b"payload").await;
    let rec = GcsRecorder::default();
    rec.inner.lock().unwrap().upload_status = Some(StatusCode::FORBIDDEN);
    let server = run_server(gcs_router(rec)).await;
    let transfer = SdkTransfer {
        gcs_endpoint: Some(server.base_url),
    };
    let creds = creds_from(gcs_creds_json("gcs-token-test"));

    let err = transfer
        .transfer(&creds, &destination(), &src.to_file())
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[tokio::test]
async fn gcs_scheme_without_credentials_errs() {
    // `credentials_json("gcs")` populates the s3 arm and nulls the gcs arm —
    // exactly the missing-gcs_credentials shape.
    let creds = creds_from(credentials_json("gcs"));
    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(err.to_string().contains("gcs_credentials"), "message: {err}");
}

#[test]
fn gcs_credentials_maps_access_token() {
    let creds: GcsUploadCredentials = serde_json::from_value(json!({
        "scheme": "gcs",
        "access_token": "gcs-token-test",
        "expires_at": "2021-01-01T01:00:00Z"
    }))
    .unwrap();
    let mapped = gcs_credentials(&creds);
    assert_eq!(mapped.access_token, "gcs-token-test");
}

#[tokio::test]
async fn unknown_scheme_is_unsupported() {
    let creds = creds_from(credentials_json("something-new"));
    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}

#[test]
fn s3_config_maps_credentials_and_endpoint() {
    let creds: S3UploadCredentials = serde_json::from_value(s3_credentials_json()).unwrap();
    let cfg = s3_config(&creds);
    assert_eq!(cfg.region, "us-east-1");
    assert_eq!(
        cfg.endpoint.as_deref(),
        Some("https://s3.us-east-1.amazonaws.com")
    );
    assert_eq!(cfg.creds.access_key_id, "AKIA_TEST");
}

#[tokio::test]
async fn s3_scheme_without_credentials_errs() {
    let creds = creds_from(json!({
        "scheme": "s3",
        "s3_credentials": null,
        "gcs_credentials": null,
        "expires_at": "2021-01-01T01:00:00Z"
    }));
    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}
