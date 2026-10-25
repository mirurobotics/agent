//! Real-GCS checks for uploads, which go through GCS's XML API with aws-sdk-s3
//! (see `agent/src/s3/gcs.rs`). Google doesn't document how GCS treats the
//! SDK's requests, and the worst break is silent: GCS stores the wrong bytes
//! and still answers 200. So these tests upload the way the agent does, with a
//! token downscoped like the backend's, and read each object back.
//!
//! They skip unless `GCS_IT_SOURCE_TOKEN` (an access token with
//! `roles/storage.objectAdmin` on the bucket) and `GCS_IT_BUCKET` are set.
//! CI's `test` job sets both whenever it can authenticate. The tests don't
//! delete what they write; the bucket's lifecycle rule does.

// standard crates
use std::collections::HashMap;

// internal crates
use crate::test_utils::filesys::files as test_files;
use miru_agent::filesys::{files, WriteOptions};
use miru_agent::s3::gcs::{self, Token};
use miru_agent::s3::{sdk, Object, Options, S3Err, Source, Store};

// external crates
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use base64::Engine;
use md5::{Digest, Md5};
use serde_json::json;
use uuid::Uuid;

const GCS_URL: &str = "https://storage.googleapis.com";
const STS_URL: &str = "https://sts.googleapis.com/v1/token";
const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";
/// The store's multipart part size (`s3::PART_SIZE`).
const PART: usize = 8 * 1024 * 1024;
/// Uploads with one `PutObject`.
const SMALL: usize = 64 * 1024;
/// Uploads in two parts.
const LARGE: usize = PART + 1024;

/// The test bucket, and a token with `roles/storage.objectAdmin` on it.
struct Gcs {
    bucket: String,
    source_token: String,
    /// Unique per test, so no two runs share an object.
    prefix: String,
}

impl Gcs {
    /// `None`, so the test skips, unless the token and bucket are set. CI sets
    /// the token to an empty string when it can't authenticate.
    fn load() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let source_token = var("GCS_IT_SOURCE_TOKEN")?;
        let bucket = var("GCS_IT_BUCKET")?;
        let prefix = format!("agent-ci/gcs-xml/{}", Uuid::new_v4());
        Some(Self {
            bucket,
            source_token,
            prefix,
        })
    }

    fn object(&self, name: &str) -> Object {
        Object {
            bucket: self.bucket.clone(),
            key: format!("{}/{name}", self.prefix),
        }
    }

    /// A store that uploads `obj` the way the agent does.
    async fn upload_store(&self, obj: &Object) -> Store {
        let config = self.upload_config(obj).await;
        Store::new(Options { config, gcs: true })
    }

    /// A client with the same config, for the calls `Store` doesn't expose.
    async fn upload_client(&self, obj: &Object) -> Client {
        Client::from_conf(self.upload_config(obj).await)
    }

    /// The production GCS config, with a token that can only create `obj`.
    async fn upload_config(&self, obj: &Object) -> aws_sdk_s3::Config {
        let token = Token::new(self.upload_token(obj).await).unwrap();
        gcs::config(token, sdk::http_client())
    }

    /// Downscopes the source token to creating `obj` alone, as the backend
    /// does for each upload (`backend/internal/pkg/gcp/creds/gcs.go`).
    async fn upload_token(&self, obj: &Object) -> String {
        let bucket = format!("projects/_/buckets/{}", obj.bucket);
        let rule = json!({
            "availableResource": format!("//storage.googleapis.com/{bucket}"),
            "availablePermissions": ["inRole:roles/storage.objectCreator"],
            "availabilityCondition": {
                "expression": format!("resource.name == '{bucket}/objects/{}'", obj.key),
            },
        });
        let options = json!({ "accessBoundary": { "accessBoundaryRules": [rule] } }).to_string();
        let form = [
            ("grant_type", TOKEN_EXCHANGE),
            ("subject_token_type", ACCESS_TOKEN),
            ("requested_token_type", ACCESS_TOKEN),
            ("subject_token", self.source_token.as_str()),
            ("options", options.as_str()),
        ];
        let resp = reqwest::Client::new()
            .post(STS_URL)
            .form(&form)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(status.is_success(), "token exchange: {status} {body}");
        body["access_token"].as_str().unwrap().to_string()
    }

    /// Reads `obj` back with the source token, outside the SDK under test, and
    /// checks it holds `expected` and the test metadata.
    async fn assert_stored(&self, obj: &Object, expected: &[u8]) {
        let resp = reqwest::Client::new()
            .get(format!("{GCS_URL}/{}/{}", obj.bucket, obj.key))
            .bearer_auth(&self.source_token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let metadata = resp.headers().get("x-goog-meta-miru-ci");
        assert_eq!(
            metadata.and_then(|v| v.to_str().ok()),
            Some("gcs-xml"),
            "{obj}: metadata"
        );
        let stored = resp.bytes().await.unwrap();
        assert!(
            stored == expected,
            "{obj}: stored {} bytes that don't match the {} uploaded",
            stored.len(),
            expected.len()
        );
    }
}

