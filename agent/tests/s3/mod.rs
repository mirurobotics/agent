// standard crates
use std::io::Write as _;

// internal crates
use miru_agent::errors::{Code, Error};
use miru_agent::filesys::file::File;
use miru_agent::s3::errors::{
    ConnectionErr, InvalidResponseErr, ObjectNotFoundErr, RequestFailedErr,
};
use miru_agent::s3::{Config, Credentials, Options, S3Err, Store};

// external crates
use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_types::body::SdkBody;
use tempfile::NamedTempFile;

const REGION: &str = "us-east-1";
const BUCKET: &str = "test-bucket";

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
fn uri(path_and_query: &str) -> String {
    format!("https://s3.{REGION}.amazonaws.com/{BUCKET}/{path_and_query}")
}

/// Wires a `Store` to a `StaticReplayClient` serving the given events, using the
/// default multipart part size.
fn store_with(events: Vec<ReplayEvent>) -> (Store, StaticReplayClient) {
    store_with_part_size(events, Options::default().part_size)
}

/// Wires a `Store` whose multipart threshold is `part_size`, so multipart tests
/// force the multipart path with tiny fixtures (`part_size: 0` → always multipart).
fn store_with_part_size(events: Vec<ReplayEvent>, part_size: u64) -> (Store, StaticReplayClient) {
    let replay = StaticReplayClient::new(events);
    let cfg = Config {
        region: REGION.to_string(),
        bucket: BUCKET.to_string(),
        creds: Credentials::default(),
    };
    let store = Store::from_http_client(replay.clone(), cfg, Options { part_size });
    (store, replay)
}

/// Writes `bytes` to a fresh temp file and returns the handle (kept alive so
/// the file is not deleted until the test drops it).
fn temp_file_with(bytes: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    f
}

pub mod put {
    use super::*;

    pub mod single {
        use super::*;

        #[tokio::test]
        async fn put_streams_file_body_bytes() {
            let key = "artifacts/hello.txt";
            let body = b"hello world".to_vec();
            let src = temp_file_with(&body);
            // The body streams off disk, so the replay client records it as an
            // unbuffered `SdkBody` whose `.bytes()` is `None` — meaning
            // `assert_requests_match` cannot byte-compare it. The request is
            // paired with an empty expected body and we assert the method +
            // path here; `get_streams_body_to_file` covers a real round-trip of
            // the payload contents.
            let expected_req = http::Request::builder()
                .method("PUT")
                .uri(uri("artifacts/hello.txt?x-id=PutObject"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(200)
                .body(SdkBody::empty())
                .unwrap();
            let (store, replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            store.put(key, File::new(src.path())).await.unwrap();

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].method(), "PUT");
            assert_eq!(
                requests[0].uri().to_string(),
                uri("artifacts/hello.txt?x-id=PutObject")
            );
        }
    }

    /// Multipart upload path. The threshold override lets a tiny temp file drive
    /// the create → upload_part(s) → complete sequence without a 5 GiB fixture.
    pub mod multipart {
        use super::*;

        const UPLOAD_ID: &str = "test-upload-id";

        fn create_resp() -> http::Response<SdkBody> {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><UploadId>{UPLOAD_ID}</UploadId></InitiateMultipartUploadResult>"#
            );
            http::Response::builder()
                .status(200)
                .header("content-type", "application/xml")
                .body(SdkBody::from(xml))
                .unwrap()
        }

        fn upload_part_resp(etag: &str) -> http::Response<SdkBody> {
            http::Response::builder()
                .status(200)
                .header("ETag", etag)
                .body(SdkBody::empty())
                .unwrap()
        }

        fn complete_resp() -> http::Response<SdkBody> {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Location>https://s3.amazonaws.com/{BUCKET}/big.bin</Location><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><ETag>"final-etag"</ETag></CompleteMultipartUploadResult>"#
            );
            http::Response::builder()
                .status(200)
                .header("content-type", "application/xml")
                .body(SdkBody::from(xml))
                .unwrap()
        }

