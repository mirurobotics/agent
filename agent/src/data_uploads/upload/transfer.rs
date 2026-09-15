// standard crates
use std::collections::HashMap;
use std::future::Future;

// internal crates
use crate::data_uploads::upload::errors::{
    classified_executor_err, executor_err, TransferErr, UploadErr,
};
use crate::filesys::File;
use crate::gcs;
use crate::s3;
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::{S3UploadCredentials, UploadCredentials, UploadDestination};

// external crates
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
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// Production [`ObjectTransfer`] that drives the native cloud SDKs: an AWS S3
/// single-part/multipart put for the `s3` scheme, and a GCS put (the SDK picks
/// simple vs resumable) with the vended OAuth2 bearer token for `gcs`. Optional
/// transport settings feed the same store builders as the default HTTPS
/// transport and cloud endpoints.
#[derive(Default)]
pub struct SdkTransfer {
    s3_http_client: Option<aws_sdk_s3::config::SharedHttpClient>,
    s3_force_path_style: bool,
    gcs_endpoint: Option<String>,
}

impl SdkTransfer {
    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) into the S3 store in place of the default HTTPS
    /// connector, so tests serve canned responses without touching the network.
    #[cfg(test)]
    pub(crate) fn with_s3_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
    ) -> Self {
        Self {
            s3_http_client: Some(aws_sdk_s3::config::IntoShared::into_shared(http_client)),
            s3_force_path_style: true,
            gcs_endpoint: None,
        }
    }

    /// Test-only constructor that points the GCS store at a local mock server
    /// instead of the real GCS endpoint, so tests serve canned responses
    /// without touching the network.
    #[cfg(test)]
    pub(crate) fn with_gcs_endpoint(endpoint: String) -> Self {
        Self {
            s3_http_client: None,
            s3_force_path_style: false,
            gcs_endpoint: Some(endpoint),
        }
    }

    /// Uploads `file` to AWS S3 using the vended session credentials. The physical
    /// bucket comes from `destination.bucket_name` (not the Miru `bucket_id`); the
    /// SDK derives the endpoint from the credential `region`.
    async fn transfer_s3(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .s3_credentials
            .as_deref()
            .ok_or_else(|| executor_err(TransferErr::MissingS3Credentials))?;
        let store = self.s3_store(s3_config(creds));
        let object = s3::Object {
            bucket: destination.bucket_name.clone(),
            key: destination.object_key.clone(),
        };
        store
            .put(file.clone(), &object, metadata)
            .await
            .map_err(classified_executor_err)
    }

    /// Uploads `file` to GCS using the vended downscoped OAuth2 bearer token,
    /// which is scoped to `storage.objects.create` on this exact object.
    async fn transfer_gcs(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .gcs_credentials
            .as_deref()
            .ok_or_else(|| executor_err(TransferErr::MissingGcsCredentials))?;
        let store = self
            .gcs_store(gcs::Credentials {
                access_token: creds.access_token.clone(),
            })
            .await
            .map_err(classified_executor_err)?;
        let object = gcs::Object {
            bucket: destination.bucket_name.clone(),
            key: destination.object_key.clone(),
        };
        store
            .put(file.clone(), &object, metadata)
            .await
            .map_err(classified_executor_err)
    }

    /// Builds the S3 store with this transfer's transport settings.
    fn s3_store(&self, cfg: s3::Config) -> s3::Store {
        s3::Store::build(cfg, self.s3_http_client.clone(), self.s3_force_path_style)
    }

    /// Builds the GCS store with this transfer's endpoint settings.
    async fn gcs_store(&self, creds: gcs::Credentials) -> Result<gcs::Store, gcs::GcsErr> {
        gcs::Store::build(creds, self.gcs_endpoint.clone(), None).await
    }
}

