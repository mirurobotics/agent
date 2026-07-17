//! Offline (replay-client) tests for the multipart upload surface, mirroring the
//! `s3::multipart` source module: the stateless multipart `put` path (create →
//! upload_part → complete, aborting the upload on failure). Size-based routing
//! in [`Store::put`] lives under [`super::put::routing`].

use super::*;
use miru_agent::s3::Source;

pub(crate) const UPLOAD_ID: &str = "test-upload-id";

/// Builds a `Source` from a temp file, reading its length off disk with the
/// crate's own `files::size`.
async fn source_of(tf: &files::TempFile) -> Source {
    let file = tf.to_file();
    let size = files::size(&file).await.unwrap();
    Source { file, size }
}

pub(crate) fn create_resp() -> http::Response<SdkBody> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><UploadId>{UPLOAD_ID}</UploadId></InitiateMultipartUploadResult>"#
    );
    resp_xml(200, &xml)
}

pub(crate) fn upload_part_resp(etag: &str) -> http::Response<SdkBody> {
    http::Response::builder()
        .status(200)
        .header("ETag", etag)
        .body(SdkBody::empty())
        .unwrap()
}

pub(crate) fn complete_resp() -> http::Response<SdkBody> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Location>https://s3.amazonaws.com/{BUCKET}/big.bin</Location><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><ETag>"final-etag"</ETag></CompleteMultipartUploadResult>"#
    );
    resp_xml(200, &xml)
}

pub(crate) fn create_req() -> http::Request<SdkBody> {
    http::Request::builder()
        .method("POST")
        .uri(uri("big.bin?uploads"))
        .body(SdkBody::empty())
        .unwrap()
}

pub(crate) fn upload_part_req(number: i32) -> http::Request<SdkBody> {
    http::Request::builder()
        .method("PUT")
        .uri(uri(&format!(
            "big.bin?x-id=UploadPart&partNumber={number}&uploadId={UPLOAD_ID}"
        )))
        .body(SdkBody::empty())
        .unwrap()
}

