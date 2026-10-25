// standard crates
use std::future::Future;

// internal crates
use crate::data_uploads::upload::{
    errors::{classified_executor_err, executor_err, TransferErr, UploadErr},
    job::{MultipartCheckpoint, UploadCheckpoint},
};
use crate::filesys::File;
use crate::s3;
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::{S3UploadCredentials, UploadCredentials};

// external crates
use aws_sdk_s3::config::SharedHttpClient;
use tracing::info;

/// Transfers a file to the checkpoint's object in cloud storage.
///
/// # Cancel safety
///
/// Either future may be dropped at any await point.
pub trait ObjectTransfer: Send + Sync {
    /// The multipart upload [`Self::transfer`] resumes: the saved one if it
    /// still matches `file`, else a new one, or [`TransferPlan::SinglePart`]
    /// for a single-part file.
    fn reuse_or_create_multipart(
        &self,
        credentials: &UploadCredentials,
        checkpoint: &UploadCheckpoint,
        file: &File,
    ) -> impl Future<Output = Result<TransferPlan, UploadErr>> + Send;

    /// Transfers the file, resuming `checkpoint.multipart` if it matches the file.
    fn transfer(
        &self,
        credentials: &UploadCredentials,
        checkpoint: &UploadCheckpoint,
        file: &File,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// How [`ObjectTransfer::transfer`] sends a file.
#[derive(Debug, PartialEq)]
pub enum TransferPlan {
    /// In parts, through this multipart upload.
    Multipart(MultipartCheckpoint),
    /// In one request, since the file fits in one part.
    SinglePart,
}

/// Production [`ObjectTransfer`] built on aws-sdk-s3 for both schemes: S3 with
/// the vended SigV4 session credentials, and GCS through its S3-compatible XML
/// API with the vended OAuth2 bearer token (see [`s3::gcs`]). Both go through
/// [`resume_or_put`]. Tests can replace the HTTP client of both stores.
pub struct SdkTransfer {
    http_client: SharedHttpClient,
    s3_force_path_style: bool,
}

/// Every upload through this transfer shares one HTTPS client and its
/// connection pool.
impl Default for SdkTransfer {
    fn default() -> Self {
        Self {
            http_client: s3::sdk::http_client(),
            s3_force_path_style: false,
        }
    }
}

impl SdkTransfer {
    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) into both stores in place of the default HTTPS
    /// connector, so tests serve canned responses without touching the network.
    #[cfg(test)]
    pub(crate) fn with_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
    ) -> Self {
        Self {
            http_client: SharedHttpClient::new(http_client),
            s3_force_path_style: true,
        }
    }

    /// The S3 or GCS store for the vended credentials.
    fn store_for(&self, credentials: &UploadCredentials) -> Result<s3::Store, UploadErr> {
        match credentials.scheme {
            Scheme::S3 => {
                let creds = credentials
                    .s3_credentials
                    .as_deref()
                    .ok_or_else(|| executor_err(TransferErr::MissingS3Credentials))?;
                Ok(self.s3_store(s3_config(creds)))
            }
            Scheme::Gcs => {
                let creds = credentials
                    .gcs_credentials
                    .as_deref()
                    .ok_or_else(|| executor_err(TransferErr::MissingGcsCredentials))?;
                let token = s3::gcs::Token::new(creds.access_token.clone())
                    .ok_or_else(|| executor_err(TransferErr::InvalidGcsToken))?;
                Ok(self.gcs_store(token))
            }
            Scheme::SchemeUnknown => Err(executor_err(TransferErr::UnrecognizedScheme)),
        }
    }

    /// Builds the S3 store with this transfer's transport settings.
    fn s3_store(&self, cfg: s3::Config) -> s3::Store {
        let config = s3::aws::config(cfg, self.http_client.clone(), self.s3_force_path_style);
        s3::Store::new(s3::Options { config, gcs: false })
    }

    /// Builds the GCS store with this transfer's transport settings.
    fn gcs_store(&self, token: s3::gcs::Token) -> s3::Store {
        let config = s3::gcs::config(token, self.http_client.clone());
        s3::Store::new(s3::Options { config, gcs: true })
    }
}

