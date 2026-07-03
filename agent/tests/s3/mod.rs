// internal crates
use miru_agent::errors::{Code, Error};
use miru_agent::s3::errors::{
    ConnectionErr, InvalidResponseErr, ObjectNotFoundErr, RequestFailedErr,
};
use miru_agent::s3::{Credentials, S3Err, S3Store};

// external crates
use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_types::body::SdkBody;

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

/// Wires an `S3Store` to a `StaticReplayClient` serving the given events.
fn store_with(events: Vec<ReplayEvent>) -> (S3Store, StaticReplayClient) {
    let replay = StaticReplayClient::new(events);
    let store = S3Store::with_http_client(replay.clone(), REGION.to_string(), BUCKET.to_string());
    (store, replay)
}

pub mod put {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        async fn put_uploads_body_bytes() {
            let key = "artifacts/hello.txt";
            let body = b"hello world".to_vec();
            let expected_req = http::Request::builder()
                .method("PUT")
                .uri(uri("artifacts/hello.txt?x-id=PutObject"))
                .body(SdkBody::from(body.clone()))
                .unwrap();
            let canned_resp = http::Response::builder()
                .status(200)
                .body(SdkBody::empty())
                .unwrap();
            let (store, replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

            store.put_object(key, body).await.unwrap();

            replay.assert_requests_match(IGNORED_HEADERS);
        }
    }
}

pub mod get {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        async fn get_round_trips_bytes() {
            let key = "blobs/data.bin";
            let payload = b"\x00\x01\x02binary-body\xff".to_vec();
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

            let got = store.get_object(key).await.unwrap();

            assert_eq!(got, payload);
            replay.assert_requests_match(IGNORED_HEADERS);
        }
    }

    pub mod not_found {
        use super::*;

        const NO_SUCH_KEY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message><Key>missing.txt</Key><RequestId>REQ123</RequestId></Error>"#;

        #[tokio::test]
        async fn get_missing_maps_to_not_found() {
            let key = "missing.txt";
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

            let err = store.get_object(key).await.unwrap_err();

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

            store.delete_object(key).await.unwrap();

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

            assert!(store.object_exists(key).await.unwrap());
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

            assert!(!store.object_exists(key).await.unwrap());
        }
    }
}

pub mod list {
    use super::*;

    const LIST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>test-bucket</Name><Prefix>blobs/</Prefix><KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated><Contents><Key>blobs/one.txt</Key><LastModified>2026-07-03T00:00:00.000Z</LastModified><ETag>"etag1"</ETag><Size>3</Size><StorageClass>STANDARD</StorageClass></Contents><Contents><Key>blobs/two.txt</Key><LastModified>2026-07-03T00:00:00.000Z</LastModified><ETag>"etag2"</ETag><Size>3</Size><StorageClass>STANDARD</StorageClass></Contents></ListBucketResult>"#;

    #[tokio::test]
    async fn list_returns_keys_in_order() {
        let canned_resp = http::Response::builder()
            .status(200)
            .header("content-type", "application/xml")
            .body(SdkBody::from(LIST_XML))
            .unwrap();
        // list_objects_v2 issues GET /<bucket>?list-type=2&prefix=blobs/;
        // the request half is unused here (we assert on the returned keys), so
        // pair it with an empty request the SDK will not match against.
        let expected_req = http::Request::builder()
            .method("GET")
            .uri(uri("?list-type=2&prefix=blobs%2F"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

        let keys = store.list_objects("blobs/").await.unwrap();

        assert_eq!(keys, vec!["blobs/one.txt", "blobs/two.txt"]);
    }

    const TRUNCATED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>test-bucket</Name><Prefix>blobs/</Prefix><KeyCount>1</KeyCount><MaxKeys>1</MaxKeys><IsTruncated>true</IsTruncated><NextContinuationToken>tok</NextContinuationToken><Contents><Key>blobs/one.txt</Key><LastModified>2026-07-03T00:00:00.000Z</LastModified><ETag>"e1"</ETag><Size>3</Size><StorageClass>STANDARD</StorageClass></Contents></ListBucketResult>"#;

    #[tokio::test]
    async fn list_truncated_returns_first_page() {
        let canned_resp = http::Response::builder()
            .status(200)
            .header("content-type", "application/xml")
            .body(SdkBody::from(TRUNCATED_XML))
            .unwrap();
        let expected_req = http::Request::builder()
            .method("GET")
            .uri(uri("?list-type=2&prefix=blobs%2F"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(expected_req, canned_resp)]);

        // A truncated response still returns the first page of keys (pagination
        // is a follow-up); it must not error.
        let keys = store.list_objects("blobs/").await.unwrap();

        assert_eq!(keys, vec!["blobs/one.txt"]);
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
        let req = http::Request::builder()
            .method("PUT")
            .uri(uri("denied.txt?x-id=PutObject"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, access_denied_resp())]);

        let err = store
            .put_object("denied.txt", b"x".to_vec())
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
        let req = http::Request::builder()
            .method("GET")
            .uri(uri("denied.txt?x-id=GetObject"))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, access_denied_resp())]);

        let err = store.get_object("denied.txt").await.unwrap_err();

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

        let err = store.delete_object("denied.txt").await.unwrap_err();

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

        let err = store.object_exists("denied.txt").await.unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn list_403_maps_to_request_failed() {
        let req = http::Request::builder()
            .method("GET")
            .uri(uri("?list-type=2&prefix="))
            .body(SdkBody::empty())
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, access_denied_resp())]);

        let err = store.list_objects("").await.unwrap_err();

        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn transport_failure_maps_to_connection_err() {
        // With no replay events, the connector fails to dispatch the request,
        // which the SDK surfaces as `SdkError::DispatchFailure` — the mapper's
        // network-connection path.
        let (store, _replay) = store_with(vec![]);

        let err = store.get_object("any.txt").await.unwrap_err();

        assert!(matches!(err, S3Err::ConnectionErr(_)));
        assert!(err.is_network_conn_err());
    }
}

/// The production constructor performs no network I/O, so it can be built and
/// dropped in a unit test to exercise that code path.
pub mod construction {
    use super::*;

    #[tokio::test]
    async fn new_builds_without_network() {
        let creds = Credentials {
            access_key_id: "AKIA_TEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: "session".to_string(),
        };
        // Constructing must not panic or touch the network.
        let _store = S3Store::new(creds, "us-west-2".to_string(), "prod-bucket".to_string());
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
