//! Google Cloud Storage through its S3-compatible XML API.
//!
//! GCS uploads use the same [`Store`](crate::s3::Store) as S3, pointed at
//! `storage.googleapis.com` and authenticated with the backend's downscoped
//! OAuth2 bearer token instead of SigV4. What differs from S3 lives here:
//!
//! - [`GcsInterceptor`] sets `Authorization: Bearer <token>` on every attempt,
//!   renames `x-amz-meta-*` headers to `x-goog-meta-*` (GCS silently drops the
//!   AWS form under bearer auth), and makes object writes create-only.
//! - [`TooManyRequests`] retries every HTTP 429. GCS's 429 codes
//!   (`TooManyRequests`, `RateLimitExceeded`) are not in the SDK's throttling
//!   list, so without it they get a single attempt.
//! - [`config`] turns the SDK's default request checksums off. For a streamed
//!   body the default is `aws-chunked` with a trailing checksum, which GCS
//!   stores as object bytes while still answering 200. Integrity comes from the
//!   `Content-MD5` the store sets instead (GCS rejects a mismatch with 400
//!   `BadDigest`).
//!
//! These rely on how GCS treats aws-sdk-s3's requests, which Google doesn't
//! document. The tests below pin the requests we send, and an aws-sdk-s3 bump
//! needs checking against real GCS before it ships (ENG-1672).

// standard crates
use std::fmt;

// internal crates
use crate::errors::HTTPCode;
use crate::s3::sdk;

// external crates
use aws_sdk_s3::config::interceptors::{BeforeTransmitInterceptorContextMut, InterceptorContext};
use aws_sdk_s3::config::retry::{ClassifyRetry, RetryAction};
use aws_sdk_s3::config::{
    ConfigBag, Intercept, Region, RequestChecksumCalculation, ResponseChecksumValidation,
    RuntimeComponents, SharedHttpClient,
};
use aws_sdk_s3::error::BoxError;
use aws_smithy_runtime_api::client::orchestrator::Metadata;
use aws_smithy_runtime_api::http::Headers;
use base64::Engine;
use md5::{Digest, Md5};
use secrecy::{ExposeSecret, SecretString};

/// GCS's XML API endpoint.
const ENDPOINT: &str = "https://storage.googleapis.com";

const AMZ_META: &str = "x-amz-meta-";
const GOOG_META: &str = "x-goog-meta-";

/// The operations that write an object, and so are sent create-only.
const OBJECT_WRITES: [&str; 2] = ["PutObject", "CompleteMultipartUpload"];

/// The backend's OAuth2 access token for one upload.
#[derive(Debug)]
pub struct Token(SecretString);

impl Token {
    /// `None` unless the token is non-empty printable ASCII, so it can be sent
    /// in a header. Checked here because the interceptor's own failure reaches
    /// callers as a connection error, which the uploader would retry without
    /// counting.
    pub fn new(token: impl Into<String>) -> Option<Self> {
        let token = token.into();
        let is_printable = token.bytes().all(|b| b.is_ascii_graphic() || b == b' ');
        (!token.is_empty() && is_printable).then(|| Self(SecretString::from(token)))
    }
}

/// Where [`GcsInterceptor`] gets the token for each request.
pub trait TokenSource: Send + Sync + fmt::Debug {
    /// The token to send now, if there is one.
    fn token(&self) -> Option<Token>;
}

/// A fixed token, for tests.
#[cfg(test)]
impl TokenSource for Token {
    fn token(&self) -> Option<Token> {
        Some(Token(self.0.clone()))
    }
}

/// The SDK config for a GCS [`Store`](crate::s3::Store).
pub(crate) fn config(
    token_source: impl TokenSource + 'static,
    http_client: SharedHttpClient,
) -> aws_sdk_s3::Config {
    sdk::base_config(http_client)
        .endpoint_url(ENDPOINT)
        .region(Region::new("auto"))
        .force_path_style(true)
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
        // No credentials provider: the SDK signs nothing and the interceptor
        // adds the bearer header.
        .allow_no_auth()
        .interceptor(GcsInterceptor { token_source })
        .retry_classifier(TooManyRequests)
        .build()
}

/// Base64 MD5 of `bytes`, for the `Content-MD5` header.
pub(crate) fn content_md5(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(bytes))
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
pub(crate) struct GcsInterceptor<T> {
    token_source: T,
}