impl ObjectTransfer for SdkTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        info!(
            "upload: transferring file {} via scheme {:?} to bucket {} key {}",
            file, credentials.scheme, destination.bucket_name, destination.object_key
        );
        match credentials.scheme {
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
    // standard crates
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // internal crates
    use super::s3_config;
    use crate::test_utils::{
        filesys::{abs_file, files as test_files, missing_file},
        http_client::run_server,
        upload::{destination, response_metadata, s3_credentials_json},
    };
    use backend_api::models::{S3UploadCredentials, UploadCredentials};
    use miru_agent::data_uploads::upload::errors::TransferErr;
    use miru_agent::data_uploads::upload::{ObjectTransfer, SdkTransfer, UploadErr};
    use miru_agent::errors::Error as ErrorTrait;
    use miru_agent::filesys::{files, WriteOptions};

    // external crates
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::Router;
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

    /// Shared state recording what the GCS upload mock server observed.
    #[derive(Clone, Default)]
    struct GcsRecorder {
        inner: Arc<Mutex<GcsRecord>>,
    }

    #[derive(Default)]
    struct GcsRecord {
        upload_hits: usize,
        /// Value of the `Authorization` header on the last upload request.
        authorization: String,
        /// Path-and-query of the last upload request.
        uri: String,
        /// Raw body of the last upload request (multipart framing included).
        body: Vec<u8>,
        /// If set, uploads respond with this status + GCS error JSON instead of
        /// 200 + Object JSON.
        status: Option<StatusCode>,
    }

    /// Handles the single-shot upload `POST /upload/storage/v1/b/{bucket}/o`,
    /// recording the request and returning a minimal `Object` JSON that the
    /// client decodes to finalize (or the configured error status).
    async fn gcs_upload_handler(
        State(rec): State<GcsRecorder>,
        uri: axum::extract::OriginalUri,
        headers: HeaderMap,
        body: Bytes,
    ) -> (
        StatusCode,
        [(axum::http::HeaderName, &'static str); 1],
        String,
    ) {
        let mut r = rec.inner.lock().unwrap();
        r.upload_hits += 1;
        r.authorization = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        r.uri = uri.0.to_string();
        r.body = body.to_vec();
        let json_ct = [(axum::http::header::CONTENT_TYPE, "application/json")];
        if let Some(status) = r.status {
            return (
                status,
                json_ct,
                "{\"error\":{\"code\":403,\"message\":\"denied\"}}".to_string(),
            );
        }
        let object = json!({ "name": "logs/a.log", "bucket": "my-bucket" }).to_string();
        (StatusCode::OK, json_ct, object)
    }

    /// Builds the GCS upload router wired to the recorder.
    fn gcs_router(rec: GcsRecorder) -> Router {
        Router::new()
            .route("/upload/storage/v1/b/{*rest}", post(gcs_upload_handler))
            .with_state(rec)
    }

    #[tokio::test]
    async fn unknown_scheme_is_unsupported() {
        let creds = credentials("something-new", s3_credentials_json(), Value::Null);
        let err = SdkTransfer::default()
            .transfer(
                &creds,
                &destination(),
                &abs_file("data/a.log"),
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
                &abs_file("data/a.log"),
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

        SdkTransfer::with_s3_http_client(replay.clone())
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

        let err = SdkTransfer::with_s3_http_client(replay.clone())
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
                &abs_file("data/a.log"),
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
        // A newline is not a valid HTTP header byte, so the GCS client build fails
        // offline — exercising the gcs arm's error mapping without a network.
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("bad\ntoken"));
        let err = SdkTransfer::default()
            .transfer(
                &creds,
                &destination(),
                &abs_file("data/a.log"),
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
        let rec = GcsRecorder::default();
        let server = run_server(gcs_router(rec.clone())).await;
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

        SdkTransfer::with_gcs_endpoint(server.base_url)
            .transfer(&creds, &destination(), src.file(), &response_metadata())
            .await
            .unwrap();

        let r = rec.inner.lock().unwrap();
        // `my-bucket` in the upload path proves `bucket_name` reached the SDK (not
        // the Miru `bucket_id`), `name=logs%2Fa.log` proves `object_key`, and the
        // bearer token proves the vended credential flows through.
        assert_eq!(
            (r.upload_hits, r.authorization.as_str(), r.uri.as_str()),
            (
                1,
                "Bearer vended-token",
                "/upload/storage/v1/b/my-bucket/o?uploadType=multipart&name=logs%2Fa.log"
            )
        );
        assert!(
            r.body
                .windows(b"hello world".len())
                .any(|w| w == b"hello world"),
            "uploaded multipart body must contain the file bytes"
        );
        // The vended metadata map lands in the object-resource JSON part.
        let contains = |needle: &[u8]| r.body.windows(needle.len()).any(|w| w == needle);
        assert!(
            contains(b"device_id") && contains(b"dvc_1"),
            "uploaded multipart body must contain the metadata map"
        );
    }

    #[tokio::test]
    async fn gcs_put_failure_maps_to_executor_err() {
        let rec = GcsRecorder::default();
        rec.inner.lock().unwrap().status = Some(StatusCode::FORBIDDEN);
        let server = run_server(gcs_router(rec)).await;
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

        let err = SdkTransfer::with_gcs_endpoint(server.base_url)
            .transfer(&creds, &destination(), src.file(), &HashMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(!err.is_network_conn_err());
        assert!(!err.is_terminal());
    }

    #[tokio::test]
    async fn gcs_default_store_fails_offline_on_missing_file() {
        // Default settings and a valid token use the shared GCS builder, which
        // builds successfully offline (the invalid-token test covers its build
        // error path). `put` stats the file before dispatching any request, so a
        // missing local file fails fast offline.
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("valid-token"));
        let missing = missing_file();

        let err = SdkTransfer::default()
            .transfer(&creds, &destination(), &missing, &HashMap::new())
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

    #[tokio::test]
    async fn gcs_connection_failure_is_classified_network() {
        let src = temp_file_with(b"hello world").await;
        let creds = credentials("gcs", Value::Null, gcs_credentials_json("vended-token"));

        // the GCS counterpart of s3_connection_failure_is_classified_network
        let err = SdkTransfer::with_gcs_endpoint("http://127.0.0.1:1".to_string())
            .transfer(&creds, &destination(), src.file(), &HashMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, UploadErr::ExecutorErr(_)), "got: {err:?}");
        assert!(err.is_network_conn_err(), "got: {err:?}");
        assert!(!err.is_terminal());
    }
}
