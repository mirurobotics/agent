// standard crates
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

// internal crates
use crate::mocks::http_client::run_server;
use miru_agent::errors::{Code, Error};
use miru_agent::filesys::file::File;
use miru_agent::filesys::path::PathExt;
use miru_agent::filesys::{dirs, files, WriteOptions};
use miru_agent::gcs::errors::{
    BuildErr, ConnectionErr, LocalIoErr, ObjectNotFoundErr, RequestFailedErr,
};
use miru_agent::gcs::{Credentials, GcsErr, Object, Store};

// external crates
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use google_cloud_gax::error::rpc::{Code as GaxCode, Status};
use google_cloud_gax::error::Error as GaxError;
use google_cloud_gax::response::Response as GaxResponse;
use google_cloud_storage as gcs;
use secrecy::{ExposeSecret, SecretString};

const BUCKET: &str = "test-bucket";

/// Builds an [`Object`] in the test bucket for the given key, keeping call sites
/// terse.
fn obj(key: &str) -> Object {
    Object {
        bucket: BUCKET.to_string(),
        key: key.to_string(),
    }
}

/// Writes `bytes` to a fresh temp file and returns the guard (kept alive so the
/// file is not deleted until the test drops it).
async fn temp_file_with(bytes: &[u8]) -> files::TempFile {
    let tf = files::temp("gcs-test").unwrap();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    tf
}

// ============================ HTTP DATA-PATH MOCK ============================ //
//
// The `Storage` (data) client uploads/downloads over HTTP/JSON. We point it at a
// local axum server via the `endpoint` override. A permissive router matches on
// method + path prefix and records the requests it saw so tests can assert an
// upload/download was issued and that an `Authorization: Bearer ...` header was
// present. Uploads return a minimal decodable GCS v1 `Object` JSON so the client
// finalizes the upload; downloads return canned bytes.

/// Shared state recording what the HTTP mock server observed.
#[derive(Clone, Default)]
struct HttpRecorder {
    inner: Arc<Mutex<HttpRecord>>,
}

#[derive(Default)]
struct HttpRecord {
    upload_hits: usize,
    download_hits: usize,
    saw_bearer: bool,
    /// Path-and-query of the last upload / download request.
    upload_uri: String,
    download_uri: String,
    /// Raw body of the last upload request (multipart framing included).
    upload_body: Vec<u8>,
    /// Bytes to serve for the download body.
    download_body: Vec<u8>,
    /// If set, downloads respond with this status instead of 200 + body.
    download_status: Option<StatusCode>,
    /// If set, downloads send one partial chunk and then break the connection.
    download_truncate: bool,
    /// If set, uploads respond with this status instead of 200 + Object JSON.
    upload_status: Option<StatusCode>,
}

fn bearer_present(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("Bearer "))
        .unwrap_or(false)
}

/// Handles the single-shot upload `POST /upload/storage/v1/b/{bucket}/o`.
/// Returns a minimal `Object` JSON that the client decodes to finalize.
async fn upload_handler(
    State(rec): State<HttpRecorder>,
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
    r.saw_bearer = r.saw_bearer || bearer_present(&headers);
    r.upload_uri = uri.0.to_string();
    r.upload_body = body.to_vec();
    let json_ct = [(axum::http::header::CONTENT_TYPE, "application/json")];
    if let Some(status) = r.upload_status {
        return (
            status,
            json_ct,
            "{\"error\":{\"code\":403,\"message\":\"denied\"}}".to_string(),
        );
    }
    let object = serde_json::json!({
        "name": "artifacts/hello.txt",
        "bucket": BUCKET,
    })
    .to_string();
    (StatusCode::OK, json_ct, object)
}

/// Handles the media download `GET /storage/v1/b/{bucket}/o/{object}`. On
/// success the client's `read_object` finalization requires an
/// `x-goog-generation` response header (the object's generation).
async fn download_handler(
    State(rec): State<HttpRecorder>,
    uri: axum::extract::OriginalUri,
    headers: HeaderMap,
) -> axum::response::Response {
    let (status, body, truncate) = {
        let mut r = rec.inner.lock().unwrap();
        r.download_hits += 1;
        r.saw_bearer = r.saw_bearer || bearer_present(&headers);
        r.download_uri = uri.0.to_string();
        (
            r.download_status,
            r.download_body.clone(),
            r.download_truncate,
        )
    };
    let gen_header = (
        axum::http::HeaderName::from_static("x-goog-generation"),
        "123456",
    );
    if let Some(status) = status {
        return (
            status,
            [gen_header],
            b"{\"error\":{\"code\":404,\"message\":\"Not Found\"}}".to_vec(),
        )
            .into_response();
    }
    if truncate {
        // One partial chunk, then a broken connection mid-body.
        let stream = futures::stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"partial-")),
            Err(std::io::Error::other("connection reset by peer")),
        ]);
        return (
            StatusCode::OK,
            [gen_header],
            axum::body::Body::from_stream(stream),
        )
            .into_response();
    }
    (StatusCode::OK, [gen_header], body).into_response()
}

