// standard crates
use std::collections::HashMap;
use std::io::SeekFrom;
use std::time::Duration;

// internal crates
use crate::errors::HTTPCode;
use crate::filesys::{errors::PathDoesNotExistErr, file::File, path::PathExt, FileSysErr};
use crate::s3::{body, errors, errors::ObjectNotFoundErr, gcs, S3Err};
use crate::trace;

// external crates
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{SharedHttpClient, StalledStreamProtectionConfig};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::Client;
use aws_smithy_http_client::{proxy, tls, Builder, ConnectorBuilder};
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// Objects larger than this stream through a multipart upload; objects at or
/// below it go through a single `PutObject`. S3's own multipart part-size
/// floor is 5 MiB; 8 MiB gives headroom while keeping part counts small.
pub(crate) const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Time limit for bodyless calls, SDK retries and backoff included.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// PutObject/UploadPart get `BODY_ATTEMPT_FLOOR` plus one second per
/// `MIN_UPLOAD_BYTES_PER_SEC` of body: a tail or response stall after the last
/// frame (which stalled-stream protection no longer watches) ends at this limit.
const BODY_ATTEMPT_FLOOR: Duration = Duration::from_secs(90);
const MIN_UPLOAD_BYTES_PER_SEC: u64 = 64 * 1024;

/// Longest a request body may go without progress.
const STALL_GRACE: Duration = Duration::from_secs(60);

/// Cap for `CompleteMultipartUpload`, which can take minutes on large uploads.
const COMPLETE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(600);

/// Best-effort `AbortMultipartUpload` cleanup.
pub(super) const ABORT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// One limit for both the whole operation and each attempt, so SDK retries
/// cannot stack past it.
fn timeout_config(limit: Duration) -> TimeoutConfig {
    TimeoutConfig::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .operation_timeout(limit)
        .operation_attempt_timeout(limit)
        .build()
}

pub(super) fn body_attempt_timeout(len: u64) -> Duration {
    BODY_ATTEMPT_FLOOR + Duration::from_secs(len.div_ceil(MIN_UPLOAD_BYTES_PER_SEC))
}

/// `CompleteMultipartUpload` shares the attempt budget with the calls before
/// it (create or list, then the parts), so it gets the body budget they left.
pub(super) fn complete_attempt_timeout(size: u64, elapsed: Duration) -> Duration {
    body_attempt_timeout(size)
        .saturating_sub(elapsed)
        .clamp(ATTEMPT_TIMEOUT, COMPLETE_ATTEMPT_TIMEOUT)
}

/// Per-operation config override applying `limit` (see [`timeout_config`]).
pub(super) fn attempt_override(limit: Duration) -> aws_sdk_s3::config::Builder {
    aws_sdk_s3::Config::builder().timeout_config(timeout_config(limit))
}

/// One HTTPS client for every store built from it, so uploads reuse pooled
/// connections instead of a fresh TCP+TLS handshake per upload. Mirrors the
/// SDK default client: rustls with aws-lc, proxy settings from the environment.
pub fn http_client() -> SharedHttpClient {
    Builder::new().build_with_connector_fn(|settings, components| {
        let provider = tls::Provider::Rustls(tls::rustls_provider::CryptoMode::AwsLc);
        let mut conn = ConnectorBuilder::default().tls_provider(provider);
        conn.set_connector_settings(settings.cloned());
        if let Some(components) = components {
            conn.set_sleep_impl(components.sleep_impl());
        }
        conn.set_proxy_config(Some(proxy::ProxyConfig::from_env()));
        conn.build()
    })
}

