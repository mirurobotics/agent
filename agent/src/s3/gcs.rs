//! Google Cloud Storage through its S3-compatible XML API.
//!
//! GCS uploads use the same [`Store`](crate::s3::Store) as S3, pointed at
//! `storage.googleapis.com` and authenticated with the backend's downscoped
//! OAuth2 bearer token instead of SigV4. Verified against real GCS in the
//! ENG-1670 spike. The pieces that differ from S3 live here:
//!
//! - [`GcsInterceptor`] sets `Authorization: Bearer <token>` on every attempt,
//!   renames `x-amz-meta-*` headers to `x-goog-meta-*` (GCS silently drops the
//!   AWS form under bearer auth), and makes object writes create-only.
//! - [`TooManyRequests`] retries every HTTP 429. GCS's 429 codes
//!   (`TooManyRequests`, `RateLimitExceeded`) are not in the SDK's throttling
//!   list, so without it they get a single attempt.
//! - [`config`] turns the SDK's default request checksums off. The default
//!   sends bodies `aws-chunked` with a trailing checksum, which GCS stores as
//!   object bytes and still answers 200. Integrity comes from the per-part
//!   `Content-MD5` the store sets instead (GCS rejects a mismatch with 400
//!   `BadDigest`).
//!
//! These depend on SDK behaviour Google does not document, so an aws-sdk-s3
//! bump needs checking against real GCS before it ships (ENG-1672).

// standard crates
use std::fmt;
use std::sync::Arc;

// external crates
use aws_sdk_s3::config::interceptors::{BeforeTransmitInterceptorContextMut, InterceptorContext};
use aws_sdk_s3::config::retry::{ClassifyRetry, RetryAction};
use aws_sdk_s3::config::{
    ConfigBag, Intercept, Region, RequestChecksumCalculation, ResponseChecksumValidation,
    RuntimeComponents, SharedHttpClient,
};
use aws_sdk_s3::error::BoxError;
use aws_sdk_s3::Client;
use aws_smithy_runtime_api::client::orchestrator::Metadata;
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};

/// GCS's XML API endpoint.
pub const ENDPOINT: &str = "https://storage.googleapis.com";

const AMZ_META: &str = "x-amz-meta-";
const GOOG_META: &str = "x-goog-meta-";

/// The operations that write an object, and so are sent create-only.
const OBJECT_WRITES: [&str; 2] = ["PutObject", "CompleteMultipartUpload"];

/// The current OAuth2 access token for GCS requests. Read synchronously on
/// every request attempt (SDK interceptors can't await), so a refreshing
/// source keeps a current token ready rather than fetching on demand.
pub trait BearerToken: Send + Sync + fmt::Debug {
    fn bearer(&self) -> SecretString;
}

/// A fixed token, for credentials vended once per upload.
pub struct StaticToken(SecretString);

impl StaticToken {
    /// `None` if the token can't be sent in a header. Checked here because the
    /// interceptor's own failure reaches callers as a connection error, which
    /// the uploader would retry without counting.
    pub fn new(token: impl Into<String>) -> Option<Self> {
        let token = token.into();
        let is_valid = token.bytes().all(|b| b.is_ascii_graphic() || b == b' ');
        is_valid.then(|| Self(SecretString::from(token)))
    }
}

impl fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StaticToken([REDACTED])")
    }
}

impl BearerToken for StaticToken {
    fn bearer(&self) -> SecretString {
        self.0.clone()
    }
}

/// The SDK client a GCS [`Store`](crate::s3::Store) uses.
pub fn client(token: Arc<dyn BearerToken>, http_client: SharedHttpClient) -> Client {
    Client::from_conf(config(crate::s3::base_config(http_client), token).build())
}

/// Applies the GCS-specific settings to an S3 config builder that already
/// carries the shared transport, timeouts and stall protection.
fn config(
    builder: aws_sdk_s3::config::Builder,
    token: Arc<dyn BearerToken>,
) -> aws_sdk_s3::config::Builder {
    builder
        .endpoint_url(ENDPOINT)
        .region(Region::new("auto"))
        .force_path_style(true)
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
        // No credentials provider: the SDK signs nothing and the interceptor
        // adds the bearer header.
        .allow_no_auth()
        .interceptor(GcsInterceptor { token })
        .retry_classifier(TooManyRequests)
}

/// Base64 MD5 of `bytes`, for the `Content-MD5` header.
pub fn content_md5(bytes: &[u8]) -> String {
    encode_md5(md5::compute(bytes))
}

/// Base64 of an MD5 digest, as `Content-MD5` carries it.
pub(crate) fn encode_md5(digest: md5::Digest) -> String {
    base64::engine::general_purpose::STANDARD.encode(digest.0)
}

