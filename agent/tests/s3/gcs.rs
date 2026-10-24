//! Real-GCS guard for uploading through GCS's XML API with aws-sdk-s3.
//!
//! The GCS upload path relies on SDK behaviour Google doesn't document (see
//! `agent/src/s3/gcs.rs`). The worst failure is silent: if an SDK bump turns
//! aws-chunked request checksums back on, GCS stores the chunk framing as
//! object bytes and still answers 200. These tests upload with a token
//! downscoped exactly as the backend vends it, then read the object back with
//! the source token and compare SHA-256 and metadata.
//!
//! Self-skips unless `GCS_IT_SOURCE_TOKEN` (an access token with
//! `roles/storage.objectAdmin` on the bucket) and `GCS_IT_BUCKET` are set. The
//! `gcs-xml` CI workflow sets both and runs on every aws-sdk-s3 change.

// standard crates
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

// internal crates
use miru_agent::filesys::File;
use miru_agent::s3::gcs::{self, content_md5, Token};
use miru_agent::s3::{sdk, Object, Options, S3Err, Source, Store};

// external crates
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::Client;
use sha2::{Digest, Sha256};

/// GCS's XML API, for reading back the metadata headers the SDK doesn't parse.
const GCS_URL: &str = "https://storage.googleapis.com";
const STS_URL: &str = "https://sts.googleapis.com/v1/token";
const MIB: usize = 1024 * 1024;
/// The store's multipart part size (`s3::PART_SIZE`).
const PART: usize = 8 * MIB;

struct Env {
    source_token: String,
    bucket: String,
    prefix: String,
}

impl Env {
    fn load() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let (Some(source_token), Some(bucket)) = (var("GCS_IT_SOURCE_TOKEN"), var("GCS_IT_BUCKET"))
        else {
            eprintln!("skipping: GCS_IT_SOURCE_TOKEN and GCS_IT_BUCKET are not set");
            return None;
        };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let prefix = format!("agent-ci/gcs-xml/{nanos}-{}", std::process::id());
        Some(Self {
            source_token,
            bucket,
            prefix,
        })
    }

    fn obj(&self, name: &str) -> Object {
        Object {
            bucket: self.bucket.clone(),
            key: format!("{}/{name}", self.prefix),
        }
    }

    /// The production GCS client with the source token, for reading back and
    /// cleaning up. Never used for the uploads under test.
    fn admin(&self) -> Client {
        client(self.source_token.clone())
    }

    /// A token for `obj` alone, downscoped like the backend does
    /// (`backend/internal/pkg/gcp/creds/gcs.go`): objectCreator on one object.
    async fn upload_token(&self, obj: &Object) -> String {
        let object = format!("projects/_/buckets/{}/objects/{}", obj.bucket, obj.key);
        let rule = serde_json::json!({
            "availableResource": format!("//storage.googleapis.com/projects/_/buckets/{}", obj.bucket),
            "availablePermissions": ["inRole:roles/storage.objectCreator"],
            "availabilityCondition": {
                "title": "single-object-access",
                "expression": format!("resource.name == '{object}'"),
            },
        });
        let options =
            serde_json::json!({ "accessBoundary": { "accessBoundaryRules": [rule] } }).to_string();
        let form = [
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange",
            ),
            (
                "subject_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            ),
            (
                "requested_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            ),
            ("subject_token", self.source_token.as_str()),
            ("options", options.as_str()),
        ];
        let resp = reqwest::Client::new()
            .post(STS_URL)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_encode(&form))
            .send()
            .await
            .expect("sts request");
        let status = resp.status();
        let json: serde_json::Value = resp.json().await.expect("sts body");
        assert!(status.is_success(), "sts {status}: {}", json["error"]);
        json["access_token"]
            .as_str()
            .expect("access_token")
            .to_string()
    }

    async fn upload_store(&self, obj: &Object) -> Store {
        let token = Token::new(self.upload_token(obj).await).unwrap();
        let config = gcs::config(token, sdk::http_client());
        Store::new(Options { config, gcs: true })
    }

    async fn upload_client(&self, obj: &Object) -> Client {
        client(self.upload_token(obj).await)
    }

    /// Asserts the stored object's SHA-256 matches `expected`.
    async fn assert_bytes(&self, obj: &Object, expected: &[u8]) {
        let out = self
            .admin()
            .get_object()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
            .await
            .unwrap_or_else(|e| panic!("read back {}: {}", obj.key, describe(&e)));
        let remote = out.body.collect().await.unwrap().into_bytes();
        assert_eq!(remote.len(), expected.len(), "{}: size", obj.key);
        assert_eq!(
            hex(&Sha256::digest(remote)),
            hex(&Sha256::digest(expected)),
            "{}: sha256",
            obj.key
        );
    }

    /// Asserts the object carries the test metadata. GCS returns it as
    /// `x-goog-meta-*`, which the S3 SDK doesn't parse, so read the raw header.
    async fn assert_metadata(&self, obj: &Object) {
        let url = format!("{GCS_URL}/{}/{}", obj.bucket, obj.key);
        let resp = reqwest::Client::new()
            .head(url)
            .bearer_auth(&self.source_token)
            .send()
            .await
            .expect("head request");
        assert!(
            resp.status().is_success(),
            "{}: HEAD {}",
            obj.key,
            resp.status()
        );
        let stored = resp
            .headers()
            .get("x-goog-meta-miru-ci")
            .and_then(|v| v.to_str().ok());
        assert_eq!(stored, Some("gcs-xml"), "{}: metadata not stored", obj.key);
    }

    async fn cleanup(&self, objs: &[&Object]) {
        let admin = self.admin();
        for obj in objs {
            let _ = admin
                .delete_object()
                .bucket(&obj.bucket)
                .key(&obj.key)
                .send()
                .await;
        }
    }
}