/// How a successful object write ended.
#[derive(Debug)]
pub(super) enum WriteOutcome {
    Created,
    /// An earlier attempt already stored the object (GCS create-only).
    AlreadyExists,
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

/// A local file opened once for upload. Its size and every byte read come from
/// the same open handle, so the upload stays bound to the opened file even if
/// its path is replaced afterwards.
pub struct Source {
    file: File,
    handle: tokio::fs::File,
    pub(super) size: u64,
}

impl Source {
    /// Opens `file` for reading. A missing file is [`S3Err::FileSysErr`]; any
    /// other open or metadata failure is [`S3Err::LocalIoErr`].
    pub async fn open(file: File, dst: &Object) -> Result<Self, S3Err> {
        let handle = tokio::fs::File::open(file.path()).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
                .into()
            } else {
                errors::map_body_io_err("open_source", dst, &file, e)
            }
        })?;
        let size = handle
            .metadata()
            .await
            .map_err(|e| errors::map_body_io_err("open_source", dst, &file, e))?
            .len();
        Ok(Self { file, handle, size })
    }

    /// Reads `[offset..offset+length]` into memory, mapping any seek/read failure
    /// to [`S3Err::LocalIoErr`]. A file truncated below `offset + length` after
    /// it was opened makes `read_exact` fail with `UnexpectedEof`, which maps to
    /// the same error.
    pub(super) async fn read(
        &mut self,
        operation: &str,
        dst: &Object,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, S3Err> {
        self.handle
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| errors::map_body_io_err(operation, dst, &self.file, e))?;
        let mut buf = vec![0u8; length as usize];
        self.handle
            .read_exact(&mut buf)
            .await
            .map_err(|e| errors::map_body_io_err(operation, dst, &self.file, e))?;
        Ok(buf)
    }
}

/// What a [`Store`] is built from.
pub struct Options {
    /// The SDK config, from [`aws::config`](crate::s3::aws::config) or
    /// [`gcs::config`].
    pub config: aws_sdk_s3::Config,
    /// Set for a [`gcs::config`]. Bodies then carry a `Content-MD5` the store
    /// computes, since the GCS client turns the SDK's own request checksums
    /// off, and object writes are create-only.
    pub gcs: bool,
}

pub struct Store {
    pub(super) client: Client,
    /// See [`Options::gcs`].
    gcs: bool,
}

impl Store {
    /// Builds a store from `opts`, adding the call limits and stalled-stream
    /// protection every store shares. No network I/O happens here; the first
    /// request is made lazily on the first call.
    pub fn new(opts: Options) -> Self {
        let config = opts
            .config
            .to_builder()
            .timeout_config(timeout_config(ATTEMPT_TIMEOUT))
            .stalled_stream_protection(
                StalledStreamProtectionConfig::enabled()
                    .upload_enabled(true)
                    .grace_period(STALL_GRACE)
                    .build(),
            )
            .build();
        Self {
            client: Client::from_conf(config),
            gcs: opts.gcs,
        }
    }

    /// Creates or overwrites an object from a local file, opened once as a
    /// [`Source`]. GCS never overwrites: an existing object counts as written
    /// (see [`gcs`]).
    ///
    /// Files at or below [`PART_SIZE`] go through one `PutObject`
    /// ([`Self::put_singlepart`]); larger files go part-by-part through a
    /// stateless multipart upload ([`Self::put_multipart`]). Either way at most
    /// one part's bytes are held in memory at a time.
    pub async fn put(
        &self,
        src: File,
        dst: &Object,
        metadata: &HashMap<String, String>,
    ) -> Result<(), S3Err> {
        let src = Source::open(src, dst).await?;
        if src.size > PART_SIZE {
            self.put_multipart(src, dst, metadata).await
        } else {
            self.put_singlepart(src, dst, metadata).await
        }
    }

    /// Uploads `src` as a single `PutObject`. The body is buffered in memory, so
    /// the SDK can replay it on a retry without reopening the path, and is
    /// yielded in frames so a stalled transfer is detected.
    pub async fn put_singlepart(
        &self,
        mut src: Source,
        dst: &Object,
        metadata: &HashMap<String, String>,
    ) -> Result<(), S3Err> {
        let size = src.size;
        let bytes = src.read("put_object", dst, 0, size).await?;
        let content_md5 = self.content_md5(&bytes);
        let result = self
            .client
            .put_object()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .body(body::chunked(Bytes::from(bytes)))
            .set_content_md5(content_md5)
            .set_metadata((!metadata.is_empty()).then(|| metadata.clone()))
            .customize()
            .config_override(attempt_override(body_attempt_timeout(size)))
            .send()
            .await
            .map(|_| ());
        self.finish_write("put_object", dst, result).map(|_| ())
    }

    /// `Content-MD5` for a body, when this store sends one.
    pub(super) fn content_md5(&self, bytes: &[u8]) -> Option<String> {
        self.gcs.then(|| gcs::content_md5(bytes))
    }

