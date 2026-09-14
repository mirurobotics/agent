// standard crates
use std::io::SeekFrom;

// internal crates
use crate::filesys::{file::File, path::PathExt};
use crate::s3::{errors, errors::NoSuchUploadErr, Object, S3Err, Store, PART_SIZE};

// external crates
use aws_sdk_s3::operation::list_parts::ListPartsOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

type UploadID = String;

// S3-defined part sized limits. These are hard limits which we cannot bypass.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB
const MAX_PARTS: u64 = 10_000; // 10,000 parts

pub struct Source {
    pub file: File,
    pub size: u64,
}

impl Store {
    /// Streams a file to S3 as a **stateless** multipart upload, one part at a time.
    ///
    /// A fresh upload is created every call: on any in-process failure the
    /// in-progress upload is aborted (best-effort) so S3 does not retain orphaned
    /// parts, then the error propagates.
    pub async fn put_multipart(
        &self,
        src: &Source,
        dst: &Object,
        metadata: &std::collections::HashMap<String, String>,
    ) -> Result<(), S3Err> {
        let upload_id = self.create_multipart_upload(dst, metadata).await?;

        match self.exec_multipart_upload(src, dst, &upload_id).await {
            Ok(()) => Ok(()),
            Err(err) => {
                // Best-effort cleanup: don't mask the original error if the abort
                // itself fails.
                let _ = self.abort_multipart_upload(dst, &upload_id).await;
                Err(err)
            }
        }
    }

    /// Resumes an existing multipart upload: lists the parts that already landed
    /// in S3, uploads only the missing parts, and completes. Never aborts, so a
    /// resume is safe to retry.
    ///
    /// `NoSuchUploadErr` if S3 no longer knows the upload. The caller must resume
    /// against the same file bytes the upload was started for.
    pub async fn resume_multipart_upload(
        &self,
        src: &Source,
        dst: &Object,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let landed = self.list_parts(dst, upload_id).await?;
        let parts = self.upload_parts(src, dst, upload_id, &landed).await?;
        self.complete_multipart_upload(dst, upload_id, &parts).await
    }

    /// Walks [`Self::part_plan`] in order. For each part number present in
    /// `landed`, reuses that [`CompletedPart`]; otherwise uploads the range.
    /// Pass an empty map to upload every part (the fresh-upload path).
    async fn upload_parts(
        &self,
        src: &Source,
        dst: &Object,
        upload_id: &str,
        landed: &std::collections::HashMap<i32, CompletedPart>,
    ) -> Result<Vec<CompletedPart>, S3Err> {
        let mut parts: Vec<CompletedPart> = Vec::new();
        for (part_number, offset, len) in Self::part_plan(src.size) {
            let part = match landed.get(&part_number) {
                Some(existing) => existing.clone(),
                None => {
                    self.upload_part(&src.file, dst, upload_id, part_number, offset, len)
                        .await?
                }
            };
            parts.push(part);
        }
        Ok(parts)
    }

    /// Lists every part already uploaded for `upload_id`, following pagination,
    /// keyed by part number for reuse by [`Self::upload_parts`].
    async fn list_parts(
        &self,
        obj: &Object,
        upload_id: &str,
    ) -> Result<std::collections::HashMap<i32, CompletedPart>, S3Err> {
        let mut parts: std::collections::HashMap<i32, CompletedPart> =
            std::collections::HashMap::new();
        let mut marker: Option<String> = None;

        loop {
            let Some(page) = self
                .list_parts_page(obj, upload_id, marker.as_deref())
                .await?
            else {
                return Err(S3Err::NoSuchUploadErr(NoSuchUploadErr {
                    object: obj.clone(),
                    upload_id: upload_id.to_string(),
                    trace: crate::trace!(),
                }));
            };

            for part in page.parts() {
                let (Some(number), Some(etag)) = (part.part_number(), part.e_tag()) else {
                    continue;
                };
                parts.insert(
                    number,
                    CompletedPart::builder()
                        .part_number(number)
                        .e_tag(etag)
                        .build(),
                );
            }

            match page.next_part_number_marker() {
                Some(next) if page.is_truncated() == Some(true) => marker = Some(next.to_string()),
                _ => return Ok(parts),
            }
        }
    }