/// The metadata every test object carries. GCS returns it as
/// `x-goog-meta-miru-ci`.
fn metadata() -> HashMap<String, String> {
    HashMap::from([("miru-ci".to_string(), "gcs-xml".to_string())])
}

/// A repeating pattern whose period (251, a prime) doesn't divide the part
/// size, so a dropped or reordered part changes the bytes.
fn test_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Base64 MD5 of `bytes`, for a `Content-MD5` header.
fn md5_of(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(bytes))
}

/// A temp file holding `bytes`, deleted when dropped.
async fn temp_file_with(bytes: &[u8]) -> test_files::TempFile {
    let tf = test_files::temp("gcs-xml").unwrap();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    tf
}

/// Single-part and multipart uploads store the exact bytes and the metadata.
#[tokio::test]
async fn put_stores_exact_bytes_and_metadata() {
    let Some(gcs) = Gcs::load() else { return };
    for (name, len) in [("single.bin", SMALL), ("multi.bin", LARGE)] {
        let obj = gcs.object(name);
        let bytes = test_bytes(len);
        let file = temp_file_with(&bytes).await;

        let store = gcs.upload_store(&obj).await;
        store
            .put(file.to_file(), &obj, &metadata())
            .await
            .expect(name);
        gcs.assert_stored(&obj, &bytes).await;
    }
}

/// Resuming an upload another client started, with a new token as after an
/// agent restart, completes it with the exact bytes.
#[tokio::test]
async fn resume_completes_a_started_upload() {
    let Some(gcs) = Gcs::load() else { return };
    let obj = gcs.object("resume.bin");
    let bytes = test_bytes(LARGE);
    let file = temp_file_with(&bytes).await;

    let client = gcs.upload_client(&obj).await;
    let upload = client
        .create_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .set_metadata(Some(metadata()))
        .send()
        .await
        .unwrap();
    let upload_id = upload.upload_id().unwrap();
    let part = &bytes[..PART];
    client
        .upload_part()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(upload_id)
        .part_number(1)
        .content_md5(md5_of(part))
        .body(ByteStream::from(part.to_vec()))
        .send()
        .await
        .unwrap();

    let store = gcs.upload_store(&obj).await;
    let src = Source::open(file.to_file(), &obj).await.unwrap();
    store
        .resume_multipart_upload(src, &obj, upload_id)
        .await
        .unwrap();
    gcs.assert_stored(&obj, &bytes).await;
}

/// GCS rejects a body that doesn't match its Content-MD5. With the SDK's own
/// checksums off, that is what catches a corrupted body.
#[tokio::test]
async fn wrong_content_md5_is_rejected() {
    let Some(gcs) = Gcs::load() else { return };
    let obj = gcs.object("bad-digest.bin");
    let client = gcs.upload_client(&obj).await;

    let err = client
        .put_object()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .content_md5(md5_of(b"not the body"))
        .body(ByteStream::from(test_bytes(SMALL)))
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some("BadDigest"), "{err:?}");
}

/// Resuming an upload GCS no longer has fails with `NoSuchUploadErr`.
#[tokio::test]
async fn resuming_an_aborted_upload_fails_with_no_such_upload() {
    let Some(gcs) = Gcs::load() else { return };
    let obj = gcs.object("aborted.bin");
    let file = temp_file_with(&test_bytes(LARGE)).await;

    let client = gcs.upload_client(&obj).await;
    let upload = client
        .create_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .send()
        .await
        .unwrap();
    let upload_id = upload.upload_id().unwrap();
    client
        .abort_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(upload_id)
        .send()
        .await
        .unwrap();

    let store = gcs.upload_store(&obj).await;
    let src = Source::open(file.to_file(), &obj).await.unwrap();
    let err = store
        .resume_multipart_upload(src, &obj, upload_id)
        .await
        .unwrap_err();
    assert!(matches!(err, S3Err::NoSuchUploadErr(_)), "{err:?}");
}

/// A put that finds its object already written, as when a retry follows a lost
/// success response, succeeds. The upload token can't overwrite, so writes are
/// create-only and the store counts GCS's 412 as done.
#[tokio::test]
async fn put_of_an_existing_object_succeeds() {
    let Some(gcs) = Gcs::load() else { return };
    for (name, len) in [("exists-single.bin", SMALL), ("exists-multi.bin", LARGE)] {
        let obj = gcs.object(name);
        let bytes = test_bytes(len);
        let file = temp_file_with(&bytes).await;

        for _ in 0..2 {
            let store = gcs.upload_store(&obj).await;
            store
                .put(file.to_file(), &obj, &metadata())
                .await
                .expect(name);
        }
        gcs.assert_stored(&obj, &bytes).await;
    }
}