/// Builds the data-path router and returns it alongside the recorder.
fn http_router(rec: HttpRecorder) -> Router {
    Router::new()
        .route("/upload/storage/v1/b/{*rest}", post(upload_handler))
        .route("/storage/v1/b/{*rest}", get(download_handler))
        .with_state(rec)
}

/// Builds a `Store` pointed at a freshly started HTTP mock server.
async fn http_store(rec: HttpRecorder) -> Store {
    let server = run_server(http_router(rec)).await;
    Store::from_endpoint(Credentials::default(), server.base_url)
        .await
        .unwrap()
}

/// The production constructor performs no real GCS call, so it can be built in a
/// unit test. A token with a byte invalid in an HTTP header value is rejected.
pub mod construction {
    use super::*;

    #[tokio::test]
    async fn new_builds_with_valid_token() {
        let creds = Credentials {
            access_token: SecretString::from("valid-token"),
        };
        let store = Store::new(creds).await;
        assert!(store.is_ok());
    }

    #[tokio::test]
    async fn new_rejects_bad_token() {
        // A newline is not a valid HTTP header value byte.
        let creds = Credentials {
            access_token: SecretString::from("bad\ntoken"),
        };
        let result = Store::new(creds).await;
        match result {
            Err(GcsErr::BuildErr(_)) => {}
            Ok(_) => panic!("expected BuildErr, got Ok"),
            Err(other) => panic!("expected BuildErr, got {other:?}"),
        }
    }

    /// The secret token round-trips: the exposed value equals what was set. Guards
    /// the `SecretString` wiring feeding the `Bearer` authorization header.
    #[test]
    fn credentials_expose_secret_round_trip() {
        let creds = Credentials {
            access_token: SecretString::from("valid-token"),
        };
        assert_eq!(creds.access_token.expose_secret(), "valid-token");
    }
}

pub mod put {
    use super::*;

    #[tokio::test]
    async fn upload_streams_file_body() {
        let rec = HttpRecorder::default();
        let store = http_store(rec.clone()).await;
        let src = temp_file_with(b"hello world").await;

        store
            .put(src.to_file(), &obj("artifacts/hello.txt"))
            .await
            .unwrap();

        let r = rec.inner.lock().unwrap();
        assert_eq!(r.upload_hits, 1);
        assert!(r.saw_bearer, "upload must carry Authorization: Bearer");
        assert_eq!(
            r.upload_uri,
            "/upload/storage/v1/b/test-bucket/o?uploadType=multipart&name=artifacts%2Fhello.txt"
        );
        assert!(
            r.upload_body
                .windows(b"hello world".len())
                .any(|w| w == b"hello world"),
            "uploaded multipart body must contain the file bytes"
        );
    }

    #[tokio::test]
    async fn upload_empty_file_succeeds() {
        // A 0-byte source still issues exactly one upload.
        let rec = HttpRecorder::default();
        let store = http_store(rec.clone()).await;
        let src = temp_file_with(b"").await;

        store
            .put(src.to_file(), &obj("artifacts/empty.txt"))
            .await
            .unwrap();

        assert_eq!(rec.inner.lock().unwrap().upload_hits, 1);
    }

    pub mod access_denied {
        use super::*;

        #[tokio::test]
        async fn upload_error_maps_to_request_failed() {
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().upload_status = Some(StatusCode::FORBIDDEN);
            let store = http_store(rec).await;
            let src = temp_file_with(b"payload").await;

            let err = store
                .put(src.to_file(), &obj("denied.txt"))
                .await
                .unwrap_err();

            // The mapper must carry the HTTP status through, not just pick
            // the variant.
            match err {
                GcsErr::RequestFailedErr(e) => assert_eq!(e.status, Some(403)),
                other => panic!("expected RequestFailedErr, got {other:?}"),
            }
        }
    }