    /// Fetches one page of [`Self::list_parts`], resuming after `marker` when given.
    /// `Ok(None)` if S3 reports the upload no longer exists (404 / NoSuchUpload);
    /// [`Self::list_parts`] turns that into [`S3Err::NoSuchUploadErr`].
    async fn list_parts_page(
        &self,
        obj: &Object,
        upload_id: &str,
        marker: Option<&str>,
    ) -> Result<Option<ListPartsOutput>, S3Err> {
        let mut req = self
            .client
            .list_parts()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .upload_id(upload_id);
        if let Some(marker) = marker {
            req = req.part_number_marker(marker);
        }

        match req.send().await {
            Ok(page) => Ok(Some(page)),
            Err(err) if errors::is_not_found(&err) => Ok(None),
            Err(err) => Err(errors::map_sdk_err("list_parts", obj, err)),
        }
    }

    /// Picks a part size that keeps the part count within S3's 10,000-part limit. Uses
    /// the fixed [`PART_SIZE`] until a file is large enough to need more than 10,000
    /// such parts, then grows the part size to `ceil(size / 10_000)` (never below the 5
    /// MiB floor).
    pub(crate) fn part_size_for(size: u64) -> u64 {
        if size.div_ceil(PART_SIZE) <= MAX_PARTS {
            PART_SIZE
        } else {
            size.div_ceil(MAX_PARTS).max(MIN_PART_SIZE)
        }
    }

    /// The `(1-based part number, byte offset, byte length)` of each part for an
    /// object of `size` bytes. Every byte is covered exactly once; the final part
    /// carries the remainder. Uses [`Self::part_size_for`] to pick the part size,
    /// so the plan always fits within S3's 10,000-part limit.
    fn part_plan(size: u64) -> Vec<(i32, u64, u64)> {
        let part_size = Self::part_size_for(size);
        let mut plan = Vec::new();
        let mut offset: u64 = 0;
        let mut part_number: i32 = 1; // S3 part numbers are 1-based.

        while offset < size {
            let len = part_size.min(size - offset);
            plan.push((part_number, offset, len));
            offset += len;
            part_number += 1;
        }

        plan
    }