pub(crate) fn complete_req() -> http::Request<SdkBody> {
    http::Request::builder()
        .method("POST")
        .uri(uri(&format!("big.bin?uploadId={UPLOAD_ID}")))
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

// Expected wire shapes for the multipart sequence. These match what the SDK
// actually emits (create/complete omit the `x-id=...` query param that the
// ReplayEvent fixtures include for matching).
pub(crate) fn create_shape() -> (String, String) {
    shape("POST", "big.bin?uploads")
}

pub(crate) fn upload_part_shape(number: i32) -> (String, String) {
    shape(
        "PUT",
        &format!("big.bin?x-id=UploadPart&partNumber={number}&uploadId={UPLOAD_ID}"),
    )
}

pub(crate) fn complete_shape() -> (String, String) {
    shape("POST", &format!("big.bin?uploadId={UPLOAD_ID}"))
}

fn abort_shape() -> (String, String) {
    shape(
        "DELETE",
        &format!("big.bin?x-id=AbortMultipartUpload&uploadId={UPLOAD_ID}"),
    )
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
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
            .await
            .unwrap();

        // create → upload_part → complete (bodies ignored via [`shape`]).
        assert_eq!(
            actual_shapes(&replay),
            vec![create_shape(), upload_part_shape(1), complete_shape()]
        );
    }

    #[tokio::test]
    async fn create_stamps_metadata_headers() {
        let key = "big.bin";
        let src = temp_file_with(b"multipart-body").await;
        let metadata = HashMap::from([("device_id".to_string(), "dvc_1".to_string())]);

        let (store, replay) = store_with(vec![
            ReplayEvent::new(create_req(), create_resp()),
            ReplayEvent::new(upload_part_req(1), upload_part_resp("\"etag-part-1\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .put_multipart(&source_of(&src).await, &obj(key), &metadata)
            .await
            .unwrap();

        // Metadata is fixed at initiation: only the CreateMultipartUpload
        // request carries the header.
        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert_eq!(
            requests[0].headers().get("x-amz-meta-device_id"),
            Some("dvc_1")
        );
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
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
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
            ReplayEvent::new(abort_req(), resp(204, &[])),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::InvalidResponseErr(_)));

        // Missing ETag → best-effort abort as the final request.
        assert_eq!(
            actual_shapes(&replay),
            vec![create_shape(), upload_part_shape(1), abort_shape()]
        );
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
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
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
            ReplayEvent::new(abort_req(), resp(204, &[])),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::RequestFailedErr(_)));

        assert_eq!(
            actual_shapes(&replay),
            vec![create_shape(), upload_part_shape(1), abort_shape()]
        );
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
            ReplayEvent::new(abort_req(), resp(204, &[])),
        ]);

        let err = store
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::RequestFailedErr(_)));

        assert_eq!(
            actual_shapes(&replay),
            vec![
                create_shape(),
                upload_part_shape(1),
                complete_shape(),
                abort_shape(),
            ]
        );
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
            .put_multipart(&source_of(&src).await, &obj(key), &HashMap::new())
            .await
            .unwrap_err();
        // The surfaced error is the original part failure, not the abort failure.
        assert!(matches!(err, S3Err::RequestFailedErr(_)));

        // Abort was still attempted (best-effort) as the final request.
        assert_eq!(
            actual_shapes(&replay),
            vec![create_shape(), upload_part_shape(1), abort_shape()]
        );
    }

    pub mod source_missing {
        use super::*;

        #[tokio::test]
        async fn put_multipart_missing_source_maps_to_local_io_err() {
            // `put_multipart` is handed a `Source` whose path does not exist.
            // CreateMultipartUpload still succeeds (it does not touch the file);
            // the failure surfaces when `upload_part` opens the path for
            // streaming → `LocalIoErr`, then a best-effort abort.
            let missing = Source {
                file: File::new("/nonexistent/definitely/not/here.bin"),
                // Claimed size > 0 so the part loop runs at least once.
                size: 14,
            };
            let (store, replay) = store_with(vec![
                ReplayEvent::new(create_req(), create_resp()),
                ReplayEvent::new(abort_req(), resp(204, &[])),
            ]);

            let err = store
                .put_multipart(&missing, &obj("big.bin"), &HashMap::new())
                .await
                .unwrap_err();

            assert!(matches!(err, S3Err::LocalIoErr(_)));
            // create succeeded; upload_part never left the client (open failed
            // locally), so the only follow-up on the wire is the abort.
            assert_eq!(actual_shapes(&replay), vec![create_shape(), abort_shape()]);
        }

        #[tokio::test]
        async fn put_multipart_deleted_source_maps_to_local_io_err() {
            // Source was valid when sized, then deleted before the part loop —
            // same `LocalIoErr` + abort path as a never-existing file.
            let src = temp_file_with(b"multipart-body").await;
            let source = source_of(&src).await;
            files::delete(src.file()).await.unwrap();

            let (store, replay) = store_with(vec![
                ReplayEvent::new(create_req(), create_resp()),
                ReplayEvent::new(abort_req(), resp(204, &[])),
            ]);

            let err = store
                .put_multipart(&source, &obj("big.bin"), &HashMap::new())
                .await
                .unwrap_err();

            assert!(matches!(err, S3Err::LocalIoErr(_)));
            assert_eq!(actual_shapes(&replay), vec![create_shape(), abort_shape()]);
        }

        #[tokio::test]
        async fn put_multipart_shrunk_source_maps_to_local_io_err() {
            // TOCTOU: the file was sized when the `Source` was built, then
            // truncated on disk to fewer bytes than the recorded `size`. Reading
            // the part range hits `read_exact` -> `UnexpectedEof`, which maps to a
            // terminal `LocalIoErr`.
            let src = temp_file_with(b"multipart-body").await;
            let source = source_of(&src).await;
            assert!(source.size > 4);

            // Truncate the on-disk file below the recorded size so the part read
            // runs short. The `TempFile` guard keeps the path alive.
            let path = src.file().path();
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap()
                .set_len(4)
                .await
                .unwrap();

            let (store, replay) = store_with(vec![
                ReplayEvent::new(create_req(), create_resp()),
                ReplayEvent::new(abort_req(), resp(204, &[])),
            ]);

            let err = store
                .put_multipart(&source, &obj("big.bin"), &HashMap::new())
                .await
                .unwrap_err();

            assert!(matches!(err, S3Err::LocalIoErr(_)));
            // create succeeded; the short read fails locally before any
            // upload_part request leaves the client, so abort is the only
            // follow-up on the wire.
            assert_eq!(actual_shapes(&replay), vec![create_shape(), abort_shape()]);
        }
    }
}