    pub mod source_missing {
        use super::*;

        #[tokio::test]
        async fn upload_missing_source_maps_to_filesys_err() {
            let rec = HttpRecorder::default();
            let store = http_store(rec.clone()).await;
            let missing = File::new("/nonexistent/definitely/not/here.bin");

            let err = store.put(missing, &obj("k")).await.unwrap_err();

            assert!(matches!(err, GcsErr::FileSysErr(_)));
            assert_eq!(rec.inner.lock().unwrap().upload_hits, 0);
        }
    }

    pub mod source_unreadable {
        use super::*;

        #[cfg(unix)]
        #[tokio::test]
        async fn upload_unreadable_source_maps_to_local_io_err() {
            // A source that stats fine but cannot be opened for reading passes
            // the up-front size read and fails at the open, surfacing as
            // `LocalIoErr` before any request is dispatched.
            let rec = HttpRecorder::default();
            let store = http_store(rec.clone()).await;
            let src = temp_file_with(b"secret").await;
            files::set_permissions(src.file(), std::fs::Permissions::from_mode(0o000))
                .await
                .unwrap();

            let err = store.put(src.to_file(), &obj("k")).await.unwrap_err();

            assert!(matches!(err, GcsErr::LocalIoErr(_)));
            assert_eq!(rec.inner.lock().unwrap().upload_hits, 0);
        }
    }
}

pub mod get {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        async fn download_streams_body_to_file() {
            let payload = b"\x00\x01\x02binary-body\xff".to_vec();
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_body = payload.clone();
            let store = http_store(rec.clone()).await;
            let dest = files::temp("gcs-dest").unwrap();

            store
                .get(&obj("blobs/data.bin"), dest.file())
                .await
                .unwrap();

            assert_eq!(files::read_bytes(dest.file()).await.unwrap(), payload);
            let r = rec.inner.lock().unwrap();
            assert_eq!(r.download_hits, 1);
            assert!(r.saw_bearer, "download must carry Authorization: Bearer");
            assert_eq!(
                r.download_uri,
                "/storage/v1/b/test-bucket/o/blobs%2Fdata.bin?alt=media"
            );
        }

        #[tokio::test]
        async fn download_overwrites_existing_destination() {
            // Stale dest content is LONGER than the new payload, so a failure to
            // truncate would leave trailing bytes.
            let payload = b"NEW".to_vec();
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_body = payload.clone();
            let store = http_store(rec).await;
            let dest = files::temp("gcs-dest").unwrap();
            files::write_bytes(
                dest.file(),
                b"OLD-STALE-CONTENT",
                WriteOptions::OVERWRITE_NONATOMIC,
            )
            .await
            .unwrap();

            store
                .get(&obj("blobs/data.bin"), dest.file())
                .await
                .unwrap();

            assert_eq!(files::read_bytes(dest.file()).await.unwrap(), payload);
        }