/// The production GCS client, for the calls `Store` doesn't expose.
fn client(token: String) -> Client {
    let config = gcs::config(Token::new(token).unwrap(), sdk::http_client());
    Client::from_conf(config)
}

fn metadata() -> HashMap<String, String> {
    HashMap::from([("miru-ci".to_string(), "gcs-xml".to_string())])
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    let encode = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Non-repeating bytes, so a dropped or reordered part changes the SHA-256.
fn test_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    file.flush().unwrap();
    file
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn describe<E: ProvideErrorMetadata + std::fmt::Debug>(
    err: &aws_sdk_s3::error::SdkError<E>,
) -> String {
    let status = err.raw_response().map(|r| r.status().as_u16());
    match err.as_service_error() {
        Some(e) => format!("status {status:?} code {:?}", e.code()),
        None => format!("status {status:?}: {err:?}"),
    }
}

fn status_and_code<E: ProvideErrorMetadata>(
    err: &aws_sdk_s3::error::SdkError<E>,
) -> (Option<u16>, Option<String>) {
    (
        err.raw_response().map(|r| r.status().as_u16()),
        err.as_service_error()
            .and_then(|e| e.code())
            .map(str::to_string),
    )
}

/// Single-part and multipart uploads through `Store::put` store the exact
/// bytes and the metadata. 64 KiB is over one body frame, so it is the size
/// the SDK's default checksums would have corrupted.
#[tokio::test]
async fn put_stores_exact_bytes_and_metadata() {
    let Some(env) = Env::load() else { return };
    for (name, len) in [("single.bin", 64 * 1024), ("multi.bin", 2 * PART + 1024)] {
        let obj = env.obj(name);
        let bytes = test_bytes(len, len as u64);
        let tmp = write_temp(&bytes);
        let store = env.upload_store(&obj).await;

        let result = store.put(File::new(tmp.path()), &obj, &metadata()).await;
        assert!(result.is_ok(), "{name}: {result:?}");
        env.assert_bytes(&obj, &bytes).await;
        env.assert_metadata(&obj).await;
        env.cleanup(&[&obj]).await;
    }
}

/// An upload started by one client resumes from ListParts in a new store with
/// a new token, and the result is byte-exact.
#[tokio::test]
async fn resume_lists_landed_parts() {
    let Some(env) = Env::load() else { return };
    let obj = env.obj("resume.bin");
    let bytes = test_bytes(2 * PART + 512, 7);
    let tmp = write_temp(&bytes);

    let first = env.upload_client(&obj).await;
    let created = first
        .create_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .send()
        .await
        .unwrap_or_else(|e| panic!("create: {}", describe(&e)));
    let upload_id = created.upload_id().unwrap().to_string();
    let part1 = &bytes[..PART];
    first
        .upload_part()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(&upload_id)
        .part_number(1)
        .content_md5(content_md5(part1))
        .body(ByteStream::from(part1.to_vec()))
        .send()
        .await
        .unwrap_or_else(|e| panic!("part 1: {}", describe(&e)));

    let store = env.upload_store(&obj).await;
    let src = Source::open(File::new(tmp.path()), &obj).await.unwrap();
    let result = store.resume_multipart_upload(src, &obj, &upload_id).await;
    assert!(result.is_ok(), "resume: {result:?}");
    env.assert_bytes(&obj, &bytes).await;
    env.cleanup(&[&obj]).await;
}

/// GCS rejects a part whose body doesn't match its Content-MD5, so a corrupted
/// part can't be stored silently.
#[tokio::test]
async fn wrong_content_md5_is_rejected() {
    let Some(env) = Env::load() else { return };
    let obj = env.obj("bad-digest.bin");
    let client = env.upload_client(&obj).await;
    let body = test_bytes(64 * 1024, 3);

    let err = client
        .put_object()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .content_md5(content_md5(b"not the body"))
        .body(ByteStream::from(body))
        .send()
        .await
        .expect_err("a wrong Content-MD5 must fail");
    let (status, code) = status_and_code(&err);
    assert_eq!(status, Some(400), "{}", describe(&err));
    assert_eq!(code.as_deref(), Some("BadDigest"), "{}", describe(&err));
}

/// Error mapping the store depends on: a bad part ETag on Complete is a failed
/// request (404 NoSuchPart), and a part for an aborted upload maps to
/// `NoSuchUploadErr` through resume.
#[tokio::test]
async fn bad_part_and_aborted_upload_map_as_expected() {
    let Some(env) = Env::load() else { return };
    let obj = env.obj("errors.bin");
    let client = env.upload_client(&obj).await;
    let create = || {
        client
            .create_multipart_upload()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
    };

    // Bad part: Complete names an ETag that was never uploaded.
    let upload_id = create().await.unwrap().upload_id().unwrap().to_string();
    let bogus = CompletedMultipartUpload::builder()
        .parts(
            CompletedPart::builder()
                .part_number(1)
                .e_tag("\"00000000000000000000000000000000\"")
                .build(),
        )
        .build();
    let err = client
        .complete_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(&upload_id)
        .multipart_upload(bogus)
        .send()
        .await
        .expect_err("completing with a bogus part must fail");
    let (status, code) = status_and_code(&err);
    assert_eq!(status, Some(404), "{}", describe(&err));
    assert_eq!(code.as_deref(), Some("NoSuchPart"), "{}", describe(&err));

    // Aborted upload: resume reports it gone.
    client
        .abort_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(&upload_id)
        .send()
        .await
        .unwrap_or_else(|e| panic!("abort: {}", describe(&e)));
    let tmp = write_temp(&test_bytes(PART + 1, 9));
    let store = env.upload_store(&obj).await;
    let src = Source::open(File::new(tmp.path()), &obj).await.unwrap();
    let err = store
        .resume_multipart_upload(src, &obj, &upload_id)
        .await
        .expect_err("resuming an aborted upload must fail");
    assert!(matches!(err, S3Err::NoSuchUploadErr(_)), "got: {err:?}");
}

/// A retried write after a lost success: the object is already there. The
/// upload token can't overwrite (403), so the store writes create-only, GCS
/// answers 412, and `Store::put` counts that as uploaded, for both single-part
/// and multipart writes.
#[tokio::test]
async fn second_write_to_an_existing_object_succeeds() {
    let Some(env) = Env::load() else { return };

    for (name, len) in [
        ("exists-single.bin", 64 * 1024),
        ("exists-multi.bin", PART + 1024),
    ] {
        let obj = env.obj(name);
        let bytes = test_bytes(len, len as u64 + 1);
        let tmp = write_temp(&bytes);
        for attempt in 1..=2 {
            let store = env.upload_store(&obj).await;
            let result = store.put(File::new(tmp.path()), &obj, &metadata()).await;
            assert!(result.is_ok(), "{name} attempt {attempt}: {result:?}");
        }
        env.assert_bytes(&obj, &bytes).await;
        env.cleanup(&[&obj]).await;
    }
}