impl<T: TokenSource> Intercept for GcsInterceptor<T> {
    fn name(&self) -> &'static str {
        "GcsInterceptor"
    }

    fn modify_before_transmit(
        &self,
        context: &mut BeforeTransmitInterceptorContextMut<'_>,
        _runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let headers = context.request_mut().headers_mut();
        if is_object_write(cfg) {
            set_create_only(headers);
        }
        rename_amz_metadata(headers)?;
        let token = self.token_source.token().ok_or("no GCS bearer token")?;
        set_bearer(headers, &token)
    }
}

/// Whether this attempt is one of [`OBJECT_WRITES`].
fn is_object_write(cfg: &ConfigBag) -> bool {
    let operation = cfg.load::<Metadata>().map(Metadata::name);
    operation.is_some_and(|op| OBJECT_WRITES.contains(&op))
}

/// Makes GCS write the object only if none exists at its key yet.
fn set_create_only(headers: &mut Headers) {
    headers.insert("x-goog-if-generation-match", "0");
}

/// Renames each `x-amz-meta-*` header to `x-goog-meta-*`.
fn rename_amz_metadata(headers: &mut Headers) -> Result<(), BoxError> {
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
    Ok(())
}

/// Sets `Authorization: Bearer <token>`.
fn set_bearer(headers: &mut Headers, token: &Token) -> Result<(), BoxError> {
    let bearer = format!("Bearer {}", token.0.expose_secret());
    // The error names no value, so the token can't reach logs through it.
    headers
        .try_insert("authorization", bearer)
        .map_err(|_| BoxError::from("GCS bearer token is not a valid header value"))?;
    Ok(())
}

/// Retries any HTTP 429 as throttling, whatever error code GCS gives it.
#[derive(Debug)]
pub(crate) struct TooManyRequests;

impl ClassifyRetry for TooManyRequests {
    fn classify_retry(&self, ctx: &InterceptorContext) -> RetryAction {
        match ctx.response() {
            Some(resp) if resp.status().as_u16() == HTTPCode::TOO_MANY_REQUESTS.as_u16() => {
                RetryAction::throttling_error()
            }
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

    // internal crates
    use super::*;
    use crate::s3::store::tests::{resp, resp_xml, temp_file_with};
    use crate::s3::store::PART_SIZE;
    use crate::s3::{Object, Options, S3Err, Store};

    // external crates
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
        let token = Token::new("gcs-token").unwrap();
        let http_client = SharedHttpClient::new(replay.clone());
        Store::new(Options {
            config: config(token, http_client),
            gcs: true,
        })
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

    /// A PutObject of `small.bin` that succeeds.
    fn put_ok() -> Vec<ReplayEvent> {
        let put = req("PUT", "small.bin?x-id=PutObject");
        vec![ReplayEvent::new(put, resp(200, &[]))]
    }

    /// A two-part multipart upload of `big.bin` that succeeds.
    fn multipart_ok() -> Vec<ReplayEvent> {
        vec![
            ReplayEvent::new(req("POST", "big.bin?uploads"), create_resp()),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e1\"")),
            ReplayEvent::new(req("PUT", "big.bin"), part_resp("\"e2\"")),
            ReplayEvent::new(req("POST", "big.bin"), complete_resp()),
        ]
    }

    /// Puts a small file (one PutObject) and then a big one (a two-part
    /// multipart upload) through a GCS store, and returns the client that
    /// recorded the requests.
    async fn put_small_and_big(metadata: &HashMap<String, String>) -> StaticReplayClient {
        let mut events = put_ok();
        events.extend(multipart_ok());
        let replay = StaticReplayClient::new(events);
        let store = gcs_store(&replay);
        let small = temp_file_with(b"hello world").await;
        let big = temp_file_with(&vec![3u8; PART_SIZE as usize + 1]).await;
        store
            .put(small.file().clone(), &obj("small.bin"), metadata)
            .await
            .unwrap();
        store
            .put(big.file().clone(), &obj("big.bin"), metadata)
            .await
            .unwrap();
        replay
    }

    mod token {
        use super::*;

        #[test]
        fn token_rejects_empty_and_unprintable() {
            assert!(Token::new("bad\ntoken").is_none());
            assert!(Token::new("").is_none());
            assert!(Token::new("ya29.a0-_~+/=").is_some());
        }

        #[test]
        fn token_debug_redacts() {
            let token = Token::new("secret-token").unwrap();
            assert!(!format!("{token:?}").contains("secret-token"));
            let interceptor = GcsInterceptor {
                token_source: token,
            };
            assert!(!format!("{interceptor:?}").contains("secret-token"));
        }
    }

    mod config {
        use super::*;

