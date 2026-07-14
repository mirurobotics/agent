// internal crates
use crate::mocks::upload_client::{credentials_json, s3_credentials_json};
use backend_api::models::{S3UploadCredentials, UploadCredentials, UploadDestination};
use miru_agent::filesys::File;
use miru_agent::upload::transfer::s3_config;
use miru_agent::upload::{ObjectTransfer, SdkTransfer, UploadErr};

// external crates
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

#[tokio::test]
async fn gcs_scheme_is_unsupported() {
    let creds = creds_from(credentials_json("gcs"));
    let err = SdkTransfer
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    assert!(err.to_string().contains("GCS"), "message: {err}");
}

#[tokio::test]
async fn unknown_scheme_is_unsupported() {
    let creds = creds_from(credentials_json("something-new"));
    let err = SdkTransfer
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
    let err = SdkTransfer
        .transfer(&creds, &destination(), &File::new("/data/a.log"))
        .await
        .unwrap_err();

    assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
}