impl ObjectTransfer for SdkTransfer {
    async fn reuse_or_create_multipart(
        &self,
        credentials: &UploadCredentials,
        checkpoint: &UploadCheckpoint,
        file: &File,
    ) -> Result<TransferPlan, UploadErr> {
        let store = self.store_for(credentials)?;
        reuse_or_create_multipart(&store, checkpoint, file)
            .await
            .map_err(classified_executor_err)
    }

    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        checkpoint: &UploadCheckpoint,
        file: &File,
    ) -> Result<(), UploadErr> {
        info!(
            "upload: transferring file {} via scheme {:?} to bucket {} key {}",
            file, credentials.scheme, checkpoint.bucket_name, checkpoint.object_key
        );
        let store = self.store_for(credentials)?;
        resume_or_put(&store, checkpoint, file)
            .await
            .map_err(classified_executor_err)
    }
}

/// See [`ObjectTransfer::reuse_or_create_multipart`].
async fn reuse_or_create_multipart(
    store: &s3::Store,
    checkpoint: &UploadCheckpoint,
    file: &File,
) -> Result<TransferPlan, s3::S3Err> {
    let object = checkpoint.object();
    let src = s3::Source::open(file.clone(), &object).await?;
    if src.size <= s3::store::PART_SIZE {
        return Ok(TransferPlan::SinglePart);
    }
    if let Some(saved) = checkpoint.resumable_multipart(&src) {
        let id = &saved.multipart_upload_id;
        if store.has_multipart_upload(&object, id).await? {
            return Ok(TransferPlan::Multipart(saved.clone()));
        }
        info!("upload: multipart upload {id} for {object} is gone");
    }
    let id = store
        .create_multipart_upload(&object, &checkpoint.metadata)
        .await?;
    info!("upload: started multipart upload {id} for {object}");
    Ok(TransferPlan::Multipart(MultipartCheckpoint {
        multipart_upload_id: id,
        size: src.size,
        part_size: s3::Store::part_size_for(src.size),
    }))
}

/// Puts `file` at the upload's object, resuming `checkpoint.multipart` if it
/// matches the file.
async fn resume_or_put(
    store: &s3::Store,
    checkpoint: &UploadCheckpoint,
    file: &File,
) -> Result<(), s3::S3Err> {
    let object = checkpoint.object();
    let src = s3::Source::open(file.clone(), &object).await?;
    match checkpoint.resumable_multipart(&src) {
        Some(multipart) => {
            let id = &multipart.multipart_upload_id;
            info!("upload: resuming multipart upload {id} for {object}");
            store.resume_multipart_upload(src, &object, id).await
        }
        None => store.put(file.clone(), &object, &checkpoint.metadata).await,
    }
}

