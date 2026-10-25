// standard crates
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

// internal crates
use super::filesys::files::{self as test_files, TempFile};
use super::token_manager::MockTokenManager;
use backend_api::models::{
    Upload, UploadCredentials, UploadDestination, UploadStatus, UploadWithCredentials,
};
use miru_agent::authn::Token;
use miru_agent::data_uploads::upload::Job;
use miru_agent::filesys::{files, File, WriteOptions};

// external crates
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

/// The `idempotency_key` of the fixture job for `name`, the same on every call.
pub fn idempotency_key(name: &str) -> Uuid {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    Uuid::from_u64_pair(0, hasher.finish())
}

pub fn make_job(name: &str) -> Job {
    let now = Utc::now();
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: now,
        first_observed_at: now,
        last_observed_at: now,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
        idempotency_key: idempotency_key(name),
    }
}

/// [`make_job`] for a temp file holding `bytes`, with the file's size, mtime
/// and digest, so an attempt finds it unchanged. The file lives as long as the
/// returned guard.
pub async fn job_on_disk(name: &str, bytes: &[u8]) -> (TempFile, Job) {
    let tf = test_files::temp("upload-job").unwrap();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    let metadata = files::metadata(tf.file()).await.unwrap();
    let job = Job {
        file: tf.to_file(),
        size: metadata.len(),
        digest: files::hash(tf.file()).await.unwrap(),
        mtime: DateTime::<Utc>::from(metadata.modified().unwrap()),
        ..make_job(name)
    };
    (tf, job)
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
