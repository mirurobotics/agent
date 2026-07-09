//! Offline (replay-client) tests for the multipart upload surface, mirroring the
//! `s3::multipart` source module: the stateless multipart `put` path (create →
//! upload_part → complete, aborting the upload on failure) and the size-based
//! routing in [`Store::put`] that picks the single-part or multipart path by
//! file size.

use super::*;
use miru_agent::s3::Source;

const UPLOAD_ID: &str = "test-upload-id";

/// Builds a `Source` from a temp file, reading its length off disk with the
/// crate's own `files::size`.
async fn source_of(tf: &files::TempFile) -> Source {
    let file = tf.to_file();
    let size = files::size(&file).await.unwrap();
    Source { file, size }
}

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

fn create_req() -> http::Request<SdkBody> {
    http::Request::builder()
        .method("POST")
        .uri(uri("big.bin?uploads&x-id=CreateMultipartUpload"))
        .body(SdkBody::empty())
        .unwrap()
}

fn upload_part_req(number: i32) -> http::Request<SdkBody> {
    http::Request::builder()
        .method("PUT")
        .uri(uri(&format!(
            "big.bin?x-id=UploadPart&partNumber={number}&uploadId={UPLOAD_ID}"
        )))
        .body(SdkBody::empty())
        .unwrap()
}

fn complete_req() -> http::Request<SdkBody> {
    http::Request::builder()
        .method("POST")
        .uri(uri(&format!(
            "big.bin?x-id=CompleteMultipartUpload&uploadId={UPLOAD_ID}"
        )))
        .body(SdkBody::empty())
        .unwrap()
}

fn abort_req() -> http::Request<SdkBody> {
    http::Request::builder()
        .method("DELETE")
        .uri(uri(&format!(
            "big.bin?x-id=AbortMultipartUpload&uploadId={UPLOAD_ID}"
        )))
        .body(SdkBody::empty())
        .unwrap()
}

fn abort_resp() -> http::Response<SdkBody> {
    http::Response::builder()
        .status(204)
        .body(SdkBody::empty())
        .unwrap()
}

/// Stateless multipart `put_multipart`: a tiny temp file rides the multipart path
/// (the 8 MiB part size dwarfs it, so it uploads as a single part), driving the
/// create → upload_part → complete sequence and the abort-on-failure paths.
pub mod put {
    use super::*;

    #[tokio::test]
    async fn small_file_uploads_as_single_part() {
        let key = "big.bin";
        // The 8 MiB part size dwarfs this file, so it uploads as a single part —
        // enough to exercise the full create → upload_part → complete sequence
        // without a huge fixture.
        let src = temp_file_with(b"multipart-body").await;

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), upload_part_resp("\"etag-part-1\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap();

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
        let src = temp_file_with(b"multipart-body").await;

        // Well-formed XML but with no <UploadId> element.
        let no_id_resp = http::Response::builder()
            .status(200)
            .header("content-type", "application/xml")
            .body(SdkBody::from(format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>big.bin</Key></InitiateMultipartUploadResult>"#
            )))
            .unwrap();