/// Maps vended S3 session credentials into an [`s3::Config`]. Kept separate so
/// the credential→SDK mapping is unit-testable without a live transfer.
fn s3_config(creds: &S3UploadCredentials) -> s3::Config {
    s3::Config {
        creds: s3::Credentials {
            access_key_id: creds.access_key_id.clone(),
            secret_access_key: creds.secret_access_key.clone(),
            session_token: creds.session_token.clone(),
        },
        region: creds.region.clone(),
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::s3_config;
    use crate::test_utils::{
        filesys::{files as test_files, missing_file},
        upload::{s3_credentials_json, upload_checkpoint},
    };
    use backend_api::models::{S3UploadCredentials, UploadCredentials};
    use miru_agent::data_uploads::upload::errors::TransferErr;
    use miru_agent::data_uploads::upload::{ObjectTransfer, SdkTransfer, UploadErr};
    use miru_agent::errors::Error as ErrorTrait;
    use miru_agent::filesys::{files, File, WriteOptions};

    // external crates
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_runtime_api::client::http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    };
    use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
    use aws_smithy_runtime_api::client::result::ConnectorError;
    use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
    use aws_smithy_types::body::SdkBody;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};

    /// Extracts the transfer layer's typed precondition failure from the single
    /// `ExecutorErr` surface the actor sees.
    fn transfer_err(err: &UploadErr) -> &TransferErr {
        let UploadErr::ExecutorErr(e) = err else {
            panic!("expected ExecutorErr, got: {err:?}");
        };
        e.source
            .downcast_ref::<TransferErr>()
            .unwrap_or_else(|| panic!("expected TransferErr source, got: {:?}", e.source))
    }

    fn gcs_credentials_json(access_token: &str) -> Value {
        json!({
            "scheme": "gcs",
            "access_token": access_token,
            "expires_at": "2021-01-01T01:00:00Z"
        })
    }

    /// A full `UploadCredentials` for `scheme` with the given credential arms.
    fn credentials(scheme: &str, s3: Value, gcs: Value) -> UploadCredentials {
        serde_json::from_value(json!({
            "scheme": scheme,
            "s3_credentials": s3,
            "gcs_credentials": gcs,
            "expires_at": "2021-01-01T01:00:00Z"
        }))
        .unwrap()
    }

    /// Writes `bytes` to a fresh temp file and returns the guard (kept alive so
    /// the file is not deleted until the test drops it).
    async fn temp_file_with(bytes: &[u8]) -> test_files::TempFile {
        let tf = test_files::temp("upload-transfer-test").unwrap();
        files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        tf
    }

    /// The path-style PutObject URI the S3 SDK emits for [`upload_checkpoint`]'s bucket
    /// and key with the vended `us-east-1` region.
    const S3_PUT_URI: &str =
        "https://s3.us-east-1.amazonaws.com/my-bucket/logs/a.log?x-id=PutObject";

    /// Builds an expected request (method + path-style URI, empty body) to match a
    /// `ReplayEvent` against. Only the method and URI are meaningful; the streamed
    /// body is not compared.
    fn s3_req(method: &str, uri: &str) -> http::Request<SdkBody> {
        http::Request::builder()
            .method(method)
            .uri(uri)
            .body(SdkBody::empty())
            .unwrap()
    }

    /// Builds a canned response with `status` and `body`, setting `content-length`
    /// to match. Pass `&[]` for a bodyless 200 OK.
    fn s3_resp(status: u16, body: &[u8]) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(status)
            .header("content-length", body.len().to_string())
            .body(SdkBody::from(body.to_vec()))
            .unwrap()
    }

    const ACCESS_DENIED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message><RequestId>REQ403</RequestId></Error>"#;

    /// Canned S3 403 AccessDenied response carrying the XML `<Error>` document S3
    /// returns for authorization failures.
    fn s3_access_denied_resp() -> http::Response<SdkBody> {
        http::Response::builder()
            .status(403)
            .header("content-type", "application/xml")
            .body(SdkBody::from(ACCESS_DENIED_XML.to_string()))
            .unwrap()
    }

    /// The path-style PutObject URI on GCS's XML API for [`upload_checkpoint`]'s
    /// bucket and key.
    const GCS_PUT_URI: &str = "https://storage.googleapis.com/my-bucket/logs/a.log?x-id=PutObject";

    #[tokio::test]
    async fn unknown_scheme_is_unsupported() {
        let creds = credentials("something-new", s3_credentials_json(), Value::Null);
        let err = SdkTransfer::default()
            .transfer(&creds, &upload_checkpoint(), &File::new("/data/a.log"))
            .await
            .unwrap_err();

        assert!(
            matches!(transfer_err(&err), TransferErr::UnrecognizedScheme),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn s3_scheme_without_credentials_errs() {
        let creds = credentials("s3", Value::Null, Value::Null);
        let err = SdkTransfer::default()
            .transfer(&creds, &upload_checkpoint(), &File::new("/data/a.log"))
            .await
            .unwrap_err();

        assert!(
            matches!(transfer_err(&err), TransferErr::MissingS3Credentials),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn s3_transfer_puts_object_to_bucket_name_and_key() {
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            s3_req("PUT", S3_PUT_URI),
            s3_resp(200, &[]),
        )]);
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("s3", s3_credentials_json(), Value::Null);

        SdkTransfer::with_http_client(replay.clone())
            .transfer(&creds, &upload_checkpoint(), src.file())
            .await
            .unwrap();

        // The URI is the load-bearing assertion: `my-bucket` in the path proves
        // `bucket_name` reached the SDK, `logs/a.log` proves `object_key`, and the
        // host proves the vended region flows through. The streamed body can't
        // be byte-compared — the replay client records it as an unbuffered
        // `SdkBody` (see `tests/s3` `put_streams_file_body_bytes`).
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), "PUT");
        assert_eq!(requests[0].uri().to_string(), S3_PUT_URI);
        // The vended metadata map is stamped as user-defined object metadata.
        assert_eq!(
            requests[0].headers().get("x-amz-meta-device_id"),
            Some("dvc_1")
        );
        assert!(requests[0]
            .headers()
            .get("authorization")
            .unwrap()
            .contains("Credential=AKIA_TEST/"));
        assert_eq!(
            Some("session"),
            requests[0].headers().get("x-amz-security-token")
        );
    }

    #[tokio::test]
    async fn s3_put_failure_maps_to_executor_err() {
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            s3_req("PUT", S3_PUT_URI),
            s3_access_denied_resp(),
        )]);
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("s3", s3_credentials_json(), Value::Null);

        let err = SdkTransfer::with_http_client(replay.clone())
            .transfer(&creds, &upload_checkpoint(), src.file())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(!err.is_network_conn_err());
        assert!(!err.is_terminal());
    }

    #[tokio::test]
    async fn s3_default_store_fails_offline_on_missing_file() {
        // Default settings use the shared S3 builder without a transport override.
        // Construction does no I/O and `put` stats the file before dispatching any
        // request, so a missing local file fails fast offline.
        let creds = credentials("s3", s3_credentials_json(), Value::Null);
        let missing = missing_file();

        let err = SdkTransfer::default()
            .transfer(&creds, &upload_checkpoint(), &missing)
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn gcs_scheme_without_credentials_errs() {
        let creds = credentials("gcs", Value::Null, Value::Null);
        let err = SdkTransfer::default()
            .transfer(&creds, &upload_checkpoint(), &File::new("/data/a.log"))
            .await
            .unwrap_err();

        assert!(
            matches!(transfer_err(&err), TransferErr::MissingGcsCredentials),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn gcs_invalid_token_surfaces_executor_err() {
        // A newline is not a valid HTTP header byte, so the token is refused
        // before anything is sent.
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("bad\ntoken"));
        let err = SdkTransfer::default()
            .transfer(&creds, &upload_checkpoint(), &File::new("/data/a.log"))
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(!err.is_network_conn_err());
        assert!(!err.is_terminal());
    }

    #[tokio::test]
    async fn gcs_transfer_puts_object_to_bucket_name_and_key() {
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            s3_req("PUT", GCS_PUT_URI),
            s3_resp(200, &[]),
        )]);
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

        SdkTransfer::with_http_client(replay.clone())
            .transfer(&creds, &upload_checkpoint(), src.file())
            .await
            .unwrap();

        // `my-bucket` in the path proves `bucket_name` reached the SDK,
        // `logs/a.log` proves `object_key`, and the bearer proves the vended
        // token flows through.
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].uri().to_string(), GCS_PUT_URI);
        let headers = requests[0].headers();
        assert_eq!(headers.get("authorization"), Some("Bearer vended-token"));
        assert_eq!(headers.get("x-goog-meta-device_id"), Some("dvc_1"));
        // GCS sends the streamed body without aws-chunked framing.
        let body = requests[0].body().try_clone().unwrap();
        let sent = body.collect().await.unwrap().to_bytes();
        assert_eq!(sent, &b"hello world"[..]);
    }

    #[tokio::test]
    async fn gcs_put_failure_maps_to_executor_err() {
        let replay = StaticReplayClient::new(vec![ReplayEvent::new(
            s3_req("PUT", GCS_PUT_URI),
            s3_access_denied_resp(),
        )]);
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

        let err = SdkTransfer::with_http_client(replay)
            .transfer(&creds, &upload_checkpoint(), src.file())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(!err.is_network_conn_err());
        assert!(!err.is_terminal());
    }

    #[tokio::test]
    async fn gcs_default_store_fails_offline_on_missing_file() {
        // Default settings and a valid token build the GCS store offline. `put`
        // opens the file before dispatching any request, so a missing local
        // file fails fast offline.
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("valid-token"));
        let missing = missing_file();

        let err = SdkTransfer::default()
            .transfer(&creds, &upload_checkpoint(), &missing)
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    }

    #[test]
    fn s3_config_maps_credentials() {
        let creds: S3UploadCredentials = serde_json::from_value(s3_credentials_json()).unwrap();
        let cfg = s3_config(&creds);
        assert_eq!(
            (
                cfg.region.as_str(),
                cfg.creds.access_key_id.as_str(),
                cfg.creds.secret_access_key.as_str(),
                cfg.creds.session_token.as_str(),
            ),
            ("us-east-1", "AKIA_TEST", "secret", "session")
        );
    }

    /// Fails every request as a refused connection.
    #[derive(Clone, Debug)]
    struct Refused;

    impl HttpConnector for Refused {
        fn call(&self, _: HttpRequest) -> HttpConnectorFuture {
            let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
            HttpConnectorFuture::ready(Err(ConnectorError::io(refused.into())))
        }
    }

    impl HttpClient for Refused {
        fn http_connector(
            &self,
            _: &HttpConnectorSettings,
            _: &RuntimeComponents,
        ) -> SharedHttpConnector {
            SharedHttpConnector::new(self.clone())
        }
    }

    #[tokio::test]
    async fn gcs_connection_failure_is_classified_network() {
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

        let err = SdkTransfer::with_http_client(Refused)
            .transfer(&creds, &upload_checkpoint(), src.file())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(err.is_network_conn_err(), "got: {err:?}");
        assert!(!err.is_terminal());
    }

    // Preparing a saved or new multipart upload, then resuming it.
    mod resume {
        // internal crates
        use super::*;
        use crate::s3::store::PART_SIZE;
        use miru_agent::data_uploads::upload::{
            MultipartCheckpoint, TransferPlan, UploadCheckpoint,
        };

        /// One part plus 1 KiB, so the file splits into two parts.
        const SIZE: u64 = PART_SIZE + 1024;
        const S3_BASE: &str = "https://s3.us-east-1.amazonaws.com/my-bucket/logs/a.log";
        const GCS_BASE: &str = "https://storage.googleapis.com/my-bucket/logs/a.log";

        /// A file of [`SIZE`] bytes.
        async fn two_part_file() -> test_files::TempFile {
            temp_file_with(&vec![7u8; SIZE as usize]).await
        }

        /// The checkpoint with multipart upload `id` saved for a file of `size`
        /// bytes, cut into [`PART_SIZE`] parts.
        fn saved(id: &str, size: u64) -> UploadCheckpoint {
            UploadCheckpoint {
                multipart: Some(MultipartCheckpoint {
                    multipart_upload_id: id.to_string(),
                    size,
                    part_size: PART_SIZE,
                }),
                ..upload_checkpoint()
            }
        }

        fn s3() -> UploadCredentials {
            credentials("s3", s3_credentials_json(), Value::Null)
        }

        fn gcs() -> UploadCredentials {
            credentials("gcs", Value::Null, gcs_credentials_json("vended-token"))
        }

        fn xml(status: u16, body: &str) -> http::Response<SdkBody> {
            http::Response::builder()
                .status(status)
                .header("content-type", "application/xml")
                .body(SdkBody::from(body.to_string()))
                .unwrap()
        }

        fn create_resp(id: &str) -> http::Response<SdkBody> {
            xml(
                200,
                &format!(
                    "<InitiateMultipartUploadResult><Bucket>my-bucket</Bucket>\
                     <Key>logs/a.log</Key><UploadId>{id}</UploadId>\
                     </InitiateMultipartUploadResult>"
                ),
            )
        }

        fn part_resp(etag: &str) -> http::Response<SdkBody> {
            http::Response::builder()
                .status(200)
                .header("ETag", etag)
                .body(SdkBody::empty())
                .unwrap()
        }

        fn complete_resp() -> http::Response<SdkBody> {
            xml(
                200,
                "<CompleteMultipartUploadResult><Bucket>my-bucket</Bucket>\
                 <Key>logs/a.log</Key><ETag>\"final\"</ETag></CompleteMultipartUploadResult>",
            )
        }

        fn list_resp(landed: &[(i32, &str)]) -> http::Response<SdkBody> {
            let parts: String = landed
                .iter()
                .map(|(n, etag)| {
                    format!("<Part><PartNumber>{n}</PartNumber><ETag>{etag}</ETag><Size>1</Size></Part>")
                })
                .collect();
            xml(
                200,
                &format!(
                    "<ListPartsResult><Bucket>my-bucket</Bucket><Key>logs/a.log</Key>\
                     <IsTruncated>false</IsTruncated>{parts}</ListPartsResult>"
                ),
            )
        }

        /// The responses to uploading a new two-part upload `id`: create, an
        /// empty part listing, both parts, then complete.
        fn new_upload_resps(id: &str) -> Vec<http::Response<SdkBody>> {
            vec![
                create_resp(id),
                list_resp(&[]),
                part_resp("\"e1\""),
                part_resp("\"e2\""),
                complete_resp(),
            ]
        }

        /// Replays `responses` in order; requests are checked afterwards.
        fn replay(responses: Vec<http::Response<SdkBody>>) -> StaticReplayClient {
            StaticReplayClient::new(
                responses
                    .into_iter()
                    .map(|resp| ReplayEvent::new(s3_req("GET", S3_BASE), resp))
                    .collect(),
            )
        }

        /// The `METHOD path?query` of each request the replay client saw,
        /// relative to the object's URL.
        fn sent(replay: &StaticReplayClient) -> Vec<String> {
            replay
                .actual_requests()
                .map(|r| {
                    let uri = r.uri().to_string();
                    let rest = uri
                        .strip_prefix(S3_BASE)
                        .or_else(|| uri.strip_prefix(GCS_BASE))
                        .unwrap_or(&uri)
                        .to_string();
                    format!("{} {rest}", r.method())
                })
                .collect()
        }

        /// Prepares and transfers `file` over `replay` as an attempt does,
        /// returning the result and the checkpoint it used.
        async fn run(
            creds: &UploadCredentials,
            replay: &StaticReplayClient,
            file: &File,
            mut checkpoint: UploadCheckpoint,
        ) -> (Result<(), UploadErr>, UploadCheckpoint) {
            let transfer = SdkTransfer::with_http_client(replay.clone());
            if let Ok(TransferPlan::Multipart(multipart)) = transfer
                .reuse_or_create_multipart(creds, &checkpoint, file)
                .await
            {
                checkpoint.multipart = Some(multipart);
            }
            let result = transfer.transfer(creds, &checkpoint, file).await;
            (result, checkpoint)
        }

        #[tokio::test]
        async fn new_upload_is_prepared_then_uploaded() {
            let tf = two_part_file().await;
            let replay = replay(new_upload_resps("mpu_new"));

            let (result, prepared) = run(&s3(), &replay, tf.file(), upload_checkpoint()).await;

            result.unwrap();
            assert_eq!(prepared, saved("mpu_new", SIZE));
            assert_eq!(
                sent(&replay),
                [
                    "POST ?uploads",
                    "GET ?x-id=ListParts&uploadId=mpu_new",
                    "PUT ?x-id=UploadPart&partNumber=1&uploadId=mpu_new",
                    "PUT ?x-id=UploadPart&partNumber=2&uploadId=mpu_new",
                    "POST ?uploadId=mpu_new",
                ]
            );
        }

        // S3 and GCS share the multipart path, so both upload only missing parts.
        #[tokio::test]
        async fn saved_upload_resumes_and_skips_landed_parts() {
            let tf = two_part_file().await;
            for creds in [s3(), gcs()] {
                let replay = replay(vec![
                    list_resp(&[(1, "\"e1\"")]),
                    list_resp(&[(1, "\"e1\"")]),
                    part_resp("\"e2\""),
                    complete_resp(),
                ]);

                let checkpoint = saved("mpu_old", SIZE);
                let (result, prepared) = run(&creds, &replay, tf.file(), checkpoint.clone()).await;

                result.unwrap();
                assert_eq!(prepared, checkpoint);
                assert_eq!(
                    sent(&replay),
                    [
                        "GET ?x-id=ListParts&uploadId=mpu_old",
                        "GET ?x-id=ListParts&uploadId=mpu_old",
                        "PUT ?x-id=UploadPart&partNumber=2&uploadId=mpu_old",
                        "POST ?uploadId=mpu_old",
                    ]
                );
            }
        }

        #[tokio::test]
        async fn gone_upload_starts_a_new_one_on_the_same_key() {
            let tf = two_part_file().await;
            let mut resps = vec![xml(
                404,
                "<Error><Code>NoSuchUpload</Code><Message>gone</Message></Error>",
            )];
            resps.extend(new_upload_resps("mpu_new"));
            let replay = replay(resps);

            let checkpoint = saved("mpu_old", SIZE);
            let (result, prepared) = run(&s3(), &replay, tf.file(), checkpoint).await;

            result.unwrap();
            assert_eq!(prepared, saved("mpu_new", SIZE));
            assert_eq!(sent(&replay)[1], "POST ?uploads");
        }

        // A saved upload for a file of another size, or one cut into another
        // part size, starts a new one without listing its parts.
        #[tokio::test]
        async fn changed_size_or_part_size_starts_a_new_upload() {
            let tf = two_part_file().await;
            let mut other_part_size = saved("mpu_old", SIZE);
            other_part_size.multipart.as_mut().unwrap().part_size = 2 * PART_SIZE;
            for checkpoint in [saved("mpu_old", SIZE + 1), other_part_size] {
                let replay = replay(new_upload_resps("mpu_new"));

                let (result, prepared) = run(&s3(), &replay, tf.file(), checkpoint).await;

                result.unwrap();
                assert_eq!(prepared, saved("mpu_new", SIZE));
                assert_eq!(sent(&replay)[0], "POST ?uploads");
            }
        }

        // A failed part leaves the new upload open for the next attempt: no
        // abort is sent.
        #[tokio::test]
        async fn failed_part_does_not_abort() {
            let tf = two_part_file().await;
            let replay = replay(vec![
                create_resp("mpu_new"),
                list_resp(&[]),
                s3_access_denied_resp(),
            ]);

            let (result, prepared) = run(&s3(), &replay, tf.file(), upload_checkpoint()).await;

            result.unwrap_err();
            assert_eq!(prepared, saved("mpu_new", SIZE));
            assert_eq!(
                sent(&replay),
                [
                    "POST ?uploads",
                    "GET ?x-id=ListParts&uploadId=mpu_new",
                    "PUT ?x-id=UploadPart&partNumber=1&uploadId=mpu_new",
                ]
            );
        }

        // If preparing can't check the saved upload, the attempt keeps it, and
        // the transfer's own part listing fails the attempt.
        #[tokio::test]
        async fn failed_check_keeps_the_saved_upload() {
            let tf = two_part_file().await;
            let replay = replay(vec![s3_access_denied_resp(), s3_access_denied_resp()]);

            let checkpoint = saved("mpu_old", SIZE);
            let (result, prepared) = run(&s3(), &replay, tf.file(), checkpoint.clone()).await;

            result.unwrap_err();
            assert_eq!(prepared, checkpoint);
            assert_eq!(
                sent(&replay),
                [
                    "GET ?x-id=ListParts&uploadId=mpu_old",
                    "GET ?x-id=ListParts&uploadId=mpu_old",
                ]
            );
        }

        // If preparing can't start a multipart upload, the transfer starts one
        // and runs it start to finish, without saving it.
        #[tokio::test]
        async fn failed_start_uploads_in_one_run() {
            let tf = two_part_file().await;
            let replay = replay(vec![
                s3_access_denied_resp(),
                create_resp("mpu_2"),
                part_resp("\"e1\""),
                part_resp("\"e2\""),
                complete_resp(),
            ]);

            let (result, prepared) = run(&s3(), &replay, tf.file(), upload_checkpoint()).await;

            result.unwrap();
            assert_eq!(prepared, upload_checkpoint());
            assert_eq!(
                sent(&replay),
                [
                    "POST ?uploads",
                    "POST ?uploads",
                    "PUT ?x-id=UploadPart&partNumber=1&uploadId=mpu_2",
                    "PUT ?x-id=UploadPart&partNumber=2&uploadId=mpu_2",
                    "POST ?uploadId=mpu_2",
                ]
            );
        }
    }
}
