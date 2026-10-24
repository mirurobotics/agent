// standard crates
use std::collections::HashMap;
use std::future::Future;

// internal crates
use crate::data_uploads::upload::{
    credentials::UploadCredentialSource,
    errors::{classified_executor_err, executor_err, TransferErr, UploadErr},
};
use crate::filesys::File;
use crate::s3;
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::UploadDestination;

// external crates
use aws_sdk_s3::config::SharedHttpClient;
use tracing::info;

/// The seam between the upload executor and the concrete cloud-storage SDKs.
/// Given the vended downscoped credentials and the server-authorized
/// destination, transfer the file's bytes to the object store. Kept separate
/// from the executor so its orchestration (create, dedup, confirm) is
/// unit-testable without a live object store.
///
/// # Cancel safety
///
/// Called from the upload actor, whose in-flight future may be dropped on
/// shutdown, so implementations must tolerate cancellation at any await point.
pub trait ObjectTransfer: Send + Sync {
    fn transfer(
        &self,
        credentials: &UploadCredentialSource,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// Production [`ObjectTransfer`] built on aws-sdk-s3 for both schemes: S3 with
/// the vended SigV4 session credentials, and GCS through its S3-compatible XML
/// API with the vended OAuth2 bearer token (see [`s3::gcs`]). Both go through
/// [`s3::Store::put`]: single-part or multipart by size. Tests can replace the
/// HTTP client of both stores.
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
    /// bucket comes from `destination.bucket_name` (not the Miru `bucket_id`); the
    /// SDK derives the endpoint from the credential `region`.
    async fn transfer_s3(
        &self,
        credentials: &UploadCredentialSource,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .current()
            .s3_credentials
            .ok_or_else(|| executor_err(TransferErr::MissingS3Credentials))?;
        let store = self.s3_store(credentials, creds.region);
        put(&store, destination, file, metadata).await
    }

    /// Uploads `file` to GCS using the vended downscoped OAuth2 bearer token,
    /// which is scoped to `storage.objects.create` on this exact object.
    async fn transfer_gcs(
        &self,
        credentials: &UploadCredentialSource,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .current()
            .gcs_credentials
            .ok_or_else(|| executor_err(TransferErr::MissingGcsCredentials))?;
        if s3::gcs::Token::new(creds.access_token).is_none() {
            return Err(executor_err(TransferErr::InvalidGcsToken));
        }
        put(&self.gcs_store(credentials), destination, file, metadata).await
    }

    /// Builds the S3 store with this transfer's transport settings.
    fn s3_store(&self, credentials: &UploadCredentialSource, region: String) -> s3::Store {
        let config = s3::aws::config(
            credentials.clone(),
            region,
            self.http_client.clone(),
            self.s3_force_path_style,
        );
        s3::Store::new(s3::Options { config, gcs: false })
    }

    /// Builds the GCS store with this transfer's transport settings.
    fn gcs_store(&self, credentials: &UploadCredentialSource) -> s3::Store {
        let config = s3::gcs::config(credentials.clone(), self.http_client.clone());
        s3::Store::new(s3::Options { config, gcs: true })
    }
}

/// Puts `file` at the destination's physical bucket (`bucket_name`, not the
/// Miru `bucket_id`) and object key.
async fn put(
    store: &s3::Store,
    destination: &UploadDestination,
    file: &File,
    metadata: &HashMap<String, String>,
) -> Result<(), UploadErr> {
    let object = s3::Object {
        bucket: destination.bucket_name.clone(),
        key: destination.object_key.clone(),
    };
    store
        .put(file.clone(), &object, metadata)
        .await
        .map_err(classified_executor_err)
}

impl ObjectTransfer for SdkTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentialSource,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        let scheme = credentials.current().scheme;
        info!(
            "upload: transferring file {} via scheme {:?} to bucket {} key {}",
            file, scheme, destination.bucket_name, destination.object_key
        );
        match scheme {
            Scheme::S3 => {
                self.transfer_s3(credentials, destination, file, metadata)
                    .await
            }
            Scheme::Gcs => {
                self.transfer_gcs(credentials, destination, file, metadata)
                    .await
            }
            Scheme::SchemeUnknown => Err(executor_err(TransferErr::UnrecognizedScheme)),
        }
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::HashMap;

    // internal crates
    use crate::test_utils::{
        filesys::{files as test_files, missing_file},
        upload::{destination, response_metadata, s3_credentials_json},
    };
    use miru_agent::data_uploads::upload::errors::TransferErr;
    use miru_agent::data_uploads::upload::{
        ObjectTransfer, SdkTransfer, UploadCredentialSource, UploadErr,
    };
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

    /// Fixed credentials for `scheme` with the given credential arms.
    fn credentials(scheme: &str, s3: Value, gcs: Value) -> UploadCredentialSource {
        let creds = serde_json::from_value(json!({
            "scheme": scheme,
            "s3_credentials": s3,
            "gcs_credentials": gcs,
            "expires_at": "2021-01-01T01:00:00Z"
        }))
        .unwrap();
        UploadCredentialSource::fixed(creds)
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

    /// The path-style PutObject URI the S3 SDK emits for [`destination`]'s bucket
    /// and key with the vended `us-east-1` region: `bucket_name` (never the Miru
    /// `bucket_id`) is the path's first segment.
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

    /// The path-style PutObject URI on GCS's XML API for [`destination`]'s
    /// bucket and key.
    const GCS_PUT_URI: &str = "https://storage.googleapis.com/my-bucket/logs/a.log?x-id=PutObject";

    #[tokio::test]
    async fn unknown_scheme_is_unsupported() {
        let creds = credentials("something-new", s3_credentials_json(), Value::Null);
        let err = SdkTransfer::default()
            .transfer(
                &creds,
                &destination(),
                &File::new("/data/a.log"),
                &HashMap::new(),
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
                &destination(),
                &File::new("/data/a.log"),
                &HashMap::new(),
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
            .transfer(&creds, &destination(), src.file(), &response_metadata())
            .await
            .unwrap();

        // The URI is the load-bearing assertion: `my-bucket` in the path proves
        // `bucket_name` reached the SDK (a `bkt_1` would mean `bucket_id` regressed
        // in), `logs/a.log` proves `object_key`, and the host proves the vended
        // region flows through. The streamed body can't be byte-compared — the
        // replay client records it as an unbuffered `SdkBody` (see
        // `tests/s3` `put_streams_file_body_bytes`).
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
            .transfer(&creds, &destination(), src.file(), &HashMap::new())
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
            .transfer(&creds, &destination(), &missing, &HashMap::new())
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
                &destination(),
                &File::new("/data/a.log"),
                &HashMap::new(),
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
                &destination(),
                &File::new("/data/a.log"),
                &HashMap::new(),
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
            .transfer(&creds, &destination(), src.file(), &response_metadata())
            .await
            .unwrap();

        // `my-bucket` in the path proves `bucket_name` reached the SDK (not the
        // Miru `bucket_id`), `logs/a.log` proves `object_key`, and the bearer
        // proves the vended token flows through.
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
            .transfer(&creds, &destination(), src.file(), &HashMap::new())
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
            .transfer(&creds, &destination(), &missing, &HashMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
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
            .transfer(&creds, &destination(), src.file(), &HashMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(err.is_network_conn_err(), "got: {err:?}");
        assert!(!err.is_terminal());
    }
}
