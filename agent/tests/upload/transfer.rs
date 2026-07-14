// internal crates
use backend_api::models::{S3UploadCredentials, UploadCredentials, UploadDestination};
use miru_agent::filesys::File;
use miru_agent::upload::transfer::s3_config;
use miru_agent::upload::{ObjectTransfer, SdkTransfer, UploadErr};

// external crates
use serde_json::{json, Value};

fn destination() -> UploadDestination {
    UploadDestination {
        bucket_id: "bkt_1".to_string(),
        bucket_name: "my-bucket".to_string(),
        object_key: "logs/a.log".to_string(),
    }
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

#[tokio::test]
async fn unknown_scheme_is_unsupported() {
    let creds = credentials("something-new", s3_credentials_json(), Value::Null);
    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(err.to_string().contains("unrecognized"), "message: {err}");
}

#[tokio::test]
async fn s3_scheme_without_credentials_errs() {
    let creds = credentials("s3", Value::Null, Value::Null);
    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(err.to_string().contains("s3_credentials"), "message: {err}");
}

#[tokio::test]
async fn gcs_scheme_without_credentials_errs() {
    let creds = credentials("gcs", Value::Null, Value::Null);
    let err = SdkTransfer::default()
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
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
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
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