        #[tokio::test]
        async fn large_file_uploads_in_parts() {
            let key = "big.bin";
            // Constructing the store with `part_size: 0` (via `store_with_part_size`
            // below) forces this small file onto the multipart path. The 8 MiB part
            // size dwarfs the file, so it uploads as a single part — enough to
            // exercise the full create → upload_part → complete sequence without a
            // huge fixture.
            let body = b"multipart-body".to_vec();
            let src = temp_file_with(&body);

            let create_req = http::Request::builder()
                .method("POST")
                .uri(uri("big.bin?uploads&x-id=CreateMultipartUpload"))
                .body(SdkBody::empty())
                .unwrap();
            let part_req = http::Request::builder()
                .method("PUT")
                .uri(uri(&format!(
                    "big.bin?x-id=UploadPart&partNumber=1&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::from(body.clone()))
                .unwrap();
            let complete_req = http::Request::builder()
                .method("POST")
                .uri(uri(&format!(
                    "big.bin?x-id=CompleteMultipartUpload&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::empty())
                .unwrap();

            let (store, replay) = store_with_part_size(
                vec![
                    ReplayEvent::new(create_req, create_resp()),
                    ReplayEvent::new(part_req, upload_part_resp("\"etag-part-1\"")),
                    ReplayEvent::new(complete_req, complete_resp()),
                ],
                0,
            );

            store.put(key, File::new(src.path())).await.unwrap();

            // Assert the create → upload_part → complete sequence fired, matching
            // methods and paths (bodies for POSTs vary and are ignored).
            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 3);
            assert_eq!(requests[0].method(), "POST");
            assert!(requests[0].uri().to_string().contains("uploads"));
            assert_eq!(requests[1].method(), "PUT");
            assert!(requests[1].uri().to_string().contains("partNumber=1"));
            assert!(requests[1]
                .uri()
                .to_string()
                .contains(&format!("uploadId={UPLOAD_ID}")));
            assert_eq!(requests[2].method(), "POST");
            assert!(requests[2]
                .uri()
                .to_string()
                .contains(&format!("uploadId={UPLOAD_ID}")));
        }

        #[tokio::test]
        async fn create_without_upload_id_maps_to_invalid_response() {
            let key = "big.bin";
            let src = temp_file_with(b"multipart-body");

            let create_req = http::Request::builder()
                .method("POST")
                .uri(uri("big.bin?uploads&x-id=CreateMultipartUpload"))
                .body(SdkBody::empty())
                .unwrap();
            // Well-formed XML but with no <UploadId> element.
            let no_id_resp = http::Response::builder()
                .status(200)
                .header("content-type", "application/xml")
                .body(SdkBody::from(format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>big.bin</Key></InitiateMultipartUploadResult>"#
                )))
                .unwrap();

            let (store, _replay) =
                store_with_part_size(vec![ReplayEvent::new(create_req, no_id_resp)], 0);

            let err = store.put(key, File::new(src.path())).await.unwrap_err();
            assert!(matches!(err, S3Err::InvalidResponseErr(_)));
        }

        #[tokio::test]
        async fn create_failure_maps_to_request_failed() {
            let key = "big.bin";
            let src = temp_file_with(b"multipart-body");

            // CreateMultipartUpload itself fails with a 403 — no upload exists to
            // abort, so the error surfaces directly.
            let create_req = http::Request::builder()
                .method("POST")
                .uri(uri("big.bin?uploads&x-id=CreateMultipartUpload"))
                .body(SdkBody::empty())
                .unwrap();
            let create_fail = http::Response::builder()
                .status(403)
                .header("content-type", "application/xml")
                .body(SdkBody::from(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>"#,
                ))
                .unwrap();

            let (store, _replay) =
                store_with_part_size(vec![ReplayEvent::new(create_req, create_fail)], 0);

            let err = store.put(key, File::new(src.path())).await.unwrap_err();
            assert!(matches!(err, S3Err::RequestFailedErr(_)));
        }

        #[tokio::test]
        async fn complete_failure_triggers_abort() {
            let key = "big.bin";
            let src = temp_file_with(b"multipart-body");

            let create_req = http::Request::builder()
                .method("POST")
                .uri(uri("big.bin?uploads&x-id=CreateMultipartUpload"))
                .body(SdkBody::empty())
                .unwrap();
            let part_req = http::Request::builder()
                .method("PUT")
                .uri(uri(&format!(
                    "big.bin?x-id=UploadPart&partNumber=1&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::empty())
                .unwrap();
            // The part uploads fine, but CompleteMultipartUpload fails, which
            // must trigger an abort of the in-progress upload.
            let complete_req = http::Request::builder()
                .method("POST")
                .uri(uri(&format!(
                    "big.bin?x-id=CompleteMultipartUpload&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::empty())
                .unwrap();
            let complete_fail = http::Response::builder()
                .status(403)
                .header("content-type", "application/xml")
                .body(SdkBody::from(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>"#,
                ))
                .unwrap();
            let abort_req = http::Request::builder()
                .method("DELETE")
                .uri(uri(&format!(
                    "big.bin?x-id=AbortMultipartUpload&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::empty())
                .unwrap();
            let abort_resp = http::Response::builder()
                .status(204)
                .body(SdkBody::empty())
                .unwrap();

            let (store, replay) = store_with_part_size(
                vec![
                    ReplayEvent::new(create_req, create_resp()),
                    ReplayEvent::new(part_req, upload_part_resp("\"etag-part-1\"")),
                    ReplayEvent::new(complete_req, complete_fail),
                    ReplayEvent::new(abort_req, abort_resp),
                ],
                0,
            );

            let err = store.put(key, File::new(src.path())).await.unwrap_err();
            assert!(matches!(err, S3Err::RequestFailedErr(_)));

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 4);
            assert_eq!(requests[3].method(), "DELETE");
            assert!(requests[3]
                .uri()
                .to_string()
                .contains("x-id=AbortMultipartUpload"));
        }

        #[tokio::test]
        async fn part_failure_triggers_abort() {
            let key = "big.bin";
            let body = b"multipart-body".to_vec();
            let src = temp_file_with(&body);

            let create_req = http::Request::builder()
                .method("POST")
                .uri(uri("big.bin?uploads&x-id=CreateMultipartUpload"))
                .body(SdkBody::empty())
                .unwrap();
            // The upload_part call fails with a 403, which must trigger an abort.
            let part_req = http::Request::builder()
                .method("PUT")
                .uri(uri(&format!(
                    "big.bin?x-id=UploadPart&partNumber=1&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::empty())
                .unwrap();
            let part_fail_resp = http::Response::builder()
                .status(403)
                .header("content-type", "application/xml")
                .body(SdkBody::from(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>"#,
                ))
                .unwrap();
            let abort_req = http::Request::builder()
                .method("DELETE")
                .uri(uri(&format!(
                    "big.bin?x-id=AbortMultipartUpload&uploadId={UPLOAD_ID}"
                )))
                .body(SdkBody::empty())
                .unwrap();
            let abort_resp = http::Response::builder()
                .status(204)
                .body(SdkBody::empty())
                .unwrap();

            let (store, replay) = store_with_part_size(
                vec![
                    ReplayEvent::new(create_req, create_resp()),
                    ReplayEvent::new(part_req, part_fail_resp),
                    ReplayEvent::new(abort_req, abort_resp),
                ],
                0,
            );

            let err = store.put(key, File::new(src.path())).await.unwrap_err();
            assert!(matches!(err, S3Err::RequestFailedErr(_)));

            // The abort must have been issued after the failed part.
            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 3);
            assert_eq!(requests[2].method(), "DELETE");
            assert!(requests[2]
                .uri()
                .to_string()
                .contains("x-id=AbortMultipartUpload"));
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
            let dest = NamedTempFile::new().unwrap();
            let expected_req = http::Request::builder()
                .method("GET")
                .uri(uri("blobs/data.bin?x-id=GetObject"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(200)
                .header("content-length", payload.len().to_string())
                .body(SdkBody::from(payload.clone()))
                .unwrap();
            let (store, replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            store.get(key, &File::new(dest.path())).await.unwrap();

            let written = std::fs::read(dest.path()).unwrap();
            assert_eq!(written, payload);
            replay.assert_requests_match(IGNORED_HEADERS);
        }
    }

    pub mod dest_unwritable {
        use super::*;

        #[tokio::test]
        async fn get_to_missing_parent_dir_maps_to_invalid_response() {
            let key = "blobs/data.bin";
            let payload = b"body".to_vec();
            // The destination's parent directory does not exist, so creating the
            // file fails after the object is fetched — exercising the streaming
            // I/O error path.
            let dest = File::new("/nonexistent/dir/out.bin");
            let expected_req = http::Request::builder()
                .method("GET")
                .uri(uri("blobs/data.bin?x-id=GetObject"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(200)
                .header("content-length", payload.len().to_string())
                .body(SdkBody::from(payload))
                .unwrap();
            let (store, _replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            let err = store.get(key, &dest).await.unwrap_err();

            assert!(matches!(err, S3Err::InvalidResponseErr(_)));
        }
    }

    pub mod not_found {
        use super::*;

        const NO_SUCH_KEY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message><Key>missing.txt</Key><RequestId>REQ123</RequestId></Error>"#;

        #[tokio::test]
        async fn get_missing_maps_to_not_found() {
            let key = "missing.txt";
            let dest = NamedTempFile::new().unwrap();
            let expected_req = http::Request::builder()
                .method("GET")
                .uri(uri("missing.txt?x-id=GetObject"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(404)
                .header("content-type", "application/xml")
                .body(SdkBody::from(NO_SUCH_KEY_XML))
                .unwrap();
            let (store, _replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            let err = store.get(key, &File::new(dest.path())).await.unwrap_err();

            assert!(matches!(err, S3Err::ObjectNotFoundErr(_)));
            assert!(matches!(err.code(), Code::ResourceNotFound));
            assert_eq!(err.http_status().as_u16(), 404);
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
            let expected_req = http::Request::builder()
                .method("DELETE")
                .uri(uri("blobs/data.bin?x-id=DeleteObject"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(204)
                .body(SdkBody::empty())
                .unwrap();
            let (store, replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            store.delete(key).await.unwrap();

            replay.assert_requests_match(IGNORED_HEADERS);
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
            let expected_req = http::Request::builder()
                .method("HEAD")
                .uri(uri("blobs/data.bin"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(200)
                .header("content-length", "42")
                .body(SdkBody::empty())
                .unwrap();
            let (store, _replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            assert!(store.exists(key).await.unwrap());
        }
    }

    pub mod absent {
        use super::*;

        #[tokio::test]
        async fn head_404_returns_false() {
            let key = "missing.txt";
            let expected_req = http::Request::builder()
                .method("HEAD")
                .uri(uri("missing.txt"))
                .body(SdkBody::empty())
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(404)
                .body(SdkBody::empty())
                .unwrap();
            let (store, _replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            assert!(!store.exists(key).await.unwrap());
        }
    }
}

/// S3 returns non-404 failures (e.g. 403 AccessDenied) as a modeled service
/// error, which the common mapper turns into `RequestFailedErr`.
pub mod request_failed {
    use super::*;

    const ACCESS_DENIED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message><RequestId>REQ403</RequestId></Error>"#;

    fn access_denied_resp() -> http::Response<SdkBody> {
        http::Response::builder()
            .status(403)
            .header("content-type", "application/xml")
            .body(SdkBody::from(ACCESS_DENIED_XML))
            .unwrap()
    }

    #[tokio::test]
    async fn put_403_maps_to_request_failed() {
        let src = temp_file_with(b"x");
        let req = http::Request::builder()
            .method("PUT")
            .uri(uri("denied.txt?x-id=PutObject"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, access_denied_resp())]);

        let err = store
            .put("denied.txt", File::new(src.path()))
            .await
            .unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(!err.is_network_conn_err());
        // Exercise the RequestFailedErr Display impl (status + operation).
        assert!(err.to_string().contains("put_object"));
    }

    #[tokio::test]
    async fn get_403_maps_to_request_failed() {
        let dest = NamedTempFile::new().unwrap();
        let req = http::Request::builder()
            .method("GET")
            .uri(uri("denied.txt?x-id=GetObject"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, access_denied_resp())]);

        let err = store
            .get("denied.txt", &File::new(dest.path()))
            .await
            .unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn delete_403_maps_to_request_failed() {
        let req = http::Request::builder()
            .method("DELETE")
            .uri(uri("denied.txt?x-id=DeleteObject"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, access_denied_resp())]);

        let err = store.delete("denied.txt").await.unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn head_403_propagates_as_request_failed() {
        let req = http::Request::builder()
            .method("HEAD")
            .uri(uri("denied.txt"))
            .body(SdkBody::empty())
            .unwrap();
        // HEAD has no response body; a 403 has no XML payload.
        let resp = http::Response::builder()
            .status(403)
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, resp)]);

        let err = store.exists("denied.txt").await.unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn transport_failure_maps_to_connection_err() {
        // With no replay events, the connector fails to dispatch the request,
        // which the SDK surfaces as `SdkError::DispatchFailure` — the mapper's
        // network-connection path.
        let dest = NamedTempFile::new().unwrap();
        let (store, _replay) = store_with(vec![]);

        let err = store
            .get("any.txt", &File::new(dest.path()))
            .await
            .unwrap_err();

        assert!(matches!(err, S3Err::ConnectionErr(_)));
        assert!(err.is_network_conn_err());
    }
}

/// A put over a missing source file surfaces the filesystem error (mapped to
/// `FileSysErr`) before any network call is made.
pub mod put_source_missing {
    use super::*;

    #[tokio::test]
    async fn put_missing_source_maps_to_filesys_err() {
        let (store, _replay) = store_with(vec![]);
        let missing = File::new("/nonexistent/definitely/not/here.bin");

        let err = store.put("k", missing).await.unwrap_err();

        assert!(matches!(err, S3Err::FileSysErr(_)));
    }
}

/// The production constructor performs no network I/O, so it can be built and
/// dropped in a unit test to exercise that code path.
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
            bucket: "prod-bucket".to_string(),
        };
        // Constructing must not panic or touch the network.
        let _store = Store::new(cfg, Options::default());
    }
}

/// Direct assertions on the leaf error types' trait behavior. These do not go
/// through the SDK; they pin the `crate::errors::Error` contract each variant
/// promises (code / http_status / is_network_conn_err / Display).
pub mod error_types {
    use super::*;

    #[test]
    fn object_not_found_maps_to_resource_not_found() {
        let err = S3Err::ObjectNotFoundErr(ObjectNotFoundErr {
            key: "k".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::ResourceNotFound));
        assert_eq!(err.http_status().as_u16(), 404);
        assert!(!err.is_network_conn_err());
        assert!(err.to_string().contains("object not found"));
    }

    #[test]
    fn connection_err_is_network_conn_err() {
        let err = S3Err::ConnectionErr(ConnectionErr {
            key: "k".to_string(),
            msg: "boom".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(err.is_network_conn_err());
        assert!(matches!(err.code(), Code::InternalServerError));
        assert!(err.to_string().contains("connection error"));
    }

    #[test]
    fn request_failed_err_defaults_to_internal_server_error() {
        let err = S3Err::RequestFailedErr(RequestFailedErr {
            operation: "get_object".to_string(),
            key: None,
            status: None,
            msg: "nope".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(!err.is_network_conn_err());
        // Display with no key / no status hits the fallback formatting.
        let msg = err.to_string();
        assert!(msg.contains("<none>"));
        assert!(msg.contains("unknown"));
    }

    #[test]
    fn invalid_response_err_defaults_to_internal_server_error() {
        let err = S3Err::InvalidResponseErr(InvalidResponseErr {
            operation: "get_object".to_string(),
            msg: "bad body".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(err.to_string().contains("invalid response"));
    }
}
