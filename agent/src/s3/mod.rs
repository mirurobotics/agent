// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::{file::File, files, path::PathExt};
use crate::trace;

// external crates
use aws_sdk_s3::config::{BehaviorVersion, Credentials as AwsCredentials, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use tokio::io::AsyncWriteExt;

pub mod errors;
pub mod multipart;

use errors::ObjectNotFoundErr;
pub use errors::S3Err;
pub use multipart::Source;

/// Objects larger than this stream through a multipart upload; objects at or
/// below it go through a single `PutObject`. S3's own multipart part-size
/// floor is 5 MiB; 8 MiB gives headroom while keeping part counts small.
pub(crate) const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

pub struct Config {
    pub creds: Credentials,
    pub region: String,
}

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub bucket: String,
    pub key: String,
}

impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s3://{}/{}", self.bucket, self.key)
    }
}

pub struct Store {
    client: Client,
}

impl Store {
    /// Builds a client from caller-supplied temporary credentials. No network
    /// I/O happens here; the first request is made lazily on the first call.
    pub fn new(cfg: Config) -> Self {
        Self::build(cfg, None, false)
    }

    pub(crate) fn build(
        cfg: Config,
        http_client: Option<aws_sdk_s3::config::SharedHttpClient>,
        force_path_style: bool,
    ) -> Self {
        let s3creds = AwsCredentials::new(
            cfg.creds.access_key_id,
            cfg.creds.secret_access_key,
            Some(cfg.creds.session_token),
            None,
            "miru-agent",
        );
        let mut builder = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region))
            .credentials_provider(s3creds)
            .force_path_style(force_path_style);
        if let Some(http_client) = http_client {
            builder = builder.http_client(http_client);
        }
        Self {
            client: Client::from_conf(builder.build()),
        }
    }

    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) in place of the default HTTPS connector, so tests
    /// serve canned responses without touching the network. Requests use the
    /// supplied credentials and path-style URLs for deterministic replay paths.
    #[cfg(test)]
    pub(crate) fn from_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
        cfg: Config,
    ) -> Self {
        Self::build(
            cfg,
            Some(aws_sdk_s3::config::IntoShared::into_shared(http_client)),
            true,
        )
    }

    /// Creates or overwrites an object by streaming a file off disk.
    ///
    /// The whole file is never held in memory: files at or below [`PART_SIZE`]
    /// stream through one `PutObject` ([`Self::put_singlepart`]); larger files
    /// stream part-by-part through a stateless multipart upload
    /// ([`Self::put_multipart`]).
    pub async fn put(
        &self,
        src: File,
        dst: &Object,
        metadata: &HashMap<String, String>,
    ) -> Result<(), S3Err> {
        let size = files::size(&src).await?;
        if size > PART_SIZE {
            self.put_multipart(&multipart::Source { file: src, size }, dst, metadata)
                .await
        } else {
            self.put_singlepart(&src, dst, metadata).await
        }
    }

    /// Streams a file to S3 as a single-part upload.
    pub async fn put_singlepart(
        &self,
        src: &File,
        dst: &Object,
        metadata: &HashMap<String, String>,
    ) -> Result<(), S3Err> {
        let body = ByteStream::from_path(src.path())
            .await
            .map_err(|e| errors::map_bytestream_err("put_object", dst, src, &e))?;
        self.client
            .put_object()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .body(body)
            .set_metadata((!metadata.is_empty()).then(|| metadata.clone()))
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("put_object", dst, e))?;
        Ok(())
    }

    /// Streams an object's body to a destination file. A missing object maps to
    /// [`S3Err::ObjectNotFoundErr`]. The body is copied through a bounded buffer rather
    /// than collected into memory.
    pub async fn get(&self, src: &Object, dest: &File) -> Result<(), S3Err> {
        let output = match self
            .client
            .get_object()
            .bucket(&src.bucket)
            .key(&src.key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(err) => {
                if errors::is_not_found(&err) {
                    return Err(S3Err::ObjectNotFoundErr(ObjectNotFoundErr {
                        object: src.clone(),
                        trace: trace!(),
                    }));
                }
                return Err(errors::map_sdk_err("get_object", src, err));
            }
        };

        // Stream the body straight to `dest`, chunk by chunk, so a body-read
        // failure (a retryable transport error) is classified distinctly from a
        // local write failure. `File::create` truncates any existing file. On
        // failure a partially-written `dest` may remain; cleaning that up is the
        // caller's responsibility.
        let mut body = output.body;
        let file = tokio::fs::File::create(dest.path())
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        // Every write on an unbuffered tokio File dispatches a blocking task —
        // buffer so large downloads don't pay one dispatch per body chunk.
        let mut writer = tokio::io::BufWriter::with_capacity(512 * 1024, file);
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| errors::map_body_read_err("get_object", src, &e))?;
            writer
                .write_all(&chunk)
                .await
                .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        }
        writer
            .flush()
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        let file = writer.into_inner();
        file.sync_data()
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        Ok(())
    }

    /// Deletes an object. Idempotent per S3 semantics (deleting a missing key still
    /// returns success).
    pub async fn delete(&self, obj: &Object) -> Result<(), S3Err> {
        self.client
            .delete_object()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("delete_object", obj, e))?;
        Ok(())
    }

    /// Returns `true` if the object exists (HEAD 200), `false` on a 404. Other errors
    /// propagate.
    pub async fn exists(&self, obj: &Object) -> Result<bool, S3Err> {
        match self
            .client
            .head_object()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                if errors::is_not_found(&err) {
                    Ok(false)
                } else {
                    Err(errors::map_sdk_err("head_object", obj, err))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::HashMap;

    // internal crates
    use crate::test_utils::{
        error_harnesses::{assert_error, Expected},
        filesys::{abs_file, files as test_files, missing_file},
    };
    use miru_agent::errors::{Code, Error, HTTPCode};
    use miru_agent::filesys::path::PathExt;
    use miru_agent::filesys::{files, WriteOptions};
    use miru_agent::s3::{Config, Credentials, Object, S3Err, Store};

    // external crates
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    const REGION: &str = "us-east-1";
    const BUCKET: &str = "test-bucket";

    fn test_credentials() -> Credentials {
        Credentials {
            access_key_id: "access-key".to_string(),
            secret_access_key: "secret-key".to_string(),
            session_token: "session-token".to_string(),
        }
    }

    // Headers that vary per request (signing, timestamps, user agent) and must be
    // excluded from request matching.
    const IGNORED_HEADERS: &[&str] = &[
        "authorization",
        "x-amz-date",
        "x-amz-content-sha256",
        "x-amz-user-agent",
        "user-agent",
        "x-amz-security-token",
        "amz-sdk-request",
        "amz-sdk-invocation-id",
        "content-type",
        "content-length",
    ];

    /// Builds a path-style S3 URI for the test bucket so replayed request URIs are
    /// deterministic (`https://s3.<region>.amazonaws.com/<bucket>/<key>...`).
    pub(super) fn uri(path_and_query: &str) -> String {
        format!("https://s3.{REGION}.amazonaws.com/{BUCKET}/{path_and_query}")
    }

    /// Builds an [`Object`] in the test bucket for the given key, keeping call sites
    /// terse.
    pub(super) fn obj(key: &str) -> Object {
        Object {
            bucket: BUCKET.to_string(),
            key: key.to_string(),
        }
    }

    /// Wires a `Store` to a `StaticReplayClient` serving the given events, so the
    /// client's requests are matched against canned responses without touching the
    /// network.
    pub(super) fn store_with(events: Vec<ReplayEvent>) -> (Store, StaticReplayClient) {
        let replay = StaticReplayClient::new(events);
        let cfg = Config {
            region: REGION.to_string(),
            creds: test_credentials(),
        };
        let store = Store::from_http_client(replay.clone(), cfg);
        (store, replay)
    }

    /// Writes `bytes` to a fresh temp file and returns the guard (kept alive so
    /// the file is not deleted until the test drops it).
    pub(super) async fn temp_file_with(bytes: &[u8]) -> test_files::TempFile {
        let tf = test_files::temp("s3-test").unwrap();
        files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        tf
    }

    /// Builds an expected request (method + path-style URI, empty body) to match a
    /// `ReplayEvent` against. Only the method and key/query are usually meaningful.
    pub(super) fn req(method: &str, path_and_query: &str) -> http::Request<SdkBody> {
        http::Request::builder()
            .method(method)
            .uri(uri(path_and_query))
            .body(SdkBody::empty())
            .unwrap()
    }

    /// Builds a canned response with `status` and `body`, setting `content-length`
    /// to match. Pass `&[]` for a bodyless response (200 OK, 204, HEAD, ...).
    pub(super) fn resp(status: u16, body: &[u8]) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(status)
            .header("content-length", body.len().to_string())
            .body(SdkBody::from(body.to_vec()))
            .unwrap()
    }

    /// Builds a canned S3 error response carrying an XML `<Error>` document, as S3
    /// returns for failures like `NoSuchKey` or `AccessDenied`.
    pub(super) fn resp_xml(status: u16, xml: &str) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(status)
            .header("content-type", "application/xml")
            .body(SdkBody::from(xml.to_string()))
            .unwrap()
    }

    const ACCESS_DENIED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message><RequestId>REQ403</RequestId></Error>"#;

    /// Canned 403 AccessDenied response. S3 returns non-404 failures as a modeled
    /// service error, which the common mapper turns into `RequestFailedErr`.
    pub(super) fn access_denied_resp() -> http::Response<SdkBody> {
        resp_xml(403, ACCESS_DENIED_XML)
    }

    /// Wires a `Store` to a replay client expecting exactly one request/response
    /// exchange — the shape of nearly every test here.
    pub(super) fn store_expecting(
        request: http::Request<SdkBody>,
        response: http::Response<SdkBody>,
    ) -> (Store, StaticReplayClient) {
        store_with(vec![ReplayEvent::new(request, response)])
    }

    /// Stable `(method, uri)` view of a request. Bodies and signing headers are
    /// intentionally dropped so sequence asserts stay comparable.
    pub(super) fn shape(method: &str, path_and_query: &str) -> (String, String) {
        (method.to_string(), uri(path_and_query))
    }

    /// Projects every request the replay client observed into [`shape`]s, in order.
    pub(super) fn actual_shapes(replay: &StaticReplayClient) -> Vec<(String, String)> {
        replay
            .actual_requests()
            .map(|r| (r.method().to_string(), r.uri().to_string()))
            .collect()
    }

    pub mod construction {
        use super::*;

        #[tokio::test]
        async fn new_builds_without_network() {
            let cfg = Config {
                creds: Credentials {
                    access_key_id: "AKIA_TEST".to_string(),
                    secret_access_key: "secret".to_string(),
                    session_token: "session".to_string(),
                },
                region: "us-west-2".to_string(),
            };
            // Constructing must not panic or touch the network.
            let _store = Store::new(cfg);
        }
    }

    pub mod put {
        use super::*;

        pub mod single {
            use super::*;

            #[tokio::test]
            async fn put_streams_file_body_bytes() {
                let key = "artifacts/hello.txt";
                let src = temp_file_with(b"hello world").await;
                // The body streams off disk, so the replay client records it as an
                // unbuffered `SdkBody` whose `.bytes()` is `None` — `assert_requests_match`
                // can't byte-compare it, so we assert the method + path by hand instead.
                // `get_streams_body_to_file` covers a real round-trip of the payload.
                let (store, replay) = store_expecting(
                    req("PUT", "artifacts/hello.txt?x-id=PutObject"),
                    resp(200, &[]),
                );

                store
                    .put_singlepart(src.file(), &obj(key), &HashMap::new())
                    .await
                    .unwrap();

                let requests = replay.actual_requests().collect::<Vec<_>>();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method(), "PUT");
                assert_eq!(
                    requests[0].uri().to_string(),
                    uri("artifacts/hello.txt?x-id=PutObject")
                );
                assert!(requests[0]
                    .headers()
                    .get("authorization")
                    .unwrap()
                    .contains("Credential=access-key/"));
                assert_eq!(
                    Some("session-token"),
                    requests[0].headers().get("x-amz-security-token")
                );
            }

            #[tokio::test]
            async fn put_empty_file_succeeds() {
                let key = "artifacts/empty.txt";
                // A 0-byte source must still produce exactly one PUT — an empty body
                // is a valid object, not a skipped request.
                let src = temp_file_with(b"").await;
                let (store, replay) = store_expecting(
                    req("PUT", "artifacts/empty.txt?x-id=PutObject"),
                    resp(200, &[]),
                );

                store
                    .put_singlepart(src.file(), &obj(key), &HashMap::new())
                    .await
                    .unwrap();

                // The streamed body can't be byte-compared, so assert the method +
                // path by hand (see `put_streams_file_body_bytes`).
                let requests = replay.actual_requests().collect::<Vec<_>>();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method(), "PUT");
                assert_eq!(
                    requests[0].uri().to_string(),
                    uri("artifacts/empty.txt?x-id=PutObject")
                );
            }

            #[tokio::test]
            async fn put_stamps_metadata_headers() {
                let key = "artifacts/hello.txt";
                let src = temp_file_with(b"hello world").await;
                let metadata = HashMap::from([
                    ("device_id".to_string(), "dvc_1".to_string()),
                    ("digest".to_string(), "sha256:abc".to_string()),
                ]);
                let (store, replay) = store_expecting(
                    req("PUT", "artifacts/hello.txt?x-id=PutObject"),
                    resp(200, &[]),
                );

                store
                    .put_singlepart(src.file(), &obj(key), &metadata)
                    .await
                    .unwrap();

                let requests = replay.actual_requests().collect::<Vec<_>>();
                let header = |name: &str| requests[0].headers().get(name);
                assert_eq!(header("x-amz-meta-device_id"), Some("dvc_1"));
                assert_eq!(header("x-amz-meta-digest"), Some("sha256:abc"));
            }
        }

        pub mod access_denied {
            use super::*;

            #[tokio::test]
            async fn put_403_maps_to_request_failed() {
                let src = temp_file_with(b"x").await;
                let (store, _replay) = store_expecting(
                    req("PUT", "denied.txt?x-id=PutObject"),
                    access_denied_resp(),
                );

                let err = store
                    .put(src.to_file(), &obj("denied.txt"), &HashMap::new())
                    .await
                    .unwrap_err();

                let S3Err::RequestFailedErr(inner) = &err else {
                    panic!("expected RequestFailedErr, got {err:?}");
                };
                assert_eq!(
                    (inner.operation.as_str(), inner.status, &inner.object),
                    ("put_object", Some(403), &obj("denied.txt"))
                );
                assert_error(
                    &err,
                    Expected::new(Code::InternalServerError, HTTPCode::INTERNAL_SERVER_ERROR),
                );
            }
        }

        pub mod source_missing {
            use super::*;

            #[tokio::test]
            async fn put_missing_source_maps_to_filesys_err() {
                // A missing LOCAL source surfaces as `FileSysErr`: `put` stats the
                // file first (to route by size), so the failure is caught reading the
                // file's metadata before any request is dispatched.
                let (store, _replay) = store_with(vec![]);
                let missing = missing_file();

                let err = store
                    .put(missing, &obj("k"), &HashMap::new())
                    .await
                    .unwrap_err();

                assert!(matches!(err, S3Err::FileSysErr(_)));
            }
        }
    }

    pub mod get {
        use super::*;

        pub mod success {
            use super::*;

            #[tokio::test]
            async fn get_streams_body_to_file() {
                let key = "blobs/data.bin";
                let payload = b"\x00\x01\x02binary-body\xff".to_vec();
                let dest = test_files::temp("s3-dest").unwrap();
                let (store, replay) = store_expecting(
                    req("GET", "blobs/data.bin?x-id=GetObject"),
                    resp(200, &payload),
                );

                store.get(&obj(key), dest.file()).await.unwrap();

                assert_eq!(files::read_bytes(dest.file()).await.unwrap(), payload);
                replay.assert_requests_match(IGNORED_HEADERS);
            }

            #[tokio::test]
            async fn get_overwrites_existing_destination() {
                let key = "blobs/data.bin";
                // Pre-write stale content that is LONGER than the new payload, so an
                // accidental append (or a failure to truncate) would leave trailing
                // bytes and be caught by the exact-equality assertion below.
                let dest = test_files::temp("s3-dest").unwrap();
                files::write_bytes(
                    dest.file(),
                    b"OLD-STALE-CONTENT",
                    WriteOptions::OVERWRITE_NONATOMIC,
                )
                .await
                .unwrap();
                let new_payload = b"NEW".to_vec();
                let (store, replay) = store_expecting(
                    req("GET", "blobs/data.bin?x-id=GetObject"),
                    resp(200, &new_payload),
                );

                store.get(&obj(key), dest.file()).await.unwrap();

                assert_eq!(files::read_bytes(dest.file()).await.unwrap(), new_payload);
                replay.assert_requests_match(IGNORED_HEADERS);
            }

            #[tokio::test]
            async fn get_empty_object_writes_empty_file() {
                let key = "blobs/empty.bin";
                // An empty object body is a success: a 0-byte file is written.
                let dest = test_files::temp("s3-dest").unwrap();
                let (store, replay) =
                    store_expecting(req("GET", "blobs/empty.bin?x-id=GetObject"), resp(200, &[]));

                store.get(&obj(key), dest.file()).await.unwrap();

                assert!(files::read_bytes(dest.file()).await.unwrap().is_empty());
                replay.assert_requests_match(IGNORED_HEADERS);
            }
        }

        pub mod dest_unwritable {
            use super::*;

            #[tokio::test]
            async fn get_to_missing_parent_dir_maps_to_local_io_err() {
                let key = "blobs/data.bin";
                // The destination's parent directory does not exist, so creating the
                // file fails after the object is fetched — exercising the streaming
                // I/O error path.
                let dest = abs_file("nonexistent/dir/out.bin");
                let (store, _replay) = store_expecting(
                    req("GET", "blobs/data.bin?x-id=GetObject"),
                    resp(200, b"body"),
                );

                let err = store.get(&obj(key), &dest).await.unwrap_err();

                assert!(matches!(err, S3Err::LocalIoErr(_)));
                // Creating the file failed, so nothing was written at `dest`.
                assert!(!dest.path().exists());
            }
        }

        pub mod truncated_body {
            use super::*;

            /// Builds a 200 response whose `content-length` header OVERSTATES the
            /// actual body bytes (declares 100, delivers ~15), simulating a body
            /// truncated mid-stream on the wire.
            fn truncated_resp() -> http::Response<SdkBody> {
                http::Response::builder()
                    .status(200)
                    .header("content-length", "100")
                    .body(SdkBody::from(&b"short-body-1234"[..]))
                    .unwrap()
            }

            #[tokio::test]
            async fn get_truncated_body_maps_to_connection_err() {
                let key = "blobs/truncated.bin";
                let dest = test_files::temp("s3-dest").unwrap();
                let (store, _replay) = store_expecting(
                    req("GET", "blobs/truncated.bin?x-id=GetObject"),
                    truncated_resp(),
                );

                let err = store.get(&obj(key), dest.file()).await.unwrap_err();

                assert!(matches!(err, S3Err::ConnectionErr(_)));
                assert!(err.is_network_conn_err());
            }
        }

        pub mod not_found {
            use super::*;

            const NO_SUCH_KEY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message><Key>missing.txt</Key><RequestId>REQ123</RequestId></Error>"#;

            #[tokio::test]
            async fn get_missing_maps_to_not_found() {
                let key = "missing.txt";
                let dest = test_files::temp("s3-dest").unwrap();
                let (store, _replay) = store_expecting(
                    req("GET", "missing.txt?x-id=GetObject"),
                    resp_xml(404, NO_SUCH_KEY_XML),
                );

                let err = store.get(&obj(key), dest.file()).await.unwrap_err();

                assert!(matches!(err, S3Err::ObjectNotFoundErr(_)));
                assert_error(
                    &err,
                    Expected::new(Code::ResourceNotFound, HTTPCode::NOT_FOUND),
                );
            }
        }

        pub mod access_denied {
            use super::*;

            #[tokio::test]
            async fn get_403_maps_to_request_failed() {
                let dest = test_files::temp("s3-dest").unwrap();
                let (store, _replay) = store_expecting(
                    req("GET", "denied.txt?x-id=GetObject"),
                    access_denied_resp(),
                );

                let err = store
                    .get(&obj("denied.txt"), dest.file())
                    .await
                    .unwrap_err();

                assert!(matches!(err, S3Err::RequestFailedErr(_)));
            }
        }

        pub mod transport_failure {
            use super::*;

            #[tokio::test]
            async fn transport_failure_maps_to_connection_err() {
                // With no replay events, the connector fails to dispatch the request,
                // which the SDK surfaces as `SdkError::DispatchFailure` — the mapper's
                // network-connection path.
                let dest = test_files::temp("s3-dest").unwrap();
                let (store, _replay) = store_with(vec![]);

                let err = store.get(&obj("any.txt"), dest.file()).await.unwrap_err();

                assert!(matches!(err, S3Err::ConnectionErr(_)));
                assert!(err.is_network_conn_err());
            }
        }
    }

    pub mod delete {
        use super::*;

        pub mod success {
            use super::*;

            #[tokio::test]
            async fn delete_removes_object() {
                let key = "blobs/data.bin";
                let (store, replay) = store_expecting(
                    req("DELETE", "blobs/data.bin?x-id=DeleteObject"),
                    resp(204, &[]),
                );

                store.delete(&obj(key)).await.unwrap();

                replay.assert_requests_match(IGNORED_HEADERS);
            }

            #[tokio::test]
            async fn delete_missing_key_is_idempotent() {
                // S3 returns 204 whether or not the key existed — this documents the
                // module's "delete is idempotent for missing keys" contract.
                let key = "blobs/never-existed.bin";
                let (store, replay) = store_expecting(
                    req("DELETE", "blobs/never-existed.bin?x-id=DeleteObject"),
                    resp(204, &[]),
                );

                store.delete(&obj(key)).await.unwrap();

                replay.assert_requests_match(IGNORED_HEADERS);
            }
        }

        pub mod access_denied {
            use super::*;

            #[tokio::test]
            async fn delete_403_maps_to_request_failed() {
                let (store, _replay) = store_expecting(
                    req("DELETE", "denied.txt?x-id=DeleteObject"),
                    access_denied_resp(),
                );

                let err = store.delete(&obj("denied.txt")).await.unwrap_err();

                assert!(matches!(err, S3Err::RequestFailedErr(_)));
            }
        }
    }

    pub mod exists {
        use super::*;

        pub mod present {
            use super::*;

            #[tokio::test]
            async fn head_200_returns_true() {
                let key = "blobs/data.bin";
                let (store, _replay) =
                    store_expecting(req("HEAD", "blobs/data.bin"), resp(200, &[]));

                assert!(store.exists(&obj(key)).await.unwrap());
            }
        }

        pub mod absent {
            use super::*;

            #[tokio::test]
            async fn head_404_returns_false() {
                let key = "missing.txt";
                let (store, _replay) = store_expecting(req("HEAD", "missing.txt"), resp(404, &[]));

                assert!(!store.exists(&obj(key)).await.unwrap());
            }
        }

        pub mod access_denied {
            use super::*;

            #[tokio::test]
            async fn head_403_propagates_as_request_failed() {
                // HEAD has no response body; a 403 has no XML payload.
                let (store, _replay) = store_expecting(req("HEAD", "denied.txt"), resp(403, &[]));

                let err = store.exists(&obj("denied.txt")).await.unwrap_err();

                assert!(matches!(err, S3Err::RequestFailedErr(_)));
            }
        }
    }
}