    /// Finishes an object write (PutObject or CompleteMultipartUpload). On GCS
    /// the client sends writes create-only (see [`gcs`]), so a 412 means an
    /// earlier attempt already stored the object at this key, which counts as
    /// done.
    pub(super) fn finish_write<E>(
        &self,
        operation: &str,
        dst: &Object,
        result: Result<(), SdkError<E>>,
    ) -> Result<WriteOutcome, S3Err>
    where
        E: std::error::Error + 'static,
    {
        match result {
            Ok(()) => Ok(WriteOutcome::Created),
            Err(e) if self.gcs && errors::status(&e) == Some(HTTPCode::PRECONDITION_FAILED) => {
                tracing::info!("upload: {dst} already exists, so an earlier {operation} landed");
                Ok(WriteOutcome::AlreadyExists)
            }
            Err(e) => Err(errors::map_sdk_err(operation, dst, e)),
        }
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
pub(super) mod tests {
    // standard crates
    use std::collections::HashMap;
    use std::time::Duration;

    // internal crates
    use crate::s3::body::chunked;
    use crate::s3::store::{
        body_attempt_timeout, complete_attempt_timeout, http_client, ABORT_ATTEMPT_TIMEOUT,
        PART_SIZE,
    };
    use crate::test_utils::{
        error_harnesses::{assert_error, Expected},
        filesys::{files as test_files, missing_file},
    };
    use miru_agent::data_uploads::upload::UploaderOptions;
    use miru_agent::errors::{Code, Error, HTTPCode};
    use miru_agent::filesys::file::File;
    use miru_agent::filesys::path::PathExt;
    use miru_agent::filesys::{files, FileSysErr, WriteOptions};
    use miru_agent::s3::errors::map_sdk_err;
    use miru_agent::s3::{aws, gcs, Config, Credentials, Object, Options, S3Err, Source, Store};

    // external crates
    use aws_sdk_s3::config::retry::RetryConfig;
    use aws_sdk_s3::config::timeout::TimeoutConfig;
    use aws_sdk_s3::config::{SharedHttpClient, StalledStreamProtectionConfig};
    use aws_smithy_http_client::test_util::{NeverClient, ReplayEvent, StaticReplayClient};
    use aws_smithy_runtime_api::client::http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    };
    use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
    use aws_smithy_runtime_api::client::result::ConnectorError;
    use aws_smithy_runtime_api::client::runtime_components::{
        RuntimeComponents, RuntimeComponentsBuilder,
    };
    use aws_smithy_types::body::SdkBody;
    use bytes::Bytes;
    use http_body_util::BodyExt;
    use tokio::time::Instant;

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
    pub(crate) fn uri(path_and_query: &str) -> String {
        format!("https://s3.{REGION}.amazonaws.com/{BUCKET}/{path_and_query}")
    }

    /// Builds an [`Object`] in the test bucket for the given key, keeping call sites
    /// terse.
    pub(crate) fn obj(key: &str) -> Object {
        Object {
            bucket: BUCKET.to_string(),
            key: key.to_string(),
        }
    }

    fn test_config() -> Config {
        Config {
            region: REGION.to_string(),
            creds: test_credentials(),
        }
    }

    /// Wires an S3 `Store` to `client` in place of the HTTPS connector.
    pub(crate) fn store_on(client: impl HttpClient + 'static) -> Store {
        // Path-style URLs, so request URIs match `uri()`.
        let config = aws::config(test_config(), Some(SharedHttpClient::new(client)), true);
        Store::new(Options { config, gcs: false })
    }

    /// Wires a `Store` to a `StaticReplayClient` serving the given events, so the
    /// client's requests are matched against canned responses without touching the
    /// network.
    pub(crate) fn store_with(events: Vec<ReplayEvent>) -> (Store, StaticReplayClient) {
        let replay = StaticReplayClient::new(events);
        (store_on(replay.clone()), replay)
    }

    /// Writes `bytes` to a fresh temp file and returns the guard (kept alive so
    /// the file is not deleted until the test drops it).
    pub(crate) async fn temp_file_with(bytes: &[u8]) -> test_files::TempFile {
        let tf = test_files::temp("s3-test").unwrap();
        files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        tf
    }

    /// Builds an expected request (method + path-style URI, empty body) to match a
    /// `ReplayEvent` against. Only the method and key/query are usually meaningful.
    pub(crate) fn req(method: &str, path_and_query: &str) -> http::Request<SdkBody> {
        http::Request::builder()
            .method(method)
            .uri(uri(path_and_query))
            .body(SdkBody::empty())
            .unwrap()
    }

    /// Builds a canned response with `status` and `body`, setting `content-length`
    /// to match. Pass `&[]` for a bodyless response (200 OK, 204, HEAD, ...).
    pub(crate) fn resp(status: u16, body: &[u8]) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(status)
            .header("content-length", body.len().to_string())
            .body(SdkBody::from(body.to_vec()))
            .unwrap()
    }