    /// Starts a multipart upload and returns its `upload_id`.
    async fn create_multipart_upload(
        &self,
        dst: &Object,
        metadata: &std::collections::HashMap<String, String>,
    ) -> Result<UploadID, S3Err> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .set_metadata((!metadata.is_empty()).then(|| metadata.clone()))
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("create_multipart_upload", dst, e))?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| {
                errors::missing_response_field("create_multipart_upload", dst, "an upload id")
            })?
            .to_string();
        Ok(upload_id)
    }

    /// Uploads every part and completes the multipart upload. Split out from
    /// [`Self::put_multipart`] so a single `?` early-return path funnels through
    /// one abort site: any failure here propagates as one `Err` that
    /// `put_multipart` catches to issue a best-effort abort.
    async fn exec_multipart_upload(
        &self,
        src: &Source,
        dst: &Object,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let parts = self
            .upload_parts(src, dst, upload_id, &std::collections::HashMap::new())
            .await?;
        self.complete_multipart_upload(dst, upload_id, &parts).await
    }

    /// Reads `src[offset..offset+length]` into an in-memory buffer, mapping any
    /// open/seek/read failure to a terminal [`S3Err::LocalIoErr`] via
    /// [`errors::map_body_io_err`]. Peak memory is one part (`length` bytes); the
    /// caller uploads parts sequentially, so at most one part is buffered at a
    /// time. A file that shrank below `offset + length` makes `read_exact` return
    /// `UnexpectedEof`, which maps to the same terminal `LocalIoErr`.
    async fn read_part_bytes(
        src: &File,
        dst: &Object,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, S3Err> {
        let mut f = tokio::fs::File::open(src.path())
            .await
            .map_err(|e| errors::map_body_io_err("upload_part", dst, src, e))?;
        f.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| errors::map_body_io_err("upload_part", dst, src, e))?;
        let mut buf = vec![0u8; length as usize];
        f.read_exact(&mut buf)
            .await
            .map_err(|e| errors::map_body_io_err("upload_part", dst, src, e))?;
        Ok(buf)
    }

    /// Streams a single part (`file[offset..offset+len]`) to S3 and returns the
    /// [`CompletedPart`] describing it. `InvalidResponseErr` if the response
    /// omits the ETag.
    async fn upload_part(
        &self,
        src: &File,
        dst: &Object,
        upload_id: &str,
        part_number: i32,
        offset: u64,
        length: u64,
    ) -> Result<CompletedPart, S3Err> {
        let buf = Self::read_part_bytes(src, dst, offset, length).await?;
        let body = ByteStream::from(buf);

        let output = self
            .client
            .upload_part()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(body)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("upload_part", dst, e))?;

        let etag = output
            .e_tag()
            .ok_or_else(|| errors::missing_response_field("upload_part", dst, "an etag"))?;
        Ok(CompletedPart::builder()
            .part_number(part_number)
            .e_tag(etag)
            .build())
    }

    /// Completes a multipart upload from the `(part_number, etag)` pairs of the
    /// landed parts.
    async fn complete_multipart_upload(
        &self,
        obj: &Object,
        upload_id: &str,
        parts: &[CompletedPart],
    ) -> Result<(), S3Err> {
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(parts.to_vec()))
            .build();
        self.client
            .complete_multipart_upload()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .upload_id(upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("complete_multipart_upload", obj, e))?;
        Ok(())
    }

    /// Aborts an in-progress multipart upload so S3 releases its parts. Returns a
    /// `Result` so callers decide whether to treat the abort as best-effort.
    async fn abort_multipart_upload(&self, obj: &Object, upload_id: &str) -> Result<(), S3Err> {
        self.client
            .abort_multipart_upload()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("abort_multipart_upload", obj, e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::HashMap;

    // internal crates
    use super::*;
    use crate::s3::tests::{
        access_denied_resp, actual_shapes, obj, req, resp, resp_xml, shape, store_expecting,
        store_with, temp_file_with, uri, BUCKET,
    };
    use crate::test_utils::filesys::files as test_files;
    use miru_agent::errors::{Code, Error};
    use miru_agent::filesys::files;

    // external crates
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    #[test]
    fn part_size_uses_fixed_size_below_the_part_ceiling() {
        // A file that fits in ≤ 10,000 fixed-size parts keeps the fixed size.
        assert_eq!(Store::part_size_for(0), PART_SIZE);
        assert_eq!(Store::part_size_for(PART_SIZE), PART_SIZE);
        assert_eq!(Store::part_size_for(PART_SIZE * MAX_PARTS), PART_SIZE);
    }

    #[test]
    fn part_size_grows_to_stay_under_the_part_ceiling() {
        // One byte past the fixed-size ceiling forces a larger part size so the
        // count stays ≤ 10,000.
        let size = PART_SIZE * MAX_PARTS + 1;
        let part = Store::part_size_for(size);
        assert!(part > PART_SIZE);
        assert!(size.div_ceil(part) <= MAX_PARTS);
    }

    #[test]
    fn part_size_never_drops_below_the_minimum() {
        // Pathological: a size that would compute a sub-5-MiB part is floored at
        // the S3 minimum. `ceil(size / 10_000)` < 5 MiB when size is small, but
        // such sizes take the fixed-size branch; to hit the floor directly we
        // check the max() guard holds at the branch boundary.
        assert!(Store::part_size_for(u64::MAX) >= MIN_PART_SIZE);
    }

    #[test]
    fn part_plan_final_part_carries_the_remainder() {
        // A full part plus a small tail: two parts, the last carrying the leftover.
        assert_eq!(
            Store::part_plan(PART_SIZE + 1024),
            vec![(1, 0, PART_SIZE), (2, PART_SIZE, 1024)]
        );
    }

    #[test]
    fn part_plan_exact_multiple_has_no_trailing_zero_part() {
        // An exact multiple of the part size splits evenly with no zero-length tail.
        assert_eq!(
            Store::part_plan(3 * PART_SIZE),
            vec![
                (1, 0, PART_SIZE),
                (2, PART_SIZE, PART_SIZE),
                (3, 2 * PART_SIZE, PART_SIZE),
            ]
        );
    }

    #[test]
    fn part_plan_single_full_part() {
        // Exactly one part size yields a single full part.
        assert_eq!(Store::part_plan(PART_SIZE), vec![(1, 0, PART_SIZE)]);
    }

    #[test]
    fn part_plan_growing_part_size_stays_within_limit_and_covers_every_byte() {
        // A size that forces `part_size_for` to grow past the fixed size. The plan
        // must stay within the part limit, be contiguous with ascending 1-based
        // part numbers, and cover every byte exactly once (no gap/overlap).
        let size = PART_SIZE * MAX_PARTS + 1;
        let plan = Store::part_plan(size);

        assert!(plan.len() as u64 <= MAX_PARTS);

        let mut expected_offset: u64 = 0;
        let mut total_len: u64 = 0;
        for (i, &(part_number, offset, len)) in plan.iter().enumerate() {
            assert_eq!(part_number, (i + 1) as i32); // 1-based, ascending.
            assert_eq!(offset, expected_offset); // contiguous.
            expected_offset += len;
            total_len += len;
        }
        assert_eq!(total_len, size); // full coverage, no gap/overlap.
    }

    const UPLOAD_ID: &str = "test-upload-id";

    /// Builds a `Source` from a temp file, reading its length off disk with the
    /// crate's own `files::size`.
    async fn source_of(tf: &test_files::TempFile) -> Source {
        let file = tf.to_file();
        let size = files::size(&file).await.unwrap();
        Source { file, size }
    }

    fn create_resp() -> http::Response<SdkBody> {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><UploadId>{UPLOAD_ID}</UploadId></InitiateMultipartUploadResult>"#
        );
        resp_xml(200, &xml)
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
        resp_xml(200, &xml)
    }

    fn create_req() -> http::Request<SdkBody> {
        http::Request::builder()
            .method("POST")
            .uri(uri("big.bin?uploads"))
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
    fn create_shape() -> (String, String) {
        shape("POST", "big.bin?uploads")
    }

    fn upload_part_shape(number: i32) -> (String, String) {
        shape(
            "PUT",
            &format!("big.bin?x-id=UploadPart&partNumber={number}&uploadId={UPLOAD_ID}"),
        )
    }

    fn complete_shape() -> (String, String) {
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
        async fn two_part_file() -> test_files::TempFile {
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

    /// Size-based routing in [`Store::put`]: small files take the single
    /// `PutObject` path; larger-than-`PART_SIZE` files take the multipart path.
    mod routing {
        use super::*;

        #[tokio::test]
        async fn small_file_routes_to_single_put() {
            // A body well under PART_SIZE must take the single-part branch:
            // exactly one PutObject, no multipart calls.
            let src = temp_file_with(b"tiny").await;
            let (store, replay) =
                store_expecting(req("PUT", "small.bin?x-id=PutObject"), resp(200, &[]));

            store
                .put(src.to_file(), &obj("small.bin"), &HashMap::new())
                .await
                .unwrap();

            assert_eq!(
                actual_shapes(&replay),
                vec![shape("PUT", "small.bin?x-id=PutObject")]
            );
        }

        #[tokio::test]
        async fn large_file_routes_to_multipart() {
            // The crate constant is private; re-declare it locally to size a
            // fixture just past the routing threshold. 8 MiB + 1 KiB => 2 parts
            // (8 MiB, 1 KiB).
            const PART_SIZE: u64 = 8 * 1024 * 1024;
            // Recognizable byte pattern so each part's body can be checked against its slice.
            let big: Vec<u8> = (0..(PART_SIZE + 1024)).map(|i| i as u8).collect();
            let src = temp_file_with(&big).await;

            let (store, replay) = store_with(vec![
                ReplayEvent::new(create_req(), create_resp()),
                ReplayEvent::new(upload_part_req(1), upload_part_resp("\"etag-part-1\"")),
                ReplayEvent::new(upload_part_req(2), upload_part_resp("\"etag-part-2\"")),
                ReplayEvent::new(complete_req(), complete_resp()),
            ]);

            store
                .put(src.to_file(), &obj("big.bin"), &HashMap::new())
                .await
                .unwrap();

            assert_eq!(
                actual_shapes(&replay),
                vec![
                    create_shape(),
                    upload_part_shape(1),
                    upload_part_shape(2),
                    complete_shape(),
                ]
            );

            // The CompleteMultipartUpload manifest is a small in-memory XML body,
            // so its bytes are readable off the recorded request. Assert it lists
            // both parts in order with their matching etags.
            let requests = replay.actual_requests().collect::<Vec<_>>();
            let complete_body = requests
                .last()
                .expect("a complete request was recorded")
                .body()
                .bytes()
                .expect("the complete manifest is an in-memory body");
            let manifest = std::str::from_utf8(complete_body).expect("manifest is UTF-8");

            let part1 = manifest
                .find("<PartNumber>1</PartNumber>")
                .expect("manifest lists part 1");
            let part2 = manifest
                .find("<PartNumber>2</PartNumber>")
                .expect("manifest lists part 2");
            assert!(part1 < part2, "parts must appear in ascending order");
            assert!(
                manifest.contains("etag-part-1"),
                "manifest carries part 1's etag"
            );
            assert!(
                manifest.contains("etag-part-2"),
                "manifest carries part 2's etag"
            );

            // Parts are sent as in-memory buffers, so their exact bytes are recorded:
            // each part must carry its own file slice (right offset and length).
            assert!(
                requests[1]
                    .body()
                    .bytes()
                    .expect("part 1 body is in-memory")
                    == &big[..PART_SIZE as usize],
                "part 1 must be the first PART_SIZE bytes of the source"
            );
            assert!(
                requests[2]
                    .body()
                    .bytes()
                    .expect("part 2 body is in-memory")
                    == &big[PART_SIZE as usize..],
                "part 2 must be the remaining bytes of the source"
            );
        }
    }
}
