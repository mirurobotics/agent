// standard crates
use std::future::Future;

// internal crates
use crate::data_uploads::upload::{
    errors::{classified_executor_err, executor_err, TransferErr, UploadErr},
    executor::Checkpointer,
    job::{MultipartCheckpoint, UploadCheckpoint},
};
use crate::filesys::File;
use crate::s3;
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::{S3UploadCredentials, UploadCredentials};

// external crates
use aws_sdk_s3::config::SharedHttpClient;
use chrono::{DateTime, Utc};
use tracing::info;

/// The seam between the upload executor and the concrete cloud-storage SDKs.
/// Given the vended downscoped credentials and the server-authorized upload,
/// transfer the file's bytes to the object store, resuming the multipart upload
/// in `upload.multipart` and saving any new one through `checkpointer`. Kept
/// separate from the executor so its orchestration (create, reuse, confirm) is
/// unit-testable without a live object store.
///
/// # Cancel safety
///
/// Called from the upload actor, whose in-flight future may be dropped on
/// shutdown, so implementations must tolerate cancellation at any await point.
pub trait ObjectTransfer: Send + Sync {
    fn transfer<K: Checkpointer>(
        &self,
        credentials: &UploadCredentials,
        upload: &UploadCheckpoint,
        file: &File,
        checkpointer: &K,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// Production [`ObjectTransfer`] built on aws-sdk-s3 for both schemes: S3 with
/// the vended SigV4 session credentials, and GCS through its S3-compatible XML
/// API with the vended OAuth2 bearer token (see [`s3::gcs`]). Both go through
/// [`put`]: one `PutObject` or a resumable multipart upload, by size. Tests can
/// replace the HTTP client of both stores.
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

    /// Uploads `file` to AWS S3 using the vended session credentials. The physical
    /// bucket comes from `upload.bucket_name` (not the Miru bucket id); the SDK
    /// derives the endpoint from the credential `region`.
    async fn transfer_s3<K: Checkpointer>(
        &self,
        credentials: &UploadCredentials,
        upload: &UploadCheckpoint,
        file: &File,
        checkpointer: &K,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .s3_credentials
            .as_deref()
            .ok_or_else(|| executor_err(TransferErr::MissingS3Credentials))?;
        let store = self.s3_store(s3_config(creds));
        put(&store, upload, file, checkpointer).await
    }

    /// Uploads `file` to GCS using the vended downscoped OAuth2 bearer token,
    /// which is scoped to `storage.objects.create` on this exact object.
    async fn transfer_gcs<K: Checkpointer>(
        &self,
        credentials: &UploadCredentials,
        upload: &UploadCheckpoint,
        file: &File,
        checkpointer: &K,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .gcs_credentials
            .as_deref()
            .ok_or_else(|| executor_err(TransferErr::MissingGcsCredentials))?;
        let token = s3::gcs::Token::new(creds.access_token.clone())
            .ok_or_else(|| executor_err(TransferErr::InvalidGcsToken))?;
        put(&self.gcs_store(token), upload, file, checkpointer).await
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

/// Puts `file` at the upload's physical bucket and object key: one `PutObject`
/// for a file of one part or less, otherwise a multipart upload a later attempt
/// can resume.
async fn put<K: Checkpointer>(
    store: &s3::Store,
    upload: &UploadCheckpoint,
    file: &File,
    checkpointer: &K,
) -> Result<(), UploadErr> {
    let object = s3::Object {
        bucket: upload.bucket_name.clone(),
        key: upload.object_key.clone(),
    };
    let src = s3::Source::open(file.clone(), &object)
        .await
        .map_err(classified_executor_err)?;
    let result = if src.size <= s3::store::PART_SIZE {
        store.put_singlepart(src, &object, &upload.metadata).await
    } else {
        put_resumable(store, src, &object, upload, checkpointer).await
    };
    result.map_err(classified_executor_err)
}

/// Resumes the multipart upload in `upload.multipart` if it was started for
/// this version of the file and cut into the same part size. Otherwise, or if
/// the provider no longer has it (`NoSuchUpload`), starts a new one on the same
/// key and saves it before the first part goes up. Never aborts on failure, so
/// the next attempt can resume.
async fn put_resumable<K: Checkpointer>(
    store: &s3::Store,
    mut src: s3::Source,
    object: &s3::Object,
    upload: &UploadCheckpoint,
    checkpointer: &K,
) -> Result<(), s3::S3Err> {
    let Some(mtime) = src.mtime.map(DateTime::<Utc>::from) else {
        // Without an mtime the upload can't be pinned to this file version, so
        // it runs start to finish without being saved.
        return store.put_multipart(src, object, &upload.metadata).await;
    };
    let part_size = s3::Store::part_size_for(src.size);
    if let Some(prev) = &upload.multipart {
        if prev.size == src.size && prev.mtime == mtime && prev.part_size == part_size {
            let id = &prev.id;
            info!("upload: resuming multipart upload {id} for {object}");
            match store.resume_multipart_upload(&mut src, object, id).await {
                Err(s3::S3Err::NoSuchUploadErr(_)) => {}
                result => return result,
            }
        }
        info!(
            "upload: multipart upload {} for {object} can't be resumed; starting a new one",
            prev.id
        );
    }

    let id = store
        .create_multipart_upload(object, &upload.metadata)
        .await?;
    let multipart = MultipartCheckpoint {
        id: id.clone(),
        size: src.size,
        mtime,
        part_size,
    };
    checkpointer
        .save(&UploadCheckpoint {
            multipart: Some(multipart),
            ..upload.clone()
        })
        .await;
    // A new upload has no parts yet, so resuming it uploads all of them.
    store.resume_multipart_upload(&mut src, object, &id).await
}

impl ObjectTransfer for SdkTransfer {
    async fn transfer<K: Checkpointer>(
        &self,
        credentials: &UploadCredentials,
        upload: &UploadCheckpoint,
        file: &File,
        checkpointer: &K,
    ) -> Result<(), UploadErr> {
        info!(
            "upload: transferring file {} via scheme {:?} to bucket {} key {}",
            file, credentials.scheme, upload.bucket_name, upload.object_key
        );
        match credentials.scheme {
            Scheme::S3 => {
                self.transfer_s3(credentials, upload, file, checkpointer)
                    .await
            }
            Scheme::Gcs => {
                self.transfer_gcs(credentials, upload, file, checkpointer)
                    .await
            }
            Scheme::SchemeUnknown => Err(executor_err(TransferErr::UnrecognizedScheme)),
        }
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
        checkpointer::{NoopCheckpointer, RecordingCheckpointer},
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
            .transfer(
                &creds,
                &upload_checkpoint(),
                &File::new("/data/a.log"),
                &NoopCheckpointer,
            )
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
            .transfer(
                &creds,
                &upload_checkpoint(),
                &File::new("/data/a.log"),
                &NoopCheckpointer,
            )
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
            .transfer(&creds, &upload_checkpoint(), src.file(), &NoopCheckpointer)
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
            .transfer(&creds, &upload_checkpoint(), src.file(), &NoopCheckpointer)
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
            .transfer(&creds, &upload_checkpoint(), &missing, &NoopCheckpointer)
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn gcs_scheme_without_credentials_errs() {
        let creds = credentials("gcs", Value::Null, Value::Null);
        let err = SdkTransfer::default()
            .transfer(
                &creds,
                &upload_checkpoint(),
                &File::new("/data/a.log"),
                &NoopCheckpointer,
            )
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
            .transfer(
                &creds,
                &upload_checkpoint(),
                &File::new("/data/a.log"),
                &NoopCheckpointer,
            )
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
            .transfer(&creds, &upload_checkpoint(), src.file(), &NoopCheckpointer)
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
        assert_eq!(requests[0].body().bytes(), Some(&b"hello world"[..]));
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
            .transfer(&creds, &upload_checkpoint(), src.file(), &NoopCheckpointer)
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
            .transfer(&creds, &upload_checkpoint(), &missing, &NoopCheckpointer)
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
            .transfer(&creds, &upload_checkpoint(), src.file(), &NoopCheckpointer)
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(err.is_network_conn_err(), "got: {err:?}");
        assert!(!err.is_terminal());
    }

    // Resuming a saved multipart upload, or starting and saving a new one.
    mod resume {
        // internal crates
        use super::*;
        use crate::s3::store::PART_SIZE;
        use miru_agent::data_uploads::upload::{MultipartCheckpoint, UploadCheckpoint};
        use miru_agent::filesys::path::PathExt;

        // external crates
        use chrono::{DateTime, TimeDelta, Utc};

        /// One part plus 1 KiB, so the file splits into two parts.
        const SIZE: u64 = PART_SIZE + 1024;
        const S3_BASE: &str = "https://s3.us-east-1.amazonaws.com/my-bucket/logs/a.log";
        const GCS_BASE: &str = "https://storage.googleapis.com/my-bucket/logs/a.log";

        /// A file of [`SIZE`] bytes and its mtime.
        async fn two_part_file() -> (test_files::TempFile, DateTime<Utc>) {
            let tf = temp_file_with(&vec![7u8; SIZE as usize]).await;
            let mtime = std::fs::metadata(tf.file().path())
                .unwrap()
                .modified()
                .unwrap();
            (tf, DateTime::<Utc>::from(mtime))
        }

        /// The upload with multipart upload `id` saved for a file of `size` bytes
        /// last modified at `mtime`, cut into [`PART_SIZE`] parts.
        fn saved(id: &str, size: u64, mtime: DateTime<Utc>) -> UploadCheckpoint {
            UploadCheckpoint {
                multipart: Some(MultipartCheckpoint {
                    id: id.to_string(),
                    size,
                    mtime,
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

        /// Transfers `file` for `upload` over `replay`, returning the result
        /// and the uploads the transfer saved.
        async fn run(
            creds: &UploadCredentials,
            replay: &StaticReplayClient,
            file: &File,
            upload: &UploadCheckpoint,
        ) -> (Result<(), UploadErr>, Vec<UploadCheckpoint>) {
            let checkpointer = RecordingCheckpointer::new();
            let result = SdkTransfer::with_http_client(replay.clone())
                .transfer(creds, upload, file, &checkpointer)
                .await;
            (result, checkpointer.saved())
        }

        #[tokio::test]
        async fn new_upload_is_saved_then_uploaded() {
            let (tf, mtime) = two_part_file().await;
            let replay = replay(new_upload_resps("mpu_new"));

            let (result, saves) = run(&s3(), &replay, tf.file(), &upload_checkpoint()).await;

            result.unwrap();
            assert_eq!(saves, [saved("mpu_new", SIZE, mtime)]);
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

        // S3 and GCS share the multipart path, so both resume a saved upload by
        // uploading only the parts that haven't landed.
        #[tokio::test]
        async fn saved_upload_resumes_and_skips_landed_parts() {
            let (tf, mtime) = two_part_file().await;
            for creds in [s3(), gcs()] {
                let replay = replay(vec![
                    list_resp(&[(1, "\"e1\"")]),
                    part_resp("\"e2\""),
                    complete_resp(),
                ]);

                let upload = saved("mpu_old", SIZE, mtime);
                let (result, saves) = run(&creds, &replay, tf.file(), &upload).await;

                result.unwrap();
                assert!(saves.is_empty());
                assert_eq!(
                    sent(&replay),
                    [
                        "GET ?x-id=ListParts&uploadId=mpu_old",
                        "PUT ?x-id=UploadPart&partNumber=2&uploadId=mpu_old",
                        "POST ?uploadId=mpu_old",
                    ]
                );
            }
        }

        #[tokio::test]
        async fn gone_upload_starts_a_new_one_on_the_same_key() {
            let (tf, mtime) = two_part_file().await;
            let mut resps = vec![xml(
                404,
                "<Error><Code>NoSuchUpload</Code><Message>gone</Message></Error>",
            )];
            resps.extend(new_upload_resps("mpu_new"));
            let replay = replay(resps);

            let upload = saved("mpu_old", SIZE, mtime);
            let (result, saves) = run(&s3(), &replay, tf.file(), &upload).await;

            result.unwrap();
            assert_eq!(saves, [saved("mpu_new", SIZE, mtime)]);
            assert_eq!(sent(&replay)[1], "POST ?uploads");
        }

        // A saved upload for another version of the file (size or mtime), or one
        // cut into another part size, starts a new one without listing its parts.
        #[tokio::test]
        async fn changed_file_or_part_size_starts_a_new_upload() {
            let (tf, mtime) = two_part_file().await;
            let mut other_part_size = saved("mpu_old", SIZE, mtime);
            other_part_size.multipart.as_mut().unwrap().part_size = 2 * PART_SIZE;
            for upload in [
                saved("mpu_old", SIZE + 1, mtime),
                saved("mpu_old", SIZE, mtime + TimeDelta::seconds(1)),
                other_part_size,
            ] {
                let replay = replay(new_upload_resps("mpu_new"));

                let (result, saves) = run(&s3(), &replay, tf.file(), &upload).await;

                result.unwrap();
                assert_eq!(saves, [saved("mpu_new", SIZE, mtime)]);
                assert_eq!(sent(&replay)[0], "POST ?uploads");
            }
        }

        // A failed part leaves the new upload open for the next attempt: no
        // abort is sent.
        #[tokio::test]
        async fn failed_part_does_not_abort() {
            let (tf, mtime) = two_part_file().await;
            let replay = replay(vec![
                create_resp("mpu_new"),
                list_resp(&[]),
                s3_access_denied_resp(),
            ]);

            let (result, saves) = run(&s3(), &replay, tf.file(), &upload_checkpoint()).await;

            result.unwrap_err();
            assert_eq!(saves, [saved("mpu_new", SIZE, mtime)]);
            assert_eq!(
                sent(&replay),
                [
                    "POST ?uploads",
                    "GET ?x-id=ListParts&uploadId=mpu_new",
                    "PUT ?x-id=UploadPart&partNumber=1&uploadId=mpu_new",
                ]
            );
        }
    }
}