    /// Builds a canned S3 error response carrying an XML `<Error>` document, as S3
    /// returns for failures like `NoSuchKey` or `AccessDenied`.
    pub(crate) fn resp_xml(status: u16, xml: &str) -> http::Response<SdkBody> {
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
    pub(crate) fn access_denied_resp() -> http::Response<SdkBody> {
        resp_xml(403, ACCESS_DENIED_XML)
    }

    /// Wires a `Store` to a replay client expecting exactly one request/response
    /// exchange — the shape of nearly every test here.
    pub(crate) fn store_expecting(
        request: http::Request<SdkBody>,
        response: http::Response<SdkBody>,
    ) -> (Store, StaticReplayClient) {
        store_with(vec![ReplayEvent::new(request, response)])
    }

    /// Stable `(method, uri)` view of a request. Bodies and signing headers are
    /// intentionally dropped so sequence asserts stay comparable.
    pub(crate) fn shape(method: &str, path_and_query: &str) -> (String, String) {
        (method.to_string(), uri(path_and_query))
    }

    /// Projects every request the replay client observed into [`shape`]s, in order.
    pub(crate) fn actual_shapes(replay: &StaticReplayClient) -> Vec<(String, String)> {
        replay
            .actual_requests()
            .map(|r| (r.method().to_string(), r.uri().to_string()))
            .collect()
    }

    /// The payload bytes a recorded request carried. Bodies over the frame size
    /// stream, so the SDK sends them aws-chunked (`<hex len>[;ext]\r\n<data>\r\n`
    /// chunks ending in a `0` chunk and trailers); those are replayed from the
    /// retryable body and decoded.
    pub(crate) async fn recorded_payload(request: &HttpRequest) -> Vec<u8> {
        if let Some(bytes) = request.body().bytes() {
            return bytes.to_vec();
        }
        let wire = request
            .body()
            .try_clone()
            .expect("request bodies are retryable")
            .collect()
            .await
            .expect("recorded body replays")
            .to_bytes();
        let aws_chunked = request
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v.contains("aws-chunked"));
        if !aws_chunked {
            return wire.to_vec();
        }
        let mut payload = Vec::new();
        let mut rest = &wire[..];
        loop {
            let header_end = rest
                .windows(2)
                .position(|w| w == b"\r\n")
                .expect("chunk header");
            let header = std::str::from_utf8(&rest[..header_end]).unwrap();
            let size_hex = header.split(';').next().unwrap_or_default();
            let size = usize::from_str_radix(size_hex, 16).unwrap();
            if size == 0 {
                return payload;
            }
            let data = &rest[header_end + 2..];
            payload.extend_from_slice(&data[..size]);
            rest = &data[size + 2..];
        }
    }

    pub mod timeouts {
        use super::*;

        #[test]
        fn call_limits_stay_under_uploader_backstop() {
            const MIB: u64 = 1024 * 1024;
            let secs = Duration::from_secs;
            assert_eq!(body_attempt_timeout(0), secs(90));
            assert_eq!(body_attempt_timeout(PART_SIZE), secs(218));
            assert_eq!(complete_attempt_timeout(PART_SIZE + 1, secs(0)), secs(219));
            assert_eq!(complete_attempt_timeout(1024 * MIB, secs(0)), secs(600));
            assert_eq!(
                complete_attempt_timeout(PART_SIZE + 1, secs(3600)),
                secs(60)
            );

            let backstop = |len: u64| UploaderOptions::default().attempt_deadline(len);
            // PutObject/UploadPart: the call limit after up to 30 s of backend create.
            for len in [0, MIB, PART_SIZE] {
                assert!(
                    body_attempt_timeout(len) + secs(30) <= backstop(len),
                    "len {len}"
                );
            }
            // Multipart: 20 s backend create, then the S3 create and the parts at the
            // minimum rate eat into complete's budget, and the abort follows. Once
            // create + parts exceed `parts + 30 s`, complete's 60 s floor can
            // overshoot the backstop; that is accepted and not covered here.
            for len in [PART_SIZE + 1, 20 * MIB, 1024 * MIB] {
                let parts = secs(len.div_ceil(64 * 1024));
                for s3_create in [secs(0), secs(30)] {
                    let spent = s3_create + parts;
                    let complete = complete_attempt_timeout(len, spent);
                    let total = secs(20) + spent + complete + ABORT_ATTEMPT_TIMEOUT;
                    assert!(total <= backstop(len), "len {len}, create {s3_create:?}");
                }
            }
        }

