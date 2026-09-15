// standard crates
use std::collections::HashMap;
use std::sync::Arc;

// internal crates
use super::filesys::abs_file;
use super::token_manager::MockTokenManager;
use backend_api::models::{
    Upload, UploadCredentials, UploadDestination, UploadStatus, UploadWithCredentials,
};
use miru_agent::authn::Token;
use miru_agent::data_uploads::upload::Job;

// external crates
use chrono::Utc;
use serde_json::{json, Value};

pub fn make_job(name: &str) -> Job {
    let now = Utc::now();
    Job {
        file: abs_file(&format!("data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: now,
        first_observed_at: now,
        last_observed_at: now,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
    }
}

pub fn destination() -> UploadDestination {
    UploadDestination {
        bucket_id: "bkt_1".to_string(),
        bucket_name: "my-bucket".to_string(),
        object_key: "logs/a.log".to_string(),
    }
}

/// The inner `s3_credentials` arm of [`s3_credentials`], as vended JSON.
pub fn s3_credentials_json() -> Value {
    json!({
        "scheme": "s3",
        "access_key_id": "AKIA_TEST",
        "secret_access_key": "secret",
        "session_token": "session",
        "region": "us-east-1",
        "expires_at": "2021-01-01T01:00:00Z"
    })
}

pub fn s3_credentials() -> UploadCredentials {
    serde_json::from_value(json!({
        "scheme": "s3",
        "s3_credentials": s3_credentials_json(),
        "gcs_credentials": null,
        "expires_at": "2021-01-01T01:00:00Z"
    }))
    .unwrap()
}

pub fn response_metadata() -> HashMap<String, String> {
    HashMap::from([("device_id".to_string(), "dvc_1".to_string())])
}

/// A `POST /uploads` response for upload `upl_1` (destination
/// `my-bucket`/`logs/a.log`, s3 credentials) in the given `status`.
pub fn response_with_status(status: UploadStatus) -> UploadWithCredentials {
    UploadWithCredentials {
        upload: Box::new(Upload {
            id: "upl_1".to_string(),
            status,
            destination: Box::new(destination()),
            ..Default::default()
        }),
        credentials: Box::new(s3_credentials()),
        metadata: response_metadata(),
    }
}

pub fn pending_response() -> UploadWithCredentials {
    response_with_status(UploadStatus::UPLOAD_STATUS_PENDING)
}

pub fn token_manager() -> Arc<MockTokenManager> {
    Arc::new(MockTokenManager::new(Token {
        token: "test-token".to_string(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    }))
}
