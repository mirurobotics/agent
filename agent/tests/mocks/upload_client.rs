// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::http::{
    self,
    request::{Meta, Params},
    HTTPErr,
};

// external crates
use serde_json::{json, Value};

// ============================== CANNED JSON BUILDERS ============================= //

/// A default `Upload` ledger entry as JSON, matching the generated model shape.
pub fn upload_json(id: &str, status: &str, bucket_name: &str, object_key: &str) -> Value {
    json!({
        "object": "upload",
        "id": id,
        "device_id": "dvc_1",
        "upload_rule_id": "rule_1",
        "upload_collection_name": "Robot Logs",
        "source": { "file_path": "/data/a.log", "mtime": "2021-01-01T00:00:00Z", "first_observed_at": "2021-01-01T00:00:00Z", "last_observed_at": "2021-01-01T00:00:00Z" },
        "destination": { "bucket_id": "bkt_1", "bucket_name": bucket_name, "object_key": object_key },
        "digest": "sha256:a",
        "size": 42,
        "status": status,
        "incomplete": false,
        "release_id": "rls_1",
        "deployment_id": "dpl_1",
        "uploaded_at": null,
        "created_at": "2021-01-01T00:00:00Z",
        "updated_at": "2021-01-01T00:00:00Z",
        "workspace_id": "wksp_1"
    })
}

/// S3 credential arm as JSON.
pub fn s3_credentials_json() -> Value {
    json!({
        "scheme": "s3",
        "access_key_id": "AKIA_TEST",
        "secret_access_key": "secret",
        "session_token": "session",
        "region": "us-east-1",
        "endpoint": "https://s3.us-east-1.amazonaws.com",
        "expires_at": "2021-01-01T01:00:00Z"
    })
}

/// A full `UploadCredentials` object as JSON for the given `scheme`, with the
/// S3 arm populated and the GCS arm null.
pub fn credentials_json(scheme: &str) -> Value {
    json!({
        "scheme": scheme,
        "s3_credentials": s3_credentials_json(),
        "gcs_credentials": null,
        "expires_at": "2021-01-01T01:00:00Z"
    })
}

/// A full `UploadWithCredentials` response as JSON.
pub fn upload_with_credentials_json(status: &str, scheme: &str) -> Value {
    json!({
        "upload": upload_json("upl_1", status, "my-bucket", "logs/a.log"),
        "credentials": credentials_json(scheme)
    })
}

// ============================== CAPTURED REQUEST ================================= //

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedUpload {
    pub method: reqwest::Method,
    pub path: String,
    pub body: Option<String>,
    pub token: Option<String>,
}

// ================================ MOCK CLIENT =================================== //

/// A `ClientI` serving the three upload-broker routes with canned JSON, capturing
/// every request. Responses are configurable so tests can drive success,
/// dedup, and decode-failure paths. An unset route returns a default body.
pub struct MockUploadClient {
    create: Mutex<String>,
    confirm: Mutex<String>,
    credentials: Mutex<String>,
    pub requests: Arc<Mutex<Vec<CapturedUpload>>>,
}

impl Default for MockUploadClient {
    fn default() -> Self {
        Self {
            create: Mutex::new(upload_with_credentials_json("pending", "s3").to_string()),
            confirm: Mutex::new(
                upload_json("upl_1", "uploaded", "my-bucket", "logs/a.log").to_string(),
            ),
            credentials: Mutex::new(credentials_json("s3").to_string()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockUploadClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_create(&self, body: impl Into<String>) {
        *self.create.lock().unwrap() = body.into();
    }

    pub fn set_confirm(&self, body: impl Into<String>) {
        *self.confirm.lock().unwrap() = body.into();
    }

    pub fn requests(&self) -> Vec<CapturedUpload> {
        self.requests.lock().unwrap().clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.requests().into_iter().map(|r| r.path).collect()
    }
}

impl http::ClientI for MockUploadClient {
    fn base_url(&self) -> &str {
        "http://mock"
    }

    async fn execute(&self, params: Params<'_>) -> Result<(String, Meta), HTTPErr> {
        let meta = params.meta()?;
        let path = params
            .url
            .strip_prefix(http::ClientI::base_url(self))
            .unwrap_or(params.url);
        let path = path.split('?').next().unwrap_or(path).to_string();
        self.requests.lock().unwrap().push(CapturedUpload {
            method: params.method.clone(),
            path: path.clone(),
            body: params.body.clone(),
            token: params.token.map(|t| t.to_string()),
        });

        let is_post = params.method == reqwest::Method::POST;
        let body = if is_post && path == "/uploads" {
            self.create.lock().unwrap().clone()
        } else if is_post && path.ends_with("/confirm") {
            self.confirm.lock().unwrap().clone()
        } else if is_post && path.ends_with("/credentials") {
            self.credentials.lock().unwrap().clone()
        } else {
            panic!("MockUploadClient: unhandled route {} {path}", params.method);
        };
        Ok((body, meta))
    }
}