        #[tokio::test(start_paused = true)]
        async fn stalled_put_maps_to_connection_err() {
            let len = 1024 * 1024;
            let tf = temp_file_with(&vec![0u8; len]).await;
            let src = Source::open(tf.to_file(), &obj("k")).await.unwrap();
            let store = store_on(NeverClient::new());
            let start = Instant::now();

            let err = store
                .put_singlepart(src, &obj("k"), &HashMap::new())
                .await
                .unwrap_err();

            assert!(matches!(err, S3Err::ConnectionErr(_)), "got {err:?}");
            assert!(err.is_network_conn_err());
            // Ends at the call limit (106 s): stall protection's window runs on the
            // wall clock, which barely moves while paused time races ahead.
            assert_eq!(start.elapsed(), body_attempt_timeout(len as u64));
        }
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
            let config = aws::config(cfg, None, false);
            let _store = Store::new(Options { config, gcs: false });
        }

        #[tokio::test]
        async fn new_sets_timeouts_and_stall_protection() {
            let token = gcs::Token::new("gcs-token").unwrap();
            let stores = [
                (aws::config(test_config(), None, false), false),
                (gcs::config(token, None), true),
            ];
            let timeouts = TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(15))
                .operation_timeout(Duration::from_secs(60))
                .operation_attempt_timeout(Duration::from_secs(60))
                .build();