        let (store, _replay) = store_with(vec![ReplayEvent::new(create_req(), no_id_resp)]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::InvalidResponseErr(_)));
    }

    #[tokio::test]
    async fn part_without_etag_maps_to_invalid_response() {
        let key = "big.bin";
        let src = temp_file_with(b"multipart-body").await;

        // The part upload succeeds (200) but the response omits the ETag header,
        // so `upload_part` yields `InvalidResponseErr`, which propagates through
        // `exec_multipart_upload` and triggers the best-effort abort.
        let part_no_etag = http::Response::builder()
            .status(200)
            .body(SdkBody::empty())
            .unwrap();

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), part_no_etag),
            ReplayEvent::new(abort_req(), abort_resp()),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::InvalidResponseErr(_)));

        // The missing ETag triggered a best-effort abort as the final request.
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].method(), "DELETE");
        assert!(requests[2]
            .uri()
            .to_string()
            .contains("x-id=AbortMultipartUpload"));
    }

    #[tokio::test]
    async fn create_failure_maps_to_request_failed() {
        let key = "big.bin";
        let src = temp_file_with(b"multipart-body").await;

        // CreateMultipartUpload itself fails with a 403 — no upload exists to
        // abort, so the error surfaces directly.
        let (store, _replay) =
            store_with(vec![ReplayEvent::new(create_req(), access_denied_resp())]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::RequestFailedErr(_)));
    }

    #[tokio::test]
    async fn part_failure_triggers_abort() {
        let key = "big.bin";
        let src = temp_file_with(b"multipart-body").await;

        // The upload_part call fails with a 403, which must trigger an abort.
        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), access_denied_resp()),
            ReplayEvent::new(abort_req(), abort_resp()),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap_err();
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

    #[tokio::test]
    async fn complete_failure_triggers_abort() {
        let key = "big.bin";
        let src = temp_file_with(b"multipart-body").await;

        // The part uploads fine, but CompleteMultipartUpload fails, which must
        // trigger an abort of the in-progress upload.
        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), upload_part_resp("\"etag-part-1\"")),
            ReplayEvent::new(complete_req(), access_denied_resp()),
            ReplayEvent::new(abort_req(), abort_resp()),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap_err();
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
    async fn failing_abort_does_not_mask_original_error() {
        let key = "big.bin";
        let src = temp_file_with(b"multipart-body").await;

        // The part upload fails, triggering a best-effort abort — but the abort
        // itself also fails (403). The abort is best-effort, so its failure is
        // swallowed and the ORIGINAL upload_part error still surfaces.
        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), access_denied_resp()),
            ReplayEvent::new(abort_req(), access_denied_resp()),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key))
            .await
            .unwrap_err();
        // The surfaced error is the original part failure, not the abort failure.
        assert!(matches!(err, S3Err::RequestFailedErr(_)));

        // The abort was still attempted (best-effort) as the final request.
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].method(), "DELETE");
        assert!(requests[2]
            .uri()
            .to_string()
            .contains("x-id=AbortMultipartUpload"));
    }
}

/// Size-based routing in [`Store::put`]: small files take the single `PutObject`
/// path, larger-than-`PART_SIZE` files take the multipart path.
pub mod routing {
    use super::*;

    #[tokio::test]
    async fn small_file_routes_to_single_put() {
        // A body well under PART_SIZE must take the single-part branch: exactly
        // one PutObject, no multipart calls.
        let src = temp_file_with(b"tiny").await;
        let (store, replay) =
            store_expecting(req("PUT", "small.bin?x-id=PutObject"), resp(200, &[]));

        store.put(src.to_file(), &obj("small.bin")).await.unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), "PUT");
        let uri = requests[0].uri().to_string();
        assert!(uri.contains("x-id=PutObject"));
        assert!(!uri.contains("uploads"));
        assert!(!uri.contains("UploadPart"));
    }

    #[tokio::test]
    async fn large_file_routes_to_multipart() {
        // The crate constant is private; re-declare it locally to size a fixture
        // just past the routing threshold. 8 MiB + 1 KiB => 2 parts (8 MiB, 1 KiB).
        const PART_SIZE: u64 = 8 * 1024 * 1024;
        let big = vec![0u8; (PART_SIZE + 1024) as usize];
        let src = temp_file_with(&big).await;

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), upload_part_resp("\"etag-part-1\"")),
            ReplayEvent::new(upload_part_req(2), upload_part_resp("\"etag-part-2\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store.put(src.to_file(), &obj("big.bin")).await.unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        // The multipart branch was taken: create first, then a part upload.
        assert_eq!(requests[0].method(), "POST");
        assert!(requests[0].uri().to_string().contains("uploads"));
        assert!(requests
            .iter()
            .any(|r| r.uri().to_string().contains("x-id=UploadPart")
                && r.uri().to_string().contains("partNumber=1")));
    }
}