/// Resumable multipart upload: given an existing `upload_id`, list the landed
/// parts, upload only the missing ones, and complete — never aborting.
pub mod resume {
    use super::*;

    const NO_SUCH_UPLOAD_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchUpload</Code><Message>The specified upload does not exist.</Message></Error>"#;

    /// Canned `ListPartsResult` XML for `parts` (`(number, etag, size)`), optionally
    /// truncated with a `NextPartNumberMarker`.
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
        resp_xml(200, &list_parts_xml(parts, next_marker))
    }

    fn list_parts_req() -> http::Request<SdkBody> {
        req(
            "GET",
            &format!("big.bin?x-id=ListParts&uploadId={UPLOAD_ID}"),
        )
    }

    fn list_parts_shape() -> (String, String) {
        shape(
            "GET",
            &format!("big.bin?x-id=ListParts&uploadId={UPLOAD_ID}"),
        )
    }

    /// The `part-number-marker` follow-up page the SDK emits for a truncated
    /// listing (query order mirrors the real SDK output).
    fn list_parts_page2_query() -> String {
        format!("big.bin?x-id=ListParts&part-number-marker=1&uploadId={UPLOAD_ID}")
    }

    fn list_parts_page2_shape() -> (String, String) {
        shape("GET", &list_parts_page2_query())
    }

    /// A two-part source file (8 MiB + 1 KiB ⇒ parts `(1, 0, 8 MiB)` and
    /// `(2, 8 MiB, 1 KiB)`).
    async fn two_part_file() -> files::TempFile {
        const PART_SIZE: u64 = 8 * 1024 * 1024;
        let bytes = vec![7u8; (PART_SIZE + 1024) as usize];
        temp_file_with(&bytes).await
    }

    /// Asserts the last recorded request's CompleteMultipartUpload manifest lists
    /// each `(part_number, etag)` in the given order.
    fn assert_complete_manifest(replay: &StaticReplayClient, parts: &[(i32, &str)]) {
        let requests = replay.actual_requests().collect::<Vec<_>>();
        let body = requests
            .last()
            .unwrap()
            .body()
            .bytes()
            .expect("in-memory body");
        let manifest = std::str::from_utf8(body).unwrap();
        let mut prev = 0;
        for (number, etag) in parts {
            let at = manifest
                .find(&format!("<PartNumber>{number}</PartNumber>"))
                .unwrap_or_else(|| panic!("part {number} not listed"));
            assert!(at >= prev, "part {number} out of order");
            prev = at;
            assert!(manifest.contains(etag), "part {number} missing etag {etag}");
        }
    }

    #[tokio::test]
    async fn resume_skips_landed_parts() {
        let tf = two_part_file().await;
        let src = source_of(&tf).await;

        let (store, replay) = store_with(vec![
            ReplayEvent::new(
                list_parts_req(),
                list_parts_resp(&[(1, "\"landed-1\"", 8 * 1024 * 1024)], None),
            ),
            ReplayEvent::new(upload_part_req(2), upload_part_resp("\"fresh-2\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        // list_parts → upload part 2 only → complete. Part 1 is never re-uploaded.
        assert_eq!(
            actual_shapes(&replay),
            vec![list_parts_shape(), upload_part_shape(2), complete_shape()]
        );

        // Both parts listed in order: part 1 reuses its landed etag, part 2 the fresh one.
        assert_complete_manifest(&replay, &[(1, "landed-1"), (2, "fresh-2")]);
    }

    #[tokio::test]
    async fn resume_expired_upload_maps_to_no_such_upload() {
        let tf = two_part_file().await;
        let src = source_of(&tf).await;

        let (store, replay) = store_with(vec![ReplayEvent::new(
            list_parts_req(),
            resp_xml(404, NO_SUCH_UPLOAD_XML),
        )]);

        let err = store
            .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap_err();

        assert!(matches!(err, S3Err::NoSuchUploadErr(_)));
        assert!(matches!(err.code(), Code::ResourceNotFound));
        assert_eq!(err.http_status().as_u16(), 404);

        // Only the list_parts call fired — no upload, no complete (and no abort).
        assert_eq!(actual_shapes(&replay), vec![list_parts_shape()]);
    }

    #[tokio::test]
    async fn resume_merges_paginated_list_parts() {
        let tf = two_part_file().await;
        let src = source_of(&tf).await;

        let page2_req = req("GET", &list_parts_page2_query());

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
            .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        // Two list pages (the second following the marker), then complete —
        // every part landed via the listing, so nothing is re-uploaded.
        assert_eq!(
            actual_shapes(&replay),
            vec![
                list_parts_shape(),
                list_parts_page2_shape(),
                complete_shape()
            ]
        );

        let requests = replay.actual_requests().collect::<Vec<_>>();
        assert!(requests[1]
            .uri()
            .to_string()
            .contains("part-number-marker=1"));
    }

    #[tokio::test]
    async fn resume_part_failure_does_not_abort() {
        // Part 1 landed; uploading part 2 fails (403). A resume must NOT abort —
        // no abort (or complete) event is queued, so an abort would panic the
        // replay client on an unexpected request.
        let tf = two_part_file().await;
        let src = source_of(&tf).await;

        let (store, replay) = store_with(vec![
            ReplayEvent::new(
                list_parts_req(),
                list_parts_resp(&[(1, "\"landed-1\"", 8 * 1024 * 1024)], None),
            ),
            ReplayEvent::new(upload_part_req(2), access_denied_resp()),
        ]);

        let err = store
            .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap_err();
        assert!(matches!(err, S3Err::RequestFailedErr(_)));

        // list_parts → upload part 2 (which failed). No abort, no complete.
        assert_eq!(
            actual_shapes(&replay),
            vec![list_parts_shape(), upload_part_shape(2)]
        );
    }

    #[tokio::test]
    async fn resume_zero_landed_uploads_all() {
        // A valid upload with zero landed parts (empty listing) is NOT a missing
        // upload: every part is uploaded, then complete.
        let tf = two_part_file().await;
        let src = source_of(&tf).await;

        let (store, replay) = store_with(vec![
            ReplayEvent::new(list_parts_req(), list_parts_resp(&[], None)),
            ReplayEvent::new(upload_part_req(1), upload_part_resp("\"fresh-1\"")),
            ReplayEvent::new(upload_part_req(2), upload_part_resp("\"fresh-2\"")),
            ReplayEvent::new(complete_req(), complete_resp()),
        ]);

        store
            .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        assert_eq!(
            actual_shapes(&replay),
            vec![
                list_parts_shape(),
                upload_part_shape(1),
                upload_part_shape(2),
                complete_shape(),
            ]
        );
    }

    #[tokio::test]
    async fn resume_all_landed_uploads_nothing() {
        // Both parts already landed (single page): nothing is re-uploaded, and
        // the complete manifest carries both landed etags in ascending order.
        let tf = two_part_file().await;
        let src = source_of(&tf).await;

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
            .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
            .await
            .unwrap();

        // list_parts → complete. No upload_part: every part was already landed.
        assert_eq!(
            actual_shapes(&replay),
            vec![list_parts_shape(), complete_shape()]
        );

        // Both landed parts listed in ascending order.
        assert_complete_manifest(&replay, &[(1, "landed-1"), (2, "landed-2")]);
    }
}