/// Sets the bearer token and GCS metadata header names on each attempt, right
/// before it is sent, and makes object writes create-only
/// (`x-goog-if-generation-match: 0`).
///
/// The upload token has `roles/storage.objectCreator`, which can't overwrite,
/// and can't read the object to check it. Without create-only, a write retried
/// after a lost success response gets the same 403 as a token for the wrong
/// object. With it, GCS answers an existing object with 412 instead, and the
/// store treats that as done: object keys are per upload, GCS only creates the
/// object once the whole upload completes, and every part was MD5-checked.
#[derive(Debug)]
pub(crate) struct GcsInterceptor {
    token: Arc<dyn BearerToken>,
}

impl Intercept for GcsInterceptor {
    fn name(&self) -> &'static str {
        "GcsInterceptor"
    }

    fn modify_before_transmit(
        &self,
        context: &mut BeforeTransmitInterceptorContextMut<'_>,
        _runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let operation = cfg.load::<Metadata>().map(Metadata::name);
        let headers = context.request_mut().headers_mut();
        if operation.is_some_and(|op| OBJECT_WRITES.contains(&op)) {
            headers.insert("x-goog-if-generation-match", "0");
        }
        let renames: Vec<(String, String, String)> = headers
            .iter()
            .filter_map(|(name, value)| {
                let suffix = name.strip_prefix(AMZ_META)?;
                Some((
                    name.to_string(),
                    format!("{GOOG_META}{suffix}"),
                    value.to_string(),
                ))
            })
            .collect();
        for (name, renamed, value) in renames {
            headers.remove(&name);
            headers.try_insert(renamed, value)?;
        }
        let bearer = format!("Bearer {}", self.token.bearer().expose_secret());
        // The error names no value, so the token can't reach logs through it.
        headers
            .try_insert("authorization", bearer)
            .map_err(|_| BoxError::from("GCS bearer token is not a valid header value"))?;
        Ok(())
    }
}

/// Retries any HTTP 429 as throttling, whatever error code GCS gives it.
#[derive(Debug)]
pub(crate) struct TooManyRequests;

impl ClassifyRetry for TooManyRequests {
    fn classify_retry(&self, ctx: &InterceptorContext) -> RetryAction {
        match ctx.response() {
            Some(resp) if resp.status().as_u16() == 429 => RetryAction::throttling_error(),
            _ => RetryAction::NoActionIndicated,
        }
    }

    fn name(&self) -> &'static str {
        "GcsTooManyRequests"
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::HashMap;
    use std::sync::Arc;

    // internal crates
    use super::*;
    use crate::s3::tests::{recorded_payload, resp, resp_xml, temp_file_with};
    use crate::s3::{Object, S3Err, Store, PART_SIZE};

    // external crates
    use aws_sdk_s3::config::IntoShared;
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    const BUCKET: &str = "gcs-bucket";
    const UPLOAD_ID: &str = "gcs-upload-1";

    fn obj(key: &str) -> Object {
        Object {
            bucket: BUCKET.to_string(),
            key: key.to_string(),
        }
    }

    fn uri(path_and_query: &str) -> String {
        format!("{ENDPOINT}/{BUCKET}/{path_and_query}")
    }

    fn req(method: &str, path_and_query: &str) -> http::Request<SdkBody> {
        http::Request::builder()
            .method(method)
            .uri(uri(path_and_query))
            .body(SdkBody::empty())
            .unwrap()
    }

    fn gcs_store(replay: &StaticReplayClient) -> Store {
        let token = Arc::new(StaticToken::new("gcs-token").unwrap());
        Store::new_gcs(token, replay.clone().into_shared())
    }

    fn metadata() -> HashMap<String, String> {
        HashMap::from([("device_id".to_string(), "dvc_1".to_string())])
    }

