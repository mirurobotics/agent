//! Integration tests for the multipart upload primitives, mirroring the
//! `s3::multipart` source module: the multipart `put` path (create →
//! upload_part → complete, aborting the upload on failure) and the `list_parts`
//! primitive (happy path, pagination across pages, and 404 → `Ok(None)`).

use super::*;
use miru_agent::s3::UploadedPart;

const UPLOAD_ID: &str = "test-upload-id";

/// Multipart upload path. Passing `PutOptions { part_size: 0 }` lets a tiny
/// temp file drive the create → upload_part(s) → complete sequence without a
/// 5 GiB fixture.
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

    #[tokio::test]
    async fn large_file_uploads_in_parts() {
        let key = "big.bin";
        // Passing `PutOptions { part_size: 0 }` forces this small file onto the
        // multipart path. The 8 MiB part size dwarfs the file, so it uploads as
        // a single part — enough to exercise the full create → upload_part →
        // complete sequence without a huge fixture.
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
            .put(
                &File::new(src.path()),
                &obj(key),
                PutOptions { part_size: 0 },
            )
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
            .put(
                &File::new(src.path()),
                &obj(key),
                PutOptions { part_size: 0 },
            )
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
            .put(
                &File::new(src.path()),
                &obj(key),
                PutOptions { part_size: 0 },
            )
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
            .put(
                &File::new(src.path()),
                &obj(key),
                PutOptions { part_size: 0 },
            )
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
            .put(
                &File::new(src.path()),
                &obj(key),
                PutOptions { part_size: 0 },
            )
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

/// `list_parts` is a public multipart primitive with no in-tree caller, so it is
/// covered directly here: the happy path, pagination across two pages, and the
/// 404 → `Ok(None)` mapping used to detect an expired upload.
pub mod list_parts {
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

    #[tokio::test]
    async fn list_parts_returns_uploaded_parts() {
        let key = "big.bin";
        let req = http::Request::builder()
            .method("GET")
            .uri(uri(&format!("big.bin?uploadId={UPLOAD_ID}&x-id=ListParts")))
            .body(SdkBody::empty())
            .unwrap();
        let resp = list_parts_resp(
            &[
                (1, "\"etag-1\"", 8 * 1024 * 1024),
                (2, "\"etag-2\"", 4 * 1024 * 1024),
            ],
            None,
        );
        let (store, replay) = store_with(vec![ReplayEvent::new(req, resp)]);

        let parts = store
            .list_parts(&obj(key), UPLOAD_ID)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            parts,
            vec![
                UploadedPart {
                    number: 1,
                    etag: "\"etag-1\"".to_string(),
                },
                UploadedPart {
                    number: 2,
                    etag: "\"etag-2\"".to_string(),
                },
            ]
        );
        assert_eq!(replay.actual_requests().count(), 1);
    }

    #[tokio::test]
    async fn list_parts_follows_pagination() {
        let key = "big.bin";
        // First page is truncated with a marker; second page completes. All parts
        // accumulate and both requests fire.
        let page1_req = http::Request::builder()
            .method("GET")
            .uri(uri(&format!("big.bin?uploadId={UPLOAD_ID}&x-id=ListParts")))
            .body(SdkBody::empty())
            .unwrap();
        let page2_req = http::Request::builder()
            .method("GET")
            .uri(uri(&format!(
                "big.bin?part-number-marker=1&uploadId={UPLOAD_ID}&x-id=ListParts"
            )))
            .body(SdkBody::empty())
            .unwrap();
        let (store, replay) = store_with(vec![
            ReplayEvent::new(
                page1_req,
                list_parts_resp(&[(1, "\"etag-1\"", 8 * 1024 * 1024)], Some(1)),
            ),
            ReplayEvent::new(
                page2_req,
                list_parts_resp(&[(2, "\"etag-2\"", 4 * 1024 * 1024)], None),
            ),
        ]);

        let parts = store
            .list_parts(&obj(key), UPLOAD_ID)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            parts,
            vec![
                UploadedPart {
                    number: 1,
                    etag: "\"etag-1\"".to_string(),
                },
                UploadedPart {
                    number: 2,
                    etag: "\"etag-2\"".to_string(),
                },
            ]
        );
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(requests.len(), 2);
        assert!(requests[1]
            .uri()
            .to_string()
            .contains("part-number-marker=1"));
    }

    #[tokio::test]
    async fn list_parts_404_maps_to_none() {
        let key = "big.bin";
        let req = http::Request::builder()
            .method("GET")
            .uri(uri(&format!("big.bin?uploadId={UPLOAD_ID}&x-id=ListParts")))
            .body(SdkBody::empty())
            .unwrap();
        // A 404 (e.g. NoSuchUpload) means the upload no longer exists.
        let resp = http::Response::builder()
            .status(404)
            .header("content-type", "application/xml")
            .body(SdkBody::from(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchUpload</Code><Message>The specified upload does not exist.</Message></Error>"#,
            ))
            .unwrap();
        let (store, _replay) = store_with(vec![ReplayEvent::new(req, resp)]);

        let result = store.list_parts(&obj(key), UPLOAD_ID).await.unwrap();

        assert!(result.is_none());
    }
}