        #[tokio::test]
        async fn download_empty_object_writes_empty_file() {
            // The recorder's default body is zero bytes.
            let rec = HttpRecorder::default();
            let store = http_store(rec).await;
            let dest = files::temp("gcs-dest").unwrap();

            store
                .get(&obj("blobs/empty.bin"), dest.file())
                .await
                .unwrap();

            assert!(dest.file().path().exists());
            assert!(files::read_bytes(dest.file()).await.unwrap().is_empty());
        }
    }

    pub mod mid_stream {
        use super::*;

        #[tokio::test]
        async fn download_body_error_maps_to_connection_err() {
            // The connection breaks after one body chunk: a retryable network
            // condition. A partial destination file may remain; the next get's
            // File::create truncates it.
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_truncate = true;
            let store = http_store(rec.clone()).await;
            let dest = files::temp("gcs-dest").unwrap();

            let err = store
                .get(&obj("blobs/data.bin"), dest.file())
                .await
                .unwrap_err();

            assert!(err.is_network_conn_err(), "expected network error: {err}");
            assert!(matches!(err, GcsErr::ConnectionErr(_)));
            let leftover = files::read_bytes(dest.file()).await.unwrap();
            assert!(leftover.len() <= b"partial-".len());
        }
    }

    pub mod dest_unwritable {
        use super::*;

        #[tokio::test]
        async fn download_to_unwritable_dest_maps_to_local_io_err() {
            let payload = b"body".to_vec();
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_body = payload;
            let store = http_store(rec).await;
            // The destination's parent directory does not exist, so creating the file
            // fails after the object is fetched.
            let dest = File::new("/nonexistent/dir/out.bin");

            let err = store.get(&obj("blobs/data.bin"), &dest).await.unwrap_err();

            assert!(matches!(err, GcsErr::LocalIoErr(_)));
            // Creating the file failed, so nothing was written at `dest`.
            assert!(!dest.path().exists());
        }

        #[tokio::test]
        async fn download_to_missing_parent_dir_maps_to_local_io_err() {
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_body = b"body".to_vec();
            let store = http_store(rec).await;
            let tmp = dirs::temp("gcs-dest").unwrap();
            let dest = File::new(tmp.path().join("no-such-dir").join("out.bin"));

            let err = store.get(&obj("blobs/data.bin"), &dest).await.unwrap_err();

            assert!(matches!(err, GcsErr::LocalIoErr(_)));
            assert!(!dest.path().exists());
        }
    }

    pub mod not_found {
        use super::*;

        #[tokio::test]
        async fn download_missing_maps_to_not_found() {
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_status = Some(StatusCode::NOT_FOUND);
            let store = http_store(rec).await;
            let dest = files::temp("gcs-dest").unwrap();

            let err = store
                .get(&obj("missing.txt"), dest.file())
                .await
                .unwrap_err();

            assert!(matches!(err, GcsErr::ObjectNotFoundErr(_)));
        }
    }

    pub mod access_denied {
        use super::*;

        #[tokio::test]
        async fn download_error_maps_to_request_failed() {
            // A non-404 download failure (403) delegates to the common mapper.
            let rec = HttpRecorder::default();
            rec.inner.lock().unwrap().download_status = Some(StatusCode::FORBIDDEN);
            let store = http_store(rec).await;
            let dest = files::temp("gcs-dest").unwrap();

            let err = store
                .get(&obj("denied.txt"), dest.file())
                .await
                .unwrap_err();

            assert!(matches!(err, GcsErr::RequestFailedErr(_)));
        }
    }

    // note: the axum data-path mock finalizes each download via the `x-goog-generation`
    // response header, so truncated-body and dispatch-failure conditions cannot be
    // simulated with it and are not covered here.
}

// ============================ gRPC CONTROL-PATH MOCK ============================ //
//
// The `StorageControl` client's delete/get-object go over gRPC. Rather than
// stand up a tonic server, we mock the public `gcs::stub::StorageControl` trait
// with `mockall` and inject it via `Store::from_stub` (which builds the control
// client with `StorageControl::from_stub`). Only `delete_object` and
// `get_object` are ever exercised; the rest of the 33-method trait surface must
// be listed for `mockall` but is never called.

