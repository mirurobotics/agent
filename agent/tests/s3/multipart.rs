//! Offline (replay-client) tests for the multipart upload surface, mirroring the
//! `s3::multipart` source module: the stateless multipart `put` path (create →
//! upload_part → complete, aborting the upload on failure) and the resumable
//! `resume_multipart_upload` path (gap-filling only the missing parts, or
//! failing with `NoSuchUploadErr` when the upload has expired).

use super::*;
use miru_agent::s3::multipart::Source;

const UPLOAD_ID: &str = "test-upload-id";

/// Writes a fresh temp file whose length forces `part_size_for` to yield exactly
/// two parts: one full `PART_SIZE` (8 MiB) part plus a small tail part. Returns
/// the handle (kept alive so the file survives until the test drops it) and the
/// total byte length.
fn two_part_file() -> (NamedTempFile, u64) {
    const PART_SIZE: u64 = 8 * 1024 * 1024;
    let len = PART_SIZE + 1024; // 8 MiB + 1 KiB => 2 parts (8 MiB, 1 KiB).
    let bytes = vec![7u8; len as usize];
    (temp_file_with(&bytes), len)
}

/// Stateless multipart `put_multipart`: a tiny temp file rides the multipart path
/// (the 8 MiB part size dwarfs it, so it uploads as a single part), driving the
/// create → upload_part → complete sequence and the abort-on-failure paths.
pub mod put {
    use super::*;

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

    /// Builds a `Source` for a temp file, reading its length off disk.
    fn source_of(f: &NamedTempFile) -> Source {
        let size = std::fs::metadata(f.path()).unwrap().len();
        Source {
            file: File::new(f.path()),
            size,
        }
    }

    #[tokio::test]
    async fn small_file_uploads_as_single_part() {
        let key = "big.bin";
        // The 8 MiB part size dwarfs this file, so it uploads as a single part —
        // enough to exercise the full create → upload_part → complete sequence
        // without a huge fixture.
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

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req, create_resp()),
            ReplayEvent::new(part_req, upload_part_resp("\"etag-part-1\"")),
            ReplayEvent::new(complete_req, complete_resp()),
        ]);

        store
            .put_multipart(&source_of(&src), &obj(key))
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

        let (store, _replay) = store_with(vec![ReplayEvent::new(create_req, no_id_resp)]);

        let err = store
            .put_multipart(&source_of(&src), &obj(key))
            .await
            .unwrap_err();
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

        let (store, _replay) = store_with(vec![ReplayEvent::new(create_req, create_fail)]);

        let err = store
            .put_multipart(&source_of(&src), &obj(key))
            .await
            .unwrap_err();
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

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req, create_resp()),
            ReplayEvent::new(part_req, upload_part_resp("\"etag-part-1\"")),
            ReplayEvent::new(complete_req, complete_fail),
            ReplayEvent::new(abort_req, abort_resp),
        ]);

        let err = store
            .put_multipart(&source_of(&src), &obj(key))
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

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req, create_resp()),
            ReplayEvent::new(part_req, part_fail_resp),
            ReplayEvent::new(abort_req, abort_resp),
        ]);

        let err = store
            .put_multipart(&source_of(&src), &obj(key))
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
}

/// `resume_multipart_upload` gap-fills an existing upload: it lists the landed
/// parts, uploads only the missing ranges, and completes — never aborting, so a
/// resume is safe to re-run. These tests also provide the `list_parts` /
/// `list_parts_page` coverage (happy path, pagination, and the 404 → expired
/// mapping) now that those primitives are private.
pub mod resume {
    use super::*;