    fn error_xml(status: u16, code: &str) -> http::Response<SdkBody> {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>{code}</Code><Message>{code}</Message></Error>"#
        );
        resp_xml(status, &xml)
    }

    fn create_resp() -> http::Response<SdkBody> {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><UploadId>{UPLOAD_ID}</UploadId></InitiateMultipartUploadResult>"#
        );
        resp_xml(200, &xml)
    }

    fn part_resp(etag: &str) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(200)
            .header("ETag", etag)
            .header("content-length", "0")
            .body(SdkBody::empty())
            .unwrap()
    }

    fn complete_resp() -> http::Response<SdkBody> {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><ETag>"final"</ETag></CompleteMultipartUploadResult>"#
        );
        resp_xml(200, &xml)
    }

    #[test]
    fn content_md5_is_base64_digest() {
        assert_eq!(content_md5(b""), "1B2M2Y8AsgTpgAmY7PhCfg==");
        assert_eq!(content_md5(b"hello world"), "XrY7u+Ae7tCTyyK7j1rNww==");
    }

    #[test]
    fn static_token_debug_redacts() {
        let token = StaticToken::new("secret-token").unwrap();
        assert!(!format!("{token:?}").contains("secret-token"));
        let interceptor = GcsInterceptor {
            token: Arc::new(token),
        };
        assert!(!format!("{interceptor:?}").contains("secret-token"));
    }

    #[tokio::test]
    async fn put_singlepart_sends_bearer_md5_and_goog_metadata() {
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            req("PUT", "small.bin?x-id=PutObject"),
            resp(200, &[]),
        )]);
        // Over one body frame, so the S3 path would have sent it aws-chunked.
        let bytes = vec![7u8; 64 * 1024];
        let src = temp_file_with(&bytes).await;

        gcs_store(&replay)
            .put(src.file().clone(), &obj("small.bin"), &metadata())
            .await
            .unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        let headers = requests[0].headers();
        assert_eq!(
            requests[0].uri().to_string(),
            uri("small.bin?x-id=PutObject")
        );
        assert_eq!(headers.get("authorization"), Some("Bearer gcs-token"));
        assert_eq!(headers.get("x-goog-meta-device_id"), Some("dvc_1"));
        assert_eq!(headers.get("x-amz-meta-device_id"), None);
        assert_eq!(
            headers.get("content-md5"),
            Some(content_md5(&bytes).as_str())
        );
        assert_eq!(headers.get("content-encoding"), None);
        assert_eq!(headers.get("x-amz-trailer"), None);
        // The wire body is the file's bytes, with no chunk framing.
        assert_eq!(recorded_payload(requests[0]).await, bytes);
    }

    #[tokio::test]
    async fn put_multipart_sends_md5_per_part() {
        let replay = StaticReplayClient::new(vec![
            ReplayEvent::new(req("POST", "big.bin?uploads"), create_resp()),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e1\"")),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e2\"")),
            ReplayEvent::new(req("POST", "big.bin"), complete_resp()),
        ]);
        let bytes: Vec<u8> = (0..PART_SIZE + 1024).map(|i| (i % 251) as u8).collect();
        let src = temp_file_with(&bytes).await;

        gcs_store(&replay)
            .put(src.file().clone(), &obj("big.bin"), &metadata())
            .await
            .unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 4);
        for r in &requests {
            assert_eq!(r.headers().get("authorization"), Some("Bearer gcs-token"));
        }
        // Metadata is set when the upload is created.
        assert_eq!(
            requests[0].headers().get("x-goog-meta-device_id"),
            Some("dvc_1")
        );
        let split = PART_SIZE as usize;
        for (part, slice) in [
            (&requests[1], &bytes[..split]),
            (&requests[2], &bytes[split..]),
        ] {
            assert_eq!(
                part.headers().get("content-md5"),
                Some(content_md5(slice).as_str())
            );
            assert_eq!(part.headers().get("content-encoding"), None);
            assert_eq!(recorded_payload(part).await, slice);
        }
    }

    #[tokio::test]
    async fn writes_are_create_only() {
        let replay = StaticReplayClient::new(vec![
            ReplayEvent::new(req("PUT", "small.bin?x-id=PutObject"), resp(200, &[])),
            ReplayEvent::new(req("POST", "big.bin?uploads"), create_resp()),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e1\"")),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e2\"")),
            ReplayEvent::new(req("POST", "big.bin"), complete_resp()),
        ]);
        let store = gcs_store(&replay);
        let small = temp_file_with(b"hello world").await;
        let big = temp_file_with(&vec![3u8; PART_SIZE as usize + 1]).await;

        store
            .put(small.file().clone(), &obj("small.bin"), &HashMap::new())
            .await
            .unwrap();
        store
            .put(big.file().clone(), &obj("big.bin"), &HashMap::new())
            .await
            .unwrap();

        // PutObject and CompleteMultipartUpload write the object; create and
        // the parts don't.
        let precondition = replay
            .actual_requests()
            .map(|r| {
                r.headers()
                    .get("x-goog-if-generation-match")
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let zero = Some("0".to_string());
        assert_eq!(precondition, vec![zero.clone(), None, None, None, zero]);
    }

    #[tokio::test]
    async fn existing_object_counts_as_uploaded() {
        // A retry after a lost success: the object is already there, so the
        // create-only write gets 412.
        let replay = StaticReplayClient::new(vec![
            ReplayEvent::new(
                req("PUT", "small.bin?x-id=PutObject"),
                error_xml(412, "PreconditionFailed"),
            ),
            ReplayEvent::new(req("POST", "big.bin?uploads"), create_resp()),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e1\"")),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e2\"")),
            ReplayEvent::new(req("POST", "big.bin"), error_xml(412, "PreconditionFailed")),
            ReplayEvent::new(req("DELETE", "big.bin"), resp(204, &[])),
        ]);
        let store = gcs_store(&replay);
        let small = temp_file_with(b"hello world").await;
        let big = temp_file_with(&vec![3u8; PART_SIZE as usize + 1]).await;

        let put = store
            .put(small.file().clone(), &obj("small.bin"), &HashMap::new())
            .await;
        let multipart = store
            .put(big.file().clone(), &obj("big.bin"), &HashMap::new())
            .await;

        assert!(put.is_ok(), "{put:?}");
        assert!(multipart.is_ok(), "{multipart:?}");
        // The earlier upload created the object, so this one's parts are
        // aborted rather than left behind.
        let methods: Vec<_> = replay
            .actual_requests()
            .map(|r| r.method().to_string())
            .collect();
        assert_eq!(methods, ["PUT", "POST", "PUT", "PUT", "POST", "DELETE"]);
    }

    #[tokio::test]
    async fn access_denied_still_fails() {
        // 403 now means the token or bucket refused the write, not that the
        // object exists.
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            req("PUT", "small.bin?x-id=PutObject"),
            error_xml(403, "AccessDenied"),
        )]);
        let src = temp_file_with(b"hello world").await;

        let err = gcs_store(&replay)
            .put(src.file().clone(), &obj("small.bin"), &HashMap::new())
            .await
            .unwrap_err();

        let S3Err::RequestFailedErr(e) = &err else {
            panic!("expected RequestFailedErr, got: {err:?}");
        };
        assert_eq!(e.status, Some(403));
    }

    #[tokio::test]
    async fn too_many_requests_is_retried() {
        let replay = StaticReplayClient::new(vec![
            ReplayEvent::new(
                req("PUT", "small.bin?x-id=PutObject"),
                error_xml(429, "TooManyRequests"),
            ),
            ReplayEvent::new(
                req("PUT", "small.bin?x-id=PutObject"),
                error_xml(429, "RateLimitExceeded"),
            ),
            ReplayEvent::new(req("PUT", "small.bin?x-id=PutObject"), resp(200, &[])),
        ]);
        let src = temp_file_with(b"hello world").await;

        gcs_store(&replay)
            .put(src.file().clone(), &obj("small.bin"), &HashMap::new())
            .await
            .unwrap();

        assert_eq!(replay.actual_requests().count(), 3);
    }

    #[tokio::test]
    async fn complete_no_such_part_is_a_failed_request() {
        // GCS answers a bad part ETag on Complete with 404 NoSuchPart where S3
        // says 400 InvalidPart. It must stay a failed request, not read as the
        // upload or object being gone.
        let replay = StaticReplayClient::new(vec![
            ReplayEvent::new(req("POST", "big.bin?uploads"), create_resp()),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e1\"")),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e2\"")),
            ReplayEvent::new(req("POST", "big.bin"), error_xml(404, "NoSuchPart")),
            ReplayEvent::new(req("DELETE", "big.bin"), resp(204, &[])),
        ]);
        let bytes = vec![1u8; PART_SIZE as usize + 1];
        let src = temp_file_with(&bytes).await;

        let err = gcs_store(&replay)
            .put(src.file().clone(), &obj("big.bin"), &HashMap::new())
            .await
            .unwrap_err();

        let S3Err::RequestFailedErr(e) = &err else {
            panic!("expected RequestFailedErr, got: {err:?}");
        };
        assert_eq!(e.status, Some(404));
        assert_eq!(e.operation, "complete_multipart_upload");
        // The failed upload is aborted.
        let last = replay
            .actual_requests()
            .last()
            .unwrap()
            .method()
            .to_string();
        assert_eq!(last, "DELETE");
    }

    #[test]
    fn static_token_rejects_non_header_values() {
        assert!(StaticToken::new("bad\ntoken").is_none());
        assert!(StaticToken::new("ya29.a0-_~+/=").is_some());
    }

    #[derive(Debug)]
    struct RawToken(&'static str);

    impl BearerToken for RawToken {
        fn bearer(&self) -> SecretString {
            SecretString::from(self.0)
        }
    }

    #[tokio::test]
    async fn invalid_token_fails_before_sending() {
        let replay = StaticReplayClient::new(vec![]);
        let token = Arc::new(RawToken("bad\ntoken"));
        let store = Store::new_gcs(token, replay.clone().into_shared());
        let src = temp_file_with(b"hello world").await;

        let err = store
            .put(src.file().clone(), &obj("small.bin"), &HashMap::new())
            .await
            .unwrap_err();

        assert!(!format!("{err:?}").contains("bad\ntoken"), "token leaked");
        assert_eq!(replay.actual_requests().count(), 0);
    }
}