            for (config, is_gcs) in stores {
                let store = Store::new(Options { config, gcs: is_gcs });
                let config = store.client.config();

                assert_eq!(config.timeout_config(), Some(&timeouts), "gcs {is_gcs}");
                let ssp = config
                    .stalled_stream_protection()
                    .expect("stall protection configured");
                assert!(ssp.upload_enabled(), "gcs {is_gcs}");
                assert_eq!(ssp.grace_period(), Duration::from_secs(60), "gcs {is_gcs}");
            }
        }

        #[tokio::test]
        async fn http_client_builds_connector() {
            let config = aws::config(test_config(), Some(http_client()), false);
            let store = Store::new(Options { config, gcs: false });
            assert!(store.client.config().http_client().is_some());

            // The SDK calls the connector closure with settings and components
            // only on a request; call it directly with both.
            let components = RuntimeComponentsBuilder::for_tests()
                .with_time_source(store.client.config().time_source())
                .build()
                .unwrap();
            let settings = HttpConnectorSettings::builder()
                .connect_timeout(Duration::from_secs(15))
                .build();
            let _connector = http_client().http_connector(&settings, &components);
        }
    }

    pub mod stall_protection {
        use super::*;

        /// Takes the first request body frame, then never polls the body again
        /// or answers.
        #[derive(Clone, Debug)]
        struct HangAfterFirstFrame;

        impl HttpConnector for HangAfterFirstFrame {
            fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
                let mut body = request.into_body();
                HttpConnectorFuture::new(async move {
                    let _ = body.frame().await;
                    std::future::pending::<Result<HttpResponse, ConnectorError>>().await
                })
            }
        }

        impl HttpClient for HangAfterFirstFrame {
            fn http_connector(
                &self,
                _: &HttpConnectorSettings,
                _: &RuntimeComponents,
            ) -> SharedHttpConnector {
                SharedHttpConnector::new(self.clone())
            }
        }

        #[tokio::test]
        async fn chunked_body_trips_stall_protection() {
            let store = store_on(HangAfterFirstFrame);
            // A 1 s grace (instead of 60 s) and no retries keep the test short.
            let fast_stall = StalledStreamProtectionConfig::enabled()
                .upload_enabled(true)
                .grace_period(Duration::from_secs(1))
                .build();
            let config = aws_sdk_s3::Config::builder()
                .stalled_stream_protection(fast_stall)
                .retry_config(RetryConfig::disabled());
            let send = store
                .client
                .put_object()
                .bucket(BUCKET)
                .key("k")
                .body(chunked(Bytes::from(vec![0u8; 256 * 1024])))
                .customize()
                .config_override(config)
                .send();

            // Well under the 60 s call limit, so only stall protection can end it.
            let result = tokio::time::timeout(Duration::from_secs(30), send)
                .await
                .expect("stall protection ends the call");

            let err = map_sdk_err("put_object", &obj("k"), result.unwrap_err());
            assert!(matches!(err, S3Err::ConnectionErr(_)), "got {err:?}");
        }
    }

    pub mod put {
        use super::*;

        pub mod single {
            use super::*;

            async fn source_of(tf: &test_files::TempFile, key: &str) -> Source {
                Source::open(tf.to_file(), &obj(key)).await.unwrap()
            }

            #[tokio::test]
            async fn put_sends_file_body_bytes() {
                let key = "artifacts/hello.txt";
                let src = temp_file_with(b"hello world").await;
                let (store, replay) = store_expecting(
                    req("PUT", "artifacts/hello.txt?x-id=PutObject"),
                    resp(200, &[]),
                );

                store
                    .put_singlepart(source_of(&src, key).await, &obj(key), &HashMap::new())
                    .await
                    .unwrap();

                let requests = replay.actual_requests().collect::<Vec<_>>();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method(), "PUT");
                assert_eq!(
                    requests[0].uri().to_string(),
                    uri("artifacts/hello.txt?x-id=PutObject")
                );
                assert_eq!(requests[0].body().bytes(), Some(&b"hello world"[..]));
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
                    .put_singlepart(source_of(&src, key).await, &obj(key), &HashMap::new())
                    .await
                    .unwrap();

                let requests = replay.actual_requests().collect::<Vec<_>>();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method(), "PUT");
                assert_eq!(
                    requests[0].uri().to_string(),
                    uri("artifacts/empty.txt?x-id=PutObject")
                );
                assert_eq!(requests[0].body().bytes(), Some(&b""[..]));
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
                    .put_singlepart(source_of(&src, key).await, &obj(key), &metadata)
                    .await
                    .unwrap();

                let requests = replay.actual_requests().collect::<Vec<_>>();
                let header = |name: &str| requests[0].headers().get(name);
                assert_eq!(header("x-amz-meta-device_id"), Some("dvc_1"));
                assert_eq!(header("x-amz-meta-digest"), Some("sha256:abc"));
            }

            #[tokio::test]
            async fn path_replaced_after_open_uploads_the_opened_file() {
                let key = "artifacts/robot.log";
                let src = temp_file_with(b"original").await;
                let source = source_of(&src, key).await;

                // Move the opened file away and put a longer, different file at
                // the same path.
                let path = src.file().path();
                let moved = path.with_extension("1");
                tokio::fs::rename(path, &moved).await.unwrap();
                tokio::fs::write(path, b"replacement-bytes").await.unwrap();

                let (store, replay) = store_expecting(
                    req("PUT", "artifacts/robot.log?x-id=PutObject"),
                    resp(200, &[]),
                );

                store
                    .put_singlepart(source, &obj(key), &HashMap::new())
                    .await
                    .unwrap();

                let requests = replay.actual_requests().collect::<Vec<_>>();
                assert_eq!(requests[0].body().bytes(), Some(&b"original"[..]));

                tokio::fs::remove_file(&moved).await.unwrap();
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
                // `put` opens the source before routing by size, so a missing
                // file fails before any request is dispatched.
                let (store, replay) = store_with(vec![]);
                let missing = missing_file();

                let err = store
                    .put(missing, &obj("k"), &HashMap::new())
                    .await
                    .unwrap_err();

                assert!(matches!(
                    err,
                    S3Err::FileSysErr(FileSysErr::PathDoesNotExistErr(_))
                ));
                assert_eq!(actual_shapes(&replay), vec![]);
            }

            #[tokio::test]
            async fn put_unopenable_source_maps_to_local_io_err() {
                // Every platform rejects a path with an interior NUL byte as
                // `InvalidInput` rather than `NotFound`, so the open fails as
                // `LocalIoErr` before any request.
                let src = File::new("/tmp/bad\0name.bin");
                let (store, replay) = store_with(vec![]);

                let err = store
                    .put(src, &obj("k"), &HashMap::new())
                    .await
                    .unwrap_err();

                let S3Err::LocalIoErr(inner) = &err else {
                    panic!("expected LocalIoErr, got {err:?}");
                };
                assert_eq!(
                    (inner.operation.as_str(), &inner.object),
                    ("open_source", &obj("k"))
                );
                assert_eq!(actual_shapes(&replay), vec![]);
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
                let dest = File::new("/nonexistent/dir/out.bin");
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