    /// Canned `ListPartsResult` XML for `parts`, optionally truncated with a
    /// `NextPartNumberMarker`, modeled on the real S3 response shape.
    fn list_parts_xml(parts: &[(i32, &str, u64)], next_marker: Option<i32>) -> String {
        let mut body = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ListPartsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>test-bucket</Bucket><Key>big.bin</Key><UploadId>test-upload-id</UploadId>"#,
        );
        match next_marker {
            Some(m) => body.push_str(&format!(
                "<IsTruncated>true</IsTruncated><NextPartNumberMarker>{m}</NextPartNumberMarker>"
            )),
            None => body.push_str("<IsTruncated>false</IsTruncated>"),
        }
        for (number, etag, size) in parts {
            body.push_str(&format!(
                "<Part><PartNumber>{number}</PartNumber><ETag>{etag}</ETag><Size>{size}</Size></Part>"
            ));
        }
        body.push_str("</ListPartsResult>");
        body
    }

    fn list_parts_resp(
        parts: &[(i32, &str, u64)],
        next_marker: Option<i32>,
    ) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(200)
            .header("content-type", "application/xml")
            .body(SdkBody::from(list_parts_xml(parts, next_marker)))
            .unwrap()
    }

    fn list_parts_req() -> http::Request<SdkBody> {
        http::Request::builder()
            .method("GET")
            .uri(uri(&format!("big.bin?uploadId={UPLOAD_ID}&x-id=ListParts")))
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

    fn upload_part_resp(etag: &str) -> http::Response<SdkBody> {
        http::Response::builder()
            .status(200)
            .header("ETag", etag)
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

    /// Only part 2 is missing: list reports part 1 landed, so resume uploads part
    /// 2 alone and completes with both etags.
    #[tokio::test]
    async fn uploads_only_missing_parts() {
        let (src, _len) = two_part_file();

        let (store, replay) = store_with(vec![
            ReplayEvent::new(
                list_parts_req(),
                list_parts_resp(&[(1, "\"landed-1\"", 8 * 1024 * 1024)], None),
            ),
            ReplayEvent::new(upload_part_req(2), upload_part_resp("\"fresh-2\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .resume_multipart_upload(&File::new(src.path()), &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        // list_parts, then upload of part 2 only, then complete.
        assert_eq!(requests.len(), 3);
        assert!(requests[0].uri().to_string().contains("x-id=ListParts"));
        assert_eq!(requests[1].method(), "PUT");
        assert!(requests[1].uri().to_string().contains("partNumber=2"));
        // Complete is a POST to `?uploadId=...` with no part number.
        assert_eq!(requests[2].method(), "POST");
        assert!(requests[2]
            .uri()
            .to_string()
            .contains(&format!("uploadId={UPLOAD_ID}")));
        assert!(!requests[2].uri().to_string().contains("partNumber"));
    }

    /// No parts landed: list is empty, so resume uploads both parts, then
    /// completes.
    #[tokio::test]
    async fn uploads_all_parts_when_none_landed() {
        let (src, _len) = two_part_file();

        let (store, replay) = store_with(vec![
            ReplayEvent::new(list_parts_req(), list_parts_resp(&[], None)),
            ReplayEvent::new(upload_part_req(1), upload_part_resp("\"fresh-1\"")),
            ReplayEvent::new(upload_part_req(2), upload_part_resp("\"fresh-2\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .resume_multipart_upload(&File::new(src.path()), &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 4);
        assert!(requests[1].uri().to_string().contains("partNumber=1"));
        assert!(requests[2].uri().to_string().contains("partNumber=2"));
        // Complete is a POST to `?uploadId=...` with no part number.
        assert_eq!(requests[3].method(), "POST");
        assert!(requests[3]
            .uri()
            .to_string()
            .contains(&format!("uploadId={UPLOAD_ID}")));
        assert!(!requests[3].uri().to_string().contains("partNumber"));
    }

    /// Both parts already landed: resume uploads nothing and completes directly
    /// from the listed etags.
    #[tokio::test]
    async fn uploads_nothing_when_all_landed() {
        let (src, _len) = two_part_file();

        let (store, replay) = store_with(vec![
            ReplayEvent::new(
                list_parts_req(),
                list_parts_resp(
                    &[
                        (1, "\"landed-1\"", 8 * 1024 * 1024),
                        (2, "\"landed-2\"", 1024),
                    ],
                    None,
                ),
            ),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .resume_multipart_upload(&File::new(src.path()), &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        // Only list_parts and complete — no upload_part fired.
        assert_eq!(requests.len(), 2);
        assert!(requests[0].uri().to_string().contains("x-id=ListParts"));
        // Complete is a POST to `?uploadId=...` with no part number.
        assert_eq!(requests[1].method(), "POST");
        assert!(requests[1]
            .uri()
            .to_string()
            .contains(&format!("uploadId={UPLOAD_ID}")));
        assert!(!requests
            .iter()
            .any(|r| r.uri().to_string().contains("x-id=UploadPart")));
    }

    /// An expired / aborted upload: list_parts 404s (`NoSuchUpload`), so resume
    /// returns `NoSuchUploadErr` and issues neither an upload_part nor a complete.
    #[tokio::test]
    async fn expired_upload_maps_to_no_such_upload() {
        let (src, _len) = two_part_file();

        let not_found = http::Response::builder()
            .status(404)
            .header("content-type", "application/xml")
            .body(SdkBody::from(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchUpload</Code><Message>The specified upload does not exist.</Message></Error>"#,
            ))
            .unwrap();

        let (store, replay) = store_with(vec![ReplayEvent::new(list_parts_req(), not_found)]);

        let err = store
            .resume_multipart_upload(&File::new(src.path()), &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap_err();

        assert!(matches!(err, S3Err::NoSuchUploadErr(_)));
        assert!(matches!(err.code(), Code::ResourceNotFound));
        assert_eq!(err.http_status().as_u16(), 404);

        // Only the list_parts call fired: no upload_part, no complete.
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].uri().to_string().contains("x-id=ListParts"));
    }

    /// The landed set is discovered across a truncated, two-page `list_parts`
    /// listing. Both parts are already present, so resume completes without any
    /// upload_part.
    #[tokio::test]
    async fn list_parts_pagination_feeds_landed_set() {
        let (src, _len) = two_part_file();

        let page2_req = http::Request::builder()
            .method("GET")
            .uri(uri(&format!(
                "big.bin?part-number-marker=1&uploadId={UPLOAD_ID}&x-id=ListParts"
            )))
            .body(SdkBody::empty())
            .unwrap();

        let (store, replay) = store_with(vec![
            ReplayEvent::new(
                list_parts_req(),
                list_parts_resp(&[(1, "\"landed-1\"", 8 * 1024 * 1024)], Some(1)),
            ),
            ReplayEvent::new(
                page2_req,
                list_parts_resp(&[(2, "\"landed-2\"", 1024)], None),
            ),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .resume_multipart_upload(&File::new(src.path()), &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        let requests = replay.actual_requests().collect::<Vec<_>>();
        // Two list pages then complete; both parts came from the listing.
        assert_eq!(requests.len(), 3);
        assert!(requests[1]
            .uri()
            .to_string()
            .contains("part-number-marker=1"));
        // Complete is a POST to `?uploadId=...` with no part number.
        assert_eq!(requests[2].method(), "POST");
        assert!(requests[2]
            .uri()
            .to_string()
            .contains(&format!("uploadId={UPLOAD_ID}")));
        assert!(!requests
            .iter()
            .any(|r| r.uri().to_string().contains("x-id=UploadPart")));
    }
}