mockall::mock! {
    #[derive(Debug)]
    StorageControl {}
    impl gcs::stub::StorageControl for StorageControl {
    async fn delete_bucket( &self, _req: gcs::model::DeleteBucketRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<()>>;
    async fn get_bucket( &self, _req: gcs::model::GetBucketRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Bucket>>;
    async fn create_bucket( &self, _req: gcs::model::CreateBucketRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Bucket>>;
    async fn list_buckets( &self, _req: gcs::model::ListBucketsRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ListBucketsResponse>>;
    async fn lock_bucket_retention_policy( &self, _req: gcs::model::LockBucketRetentionPolicyRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Bucket>>;
    async fn get_iam_policy( &self, _req: google_cloud_iam_v1::model::GetIamPolicyRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_iam_v1::model::Policy>>;
    async fn set_iam_policy( &self, _req: google_cloud_iam_v1::model::SetIamPolicyRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_iam_v1::model::Policy>>;
    async fn test_iam_permissions( &self, _req: google_cloud_iam_v1::model::TestIamPermissionsRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_iam_v1::model::TestIamPermissionsResponse>>;
    async fn update_bucket( &self, _req: gcs::model::UpdateBucketRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Bucket>>;
    async fn compose_object( &self, _req: gcs::model::ComposeObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Object>>;
    async fn delete_object( &self, _req: gcs::model::DeleteObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<()>>;
    async fn restore_object( &self, _req: gcs::model::RestoreObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Object>>;
    async fn get_object( &self, _req: gcs::model::GetObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Object>>;
    async fn update_object( &self, _req: gcs::model::UpdateObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Object>>;
    async fn list_objects( &self, _req: gcs::model::ListObjectsRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ListObjectsResponse>>;
    async fn rewrite_object( &self, _req: gcs::model::RewriteObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::RewriteResponse>>;
    async fn move_object( &self, _req: gcs::model::MoveObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Object>>;
    async fn create_folder( &self, _req: gcs::model::CreateFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Folder>>;
    async fn delete_folder( &self, _req: gcs::model::DeleteFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<()>>;
    async fn get_folder( &self, _req: gcs::model::GetFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Folder>>;
    async fn list_folders( &self, _req: gcs::model::ListFoldersRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ListFoldersResponse>>;
    async fn rename_folder( &self, _req: gcs::model::RenameFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_longrunning::model::Operation>>;
    async fn get_storage_layout( &self, _req: gcs::model::GetStorageLayoutRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::StorageLayout>>;
    async fn create_managed_folder( &self, _req: gcs::model::CreateManagedFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ManagedFolder>>;
    async fn delete_managed_folder( &self, _req: gcs::model::DeleteManagedFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<()>>;
    async fn get_managed_folder( &self, _req: gcs::model::GetManagedFolderRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ManagedFolder>>;
    async fn list_managed_folders( &self, _req: gcs::model::ListManagedFoldersRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ListManagedFoldersResponse>>;
    async fn create_anywhere_cache( &self, _req: gcs::model::CreateAnywhereCacheRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_longrunning::model::Operation>>;
    async fn update_anywhere_cache( &self, _req: gcs::model::UpdateAnywhereCacheRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_longrunning::model::Operation>>;
    async fn disable_anywhere_cache( &self, _req: gcs::model::DisableAnywhereCacheRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::AnywhereCache>>;
    async fn pause_anywhere_cache( &self, _req: gcs::model::PauseAnywhereCacheRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::AnywhereCache>>;
    async fn resume_anywhere_cache( &self, _req: gcs::model::ResumeAnywhereCacheRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::AnywhereCache>>;
    async fn get_anywhere_cache( &self, _req: gcs::model::GetAnywhereCacheRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::AnywhereCache>>;
    async fn list_anywhere_caches( &self, _req: gcs::model::ListAnywhereCachesRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::ListAnywhereCachesResponse>>;
    async fn get_folder_intelligence_config( &self, _req: gcs::model::GetFolderIntelligenceConfigRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::IntelligenceConfig>>;
    async fn update_folder_intelligence_config( &self, _req: gcs::model::UpdateFolderIntelligenceConfigRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::IntelligenceConfig>>;
    async fn get_project_intelligence_config( &self, _req: gcs::model::GetProjectIntelligenceConfigRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::IntelligenceConfig>>;
    async fn update_project_intelligence_config( &self, _req: gcs::model::UpdateProjectIntelligenceConfigRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::IntelligenceConfig>>;
    async fn get_organization_intelligence_config( &self, _req: gcs::model::GetOrganizationIntelligenceConfigRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::IntelligenceConfig>>;
    async fn update_organization_intelligence_config( &self, _req: gcs::model::UpdateOrganizationIntelligenceConfigRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::IntelligenceConfig>>;
    async fn get_operation( &self, _req: google_cloud_longrunning::model::GetOperationRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<google_cloud_longrunning::model::Operation>>;
    }
}

/// Builds a `Store` whose control client is the given mock. The data client is
/// pointed at an unused loopback endpoint (never called by delete/exists).
async fn control_store(mock: MockStorageControl) -> Store {
    Store::from_stub(
        mock,
        Credentials::default(),
        "http://127.0.0.1:0".to_string(),
    )
    .await
    .unwrap()
}

fn not_found_err() -> GaxError {
    GaxError::service(Status::default().set_code(GaxCode::NotFound))
}

fn permission_denied_err() -> GaxError {
    GaxError::service(Status::default().set_code(GaxCode::PermissionDenied))
}

pub mod delete {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        async fn delete_removes_object() {
            let mut mock = MockStorageControl::new();
            mock.expect_delete_object()
                .times(1)
                .withf(|req, _| {
                    req.bucket == "projects/_/buckets/test-bucket" && req.object == "blobs/data.bin"
                })
                .returning(|_, _| Ok(GaxResponse::from(())));
            let store = control_store(mock).await;

            store.delete(&obj("blobs/data.bin")).await.unwrap();
        }

        #[tokio::test]
        async fn delete_missing_is_idempotent() {
            let mut mock = MockStorageControl::new();
            mock.expect_delete_object()
                .times(1)
                .returning(|_, _| Err(not_found_err()));
            let store = control_store(mock).await;

            // A NOT_FOUND delete is a success (idempotent).
            store.delete(&obj("missing.txt")).await.unwrap();
        }
    }

    pub mod access_denied {
        use super::*;

        #[tokio::test]
        async fn delete_error_maps_to_request_failed() {
            let mut mock = MockStorageControl::new();
            mock.expect_delete_object()
                .times(1)
                .returning(|_, _| Err(permission_denied_err()));
            let store = control_store(mock).await;

            let err = store.delete(&obj("denied.txt")).await.unwrap_err();

            assert!(matches!(err, GcsErr::RequestFailedErr(_)));
        }
    }
}

pub mod exists {
    use super::*;

    pub mod present {
        use super::*;

        #[tokio::test]
        async fn present_returns_true() {
            let mut mock = MockStorageControl::new();
            mock.expect_get_object()
                .times(1)
                .withf(|req, _| {
                    req.bucket == "projects/_/buckets/test-bucket" && req.object == "blobs/data.bin"
                })
                .returning(|_, _| Ok(GaxResponse::from(gcs::model::Object::default())));
            let store = control_store(mock).await;

            assert!(store.exists(&obj("blobs/data.bin")).await.unwrap());
        }
    }

    pub mod absent {
        use super::*;

        #[tokio::test]
        async fn absent_returns_false() {
            let mut mock = MockStorageControl::new();
            mock.expect_get_object()
                .times(1)
                .returning(|_, _| Err(not_found_err()));
            let store = control_store(mock).await;

            assert!(!store.exists(&obj("missing.txt")).await.unwrap());
        }
    }

    pub mod access_denied {
        use super::*;

        #[tokio::test]
        async fn error_propagates() {
            let mut mock = MockStorageControl::new();
            mock.expect_get_object()
                .times(1)
                .returning(|_, _| Err(permission_denied_err()));
            let store = control_store(mock).await;

            let err = store.exists(&obj("denied.txt")).await.unwrap_err();

            assert!(matches!(err, GcsErr::RequestFailedErr(_)));
        }
    }
}

/// Direct assertions on the leaf error types' trait behavior. These pin the
/// `crate::errors::Error` contract each variant promises (code / http_status /
/// is_network_conn_err / Display).
pub mod error_types {
    use super::*;

    #[test]
    fn object_not_found_maps_to_resource_not_found() {
        let err = GcsErr::ObjectNotFoundErr(ObjectNotFoundErr {
            object: obj("k"),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::ResourceNotFound));
        assert_eq!(err.http_status().as_u16(), 404);
        assert!(!err.is_network_conn_err());
        assert!(err.to_string().contains("object not found"));
    }

    #[test]
    fn connection_err_is_network_conn_err() {
        let err = GcsErr::ConnectionErr(ConnectionErr {
            object: obj("k"),
            msg: "boom".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(err.is_network_conn_err());
        assert!(matches!(err.code(), Code::InternalServerError));
        assert!(err.to_string().contains("connection error"));
    }

    #[test]
    fn request_failed_err_defaults_to_internal_server_error() {
        let err = GcsErr::RequestFailedErr(RequestFailedErr {
            operation: "get_object".to_string(),
            object: obj("key"),
            status: None,
            msg: "nope".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(!err.is_network_conn_err());
        // Display with no status hits the fallback formatting.
        let msg = err.to_string();
        assert!(msg.contains("gs://test-bucket/key"));
        assert!(msg.contains("unknown"));
        assert!(msg.contains("get_object"));
    }

    #[test]
    fn build_err_defaults_to_internal_server_error() {
        let err = GcsErr::BuildErr(BuildErr {
            msg: "bad token".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(err.to_string().contains("failed to build GCS client"));
    }

    #[test]
    fn local_io_err_defaults_to_internal_server_error() {
        let err = GcsErr::LocalIoErr(LocalIoErr {
            operation: "get_object".to_string(),
            object: obj("key"),
            msg: "no such file or directory".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(!err.is_network_conn_err());
        let msg = err.to_string();
        assert!(msg.contains("gs://test-bucket/key"));
        assert!(msg.contains("no such file or directory"));
    }
}