        #[tokio::test]
        async fn requests_go_to_gcs_without_sigv4_or_sdk_checksums() {
            let replay = StaticReplayClient::new(put_ok());
            let bytes = vec![7u8; 64 * 1024];
            let src = temp_file_with(&bytes).await;

            gcs_store(&replay)
                .put(src.file().clone(), &obj("small.bin"), &HashMap::new())
                .await
                .unwrap();

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].uri().to_string(),
                uri("small.bin?x-id=PutObject")
            );
            let headers = requests[0].headers();
            // No SigV4 and no SDK checksums, which GCS doesn't take.
            assert_eq!(headers.get("x-amz-date"), None);
            assert_eq!(headers.get("content-encoding"), None);
            assert!(
                !headers.iter().any(|(k, _)| k.starts_with("x-amz-checksum")),
                "SDK checksum header sent to GCS"
            );
            assert_eq!(requests[0].body().bytes(), Some(&bytes[..]));
        }
    }

    mod content_md5 {
        use super::*;

        #[test]
        fn content_md5_is_base64_digest() {
            assert_eq!(content_md5(b""), "1B2M2Y8AsgTpgAmY7PhCfg==");
            assert_eq!(content_md5(b"hello world"), "XrY7u+Ae7tCTyyK7j1rNww==");
        }

        #[tokio::test]
        async fn put_singlepart_sends_md5() {
            let replay = StaticReplayClient::new(put_ok());
            let bytes = vec![7u8; 64 * 1024];
            let src = temp_file_with(&bytes).await;

            gcs_store(&replay)
                .put(src.file().clone(), &obj("small.bin"), &HashMap::new())
                .await
                .unwrap();

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(
                requests[0].headers().get("content-md5"),
                Some(content_md5(&bytes).as_str())
            );
        }

        #[tokio::test]
        async fn put_multipart_sends_md5_per_part() {
            let replay = StaticReplayClient::new(multipart_ok());
            let bytes: Vec<u8> = (0..PART_SIZE + 1024).map(|i| (i % 251) as u8).collect();
            let src = temp_file_with(&bytes).await;

            gcs_store(&replay)
                .put(src.file().clone(), &obj("big.bin"), &HashMap::new())
                .await
                .unwrap();

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 4);
            let split = PART_SIZE as usize;
            for (part, slice) in [
                (&requests[1], &bytes[..split]),
                (&requests[2], &bytes[split..]),
            ] {
                assert_eq!(
                    part.headers().get("content-md5"),
                    Some(content_md5(slice).as_str())
                );
                assert_eq!(part.body().bytes(), Some(slice));
            }
        }
    }

    mod interceptor {
        use super::*;

        #[tokio::test]
        async fn writes_are_create_only() {
            let replay = put_small_and_big(&HashMap::new()).await;

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
        async fn renames_amz_metadata_to_goog() {
            let replay = put_small_and_big(&metadata()).await;

            // The PutObject and the multipart create carry the metadata.
            let requests = replay.actual_requests().collect::<Vec<_>>();
            for r in [requests[0], requests[1]] {
                assert_eq!(r.headers().get("x-goog-meta-device_id"), Some("dvc_1"));
                assert_eq!(r.headers().get("x-amz-meta-device_id"), None);
            }
        }

        #[tokio::test]
        async fn sets_bearer_on_every_request() {
            let replay = put_small_and_big(&HashMap::new()).await;

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 5);
            for r in &requests {
                assert_eq!(r.headers().get("authorization"), Some("Bearer gcs-token"));
            }
        }

        #[tokio::test]
        async fn unsendable_token_fails_before_sending() {
            // Token::new refuses this; built directly to check the interceptor's
            // own failure.
            let replay = StaticReplayClient::new(vec![]);
            let token = Token(SecretString::from("bad\ntoken"));
            let http_client = SharedHttpClient::new(replay.clone());
            let store = Store::new(Options {
                config: config(token, http_client),
                gcs: true,
            });
            let src = temp_file_with(b"hello world").await;

            let err = store
                .put(src.file().clone(), &obj("small.bin"), &HashMap::new())
                .await
                .unwrap_err();

            assert!(!format!("{err:?}").contains("bad\ntoken"), "token leaked");
            // A network error, which the uploader would retry without counting.
            assert!(matches!(err, S3Err::ConnectionErr(_)));
            assert_eq!(replay.actual_requests().count(), 0);
        }
    }

    mod too_many_requests {
        use super::*;

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
    }

    // How the store reads GCS's answers to an upload.
    mod put {
        use super::*;

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
            // 403 means the token or bucket refused the write, not that the object
            // exists.
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
    }
}
