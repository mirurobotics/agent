//! Real-GCS checks for uploads, which go through GCS's XML API with aws-sdk-s3.
//! Google doesn't document how GCS treats the SDK's requests, and the worst
//! break is silent: GCS stores the wrong bytes and still answers 200. So these
//! tests upload the way the agent does, with a token downscoped like the
//! backend's, and read each object back.

// standard crates
use std::collections::HashMap;

// internal crates
use crate::s3::harnesses::{env_var, real_bucket_tests, test_bytes, RealBucket, SMALL};
use miru_agent::s3::gcs::{self, Token};
use miru_agent::s3::{sdk, Object, Options};

// external crates
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use base64::Engine;
use md5::{Digest, Md5};
use serde_json::json;

const GCS_URL: &str = "https://storage.googleapis.com";
const STS_URL: &str = "https://sts.googleapis.com/v1/token";
const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";

/// The test bucket, and a token that can read and write objects in it.
struct Gcs {
    bucket: String,
    source_token: String,
}

impl RealBucket for Gcs {
    fn load() -> Self {
        Self {
            source_token: env_var("GCS_IT_SOURCE_TOKEN"),
            bucket: env_var("GCS_IT_BUCKET"),
        }
    }

    fn name(&self) -> &str {
        &self.bucket
    }

    async fn upload_options(&self, obj: &Object) -> Options {
        let token = Token::new(self.upload_token(obj).await).unwrap();
        let config = gcs::config(token, sdk::http_client());
        Options { config, gcs: true }
    }

    /// Reads with the source token, outside the SDK under test.
    async fn read(&self, obj: &Object) -> (HashMap<String, String>, Vec<u8>) {
        let resp = reqwest::Client::new()
            .get(format!("{GCS_URL}/{}/{}", obj.bucket, obj.key))
            .bearer_auth(&self.source_token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let metadata = resp
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                let key = name.as_str().strip_prefix("x-goog-meta-")?;
                Some((key.to_string(), value.to_str().unwrap().to_string()))
            })
            .collect();
        let stored = resp.bytes().await.unwrap().to_vec();
        (metadata, stored)
    }
}

impl Gcs {
    /// Downscopes the source token to creating `obj` alone, as the backend
    /// does for each upload.
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
}

real_bucket_tests!(Gcs);

/// Base64 MD5 of `bytes`, for a `Content-MD5` header.
fn md5_of(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(bytes))
}

/// GCS rejects a body that doesn't match its Content-MD5. With the SDK's own
/// checksums off, that is what catches a corrupted body.
#[tokio::test]
async fn wrong_content_md5_is_rejected() {
    let gcs = Gcs::load();
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
