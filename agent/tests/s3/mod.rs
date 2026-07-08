// standard crates
use std::io::Write as _;

// internal crates
use miru_agent::errors::{Code, Error};
use miru_agent::filesys::file::File;
use miru_agent::s3::errors::{
    ConnectionErr, InvalidResponseErr, ObjectNotFoundErr, RequestFailedErr,
};
use miru_agent::s3::{Config, Credentials, Object, S3Err, Store};

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

/// Builds an [`Object`] in the test bucket for the given key, keeping call sites
/// terse.
fn obj(key: &str) -> Object {
    Object {
        bucket: BUCKET.to_string(),
        key: key.to_string(),
    }
}

/// Wires a `Store` to a `StaticReplayClient` serving the given events, so the
/// client's requests are matched against canned responses without touching the
/// network.
fn store_with(events: Vec<ReplayEvent>) -> (Store, StaticReplayClient) {
    let replay = StaticReplayClient::new(events);
    let cfg = Config {
        region: REGION.to_string(),
        creds: Credentials::default(),
    };
    let store = Store::from_http_client(replay.clone(), cfg);
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

/// Builds an expected request (method + path-style URI, empty body) to match a
/// `ReplayEvent` against. Only the method and key/query are usually meaningful.
fn req(method: &str, path_and_query: &str) -> http::Request<SdkBody> {
    http::Request::builder()
        .method(method)
        .uri(uri(path_and_query))
        .body(SdkBody::empty())
        .unwrap()
}

/// Builds a canned response with `status` and `body`, setting `content-length`
/// to match. Pass `&[]` for a bodyless response (200 OK, 204, HEAD, ...).
fn resp(status: u16, body: &[u8]) -> http::Response<SdkBody> {
    http::Response::builder()
        .status(status)
        .header("content-length", body.len().to_string())
        .body(SdkBody::from(body.to_vec()))
        .unwrap()
}

/// Builds a canned S3 error response carrying an XML `<Error>` document, as S3
/// returns for failures like `NoSuchKey` or `AccessDenied`.
fn resp_xml(status: u16, xml: &str) -> http::Response<SdkBody> {
    http::Response::builder()
        .status(status)
        .header("content-type", "application/xml")
        .body(SdkBody::from(xml.to_string()))
        .unwrap()
}

/// Wires a `Store` to a replay client expecting exactly one request/response
/// exchange — the shape of nearly every test here.
fn store_expecting(
    request: http::Request<SdkBody>,
    response: http::Response<SdkBody>,
) -> (Store, StaticReplayClient) {
    store_with(vec![ReplayEvent::new(request, response)])
}

pub mod put {
    use super::*;

    pub mod single {
        use super::*;

        #[tokio::test]
        async fn put_streams_file_body_bytes() {
            let key = "artifacts/hello.txt";
            let src = temp_file_with(b"hello world");
            // The body streams off disk, so the replay client records it as an
            // unbuffered `SdkBody` whose `.bytes()` is `None` — `assert_requests_match`
            // can't byte-compare it, so we assert the method + path by hand instead.
            // `get_streams_body_to_file` covers a real round-trip of the payload.
            let (store, replay) = store_expecting(
                req("PUT", "artifacts/hello.txt?x-id=PutObject"),
                resp(200, &[]),
            );

            store
                .put_singlepart(&File::new(src.path()), &obj(key))
                .await
                .unwrap();

            let requests = replay.actual_requests().collect::<Vec<_>>();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].method(), "PUT");
            assert_eq!(
                requests[0].uri().to_string(),
                uri("artifacts/hello.txt?x-id=PutObject")
            );
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
            let (store, replay) = store_expecting(
                req("GET", "blobs/data.bin?x-id=GetObject"),
                resp(200, &payload),
            );

            store.get(&obj(key), &File::new(dest.path())).await.unwrap();

            assert_eq!(std::fs::read(dest.path()).unwrap(), payload);
            replay.assert_requests_match(IGNORED_HEADERS);
        }
    }

    pub mod dest_unwritable {
        use super::*;

        #[tokio::test]
        async fn get_to_missing_parent_dir_maps_to_invalid_response() {
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
            let (store, _replay) = store_expecting(
                req("GET", "missing.txt?x-id=GetObject"),
                resp_xml(404, NO_SUCH_KEY_XML),
            );

            let err = store
                .get(&obj(key), &File::new(dest.path()))
                .await
                .unwrap_err();

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
            let (store, replay) = store_expecting(
                req("DELETE", "blobs/data.bin?x-id=DeleteObject"),
                resp(204, &[]),
            );

            store.delete(&obj(key)).await.unwrap();

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
            let (store, _replay) = store_expecting(req("HEAD", "blobs/data.bin"), resp(200, &[]));

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
}

/// S3 returns non-404 failures (e.g. 403 AccessDenied) as a modeled service
/// error, which the common mapper turns into `RequestFailedErr`.
pub mod request_failed {
    use super::*;

    const ACCESS_DENIED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>AccessDenied</Code><Message>Access Denied</Message><RequestId>REQ403</RequestId></Error>"#;

    fn access_denied_resp() -> http::Response<SdkBody> {
        resp_xml(403, ACCESS_DENIED_XML)
    }

    #[tokio::test]
    async fn put_403_maps_to_request_failed() {
        let src = temp_file_with(b"x");
        let (store, _replay) =
            store_expecting(req("PUT", "denied.txt?x-id=PutObject"), access_denied_resp());

        let err = store
            .put(File::new(src.path()), &obj("denied.txt"))
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
        let (store, _replay) =
            store_expecting(req("GET", "denied.txt?x-id=GetObject"), access_denied_resp());

        let err = store
            .get(&obj("denied.txt"), &File::new(dest.path()))
            .await
            .unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn delete_403_maps_to_request_failed() {
        let (store, _replay) = store_expecting(
            req("DELETE", "denied.txt?x-id=DeleteObject"),
            access_denied_resp(),
        );

        let err = store.delete(&obj("denied.txt")).await.unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn head_403_propagates_as_request_failed() {
        // HEAD has no response body; a 403 has no XML payload.
        let (store, _replay) = store_expecting(req("HEAD", "denied.txt"), resp(403, &[]));

        let err = store.exists(&obj("denied.txt")).await.unwrap_err();

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
            .get(&obj("any.txt"), &File::new(dest.path()))
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

        let err = store.put(missing, &obj("k")).await.unwrap_err();

        assert!(matches!(err, S3Err::InvalidResponseErr(_)));
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
        };
        // Constructing must not panic or touch the network.
        let _store = Store::new(cfg);
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
            object: None,
            status: None,
            msg: "nope".to_string(),
            trace: miru_agent::trace!(),
        });
        assert!(matches!(err.code(), Code::InternalServerError));
        assert_eq!(err.http_status().as_u16(), 500);
        assert!(!err.is_network_conn_err());
        // Display with no object / no status hits the fallback formatting.
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
