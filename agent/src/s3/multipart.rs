//! Multipart upload machinery for [`Store`].
//!
//! S3 requires objects larger than a single `PutObject` to be uploaded in
//! parts: create an upload, upload each chunk, then complete (or abort) it. This
//! module holds the stateless multipart surface ([`Store::put_multipart`]) over a
//! set of internal per-part primitives (create / upload_part / complete / abort)
//! plus the part-sizing policy ([`Store::part_size_for`]). The single-part path
//! and the rest of the object API live in the parent [`crate::s3`] module.

// internal crates
use crate::filesys::file::File;
use crate::filesys::path::PathExt;
use crate::s3::errors;
use crate::s3::{Object, S3Err, Store, PART_SIZE};

// external crates
use aws_sdk_s3::primitives::{ByteStream, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};

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
    ///
    /// The `Source` file is treated as an immutable snapshot of `size` bytes
    /// captured at call time; if the file changes during the upload, behavior is
    /// undefined (shrinking makes a part read fail; growth means only the original
    /// `size` bytes are uploaded).
    ///
    /// If the returned future is dropped (caller cancellation/timeout) after the
    /// upload is created, the best-effort abort does not run and an in-progress
    /// upload is left on S3; deployments should configure an S3
    /// `AbortIncompleteMultipartUpload` lifecycle rule.
    pub async fn put_multipart(&self, src: &Source, dst: &Object) -> Result<(), S3Err> {
        let upload_id = self.create_multipart_upload(dst).await?;

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
    async fn create_multipart_upload(&self, dst: &Object) -> Result<UploadID, S3Err> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err("create_multipart_upload", Some(dst.key.to_string()), e)
            })?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| {
                errors::missing_response_field("create_multipart_upload", "an upload id")
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
        let parts = self.upload_parts(src, dst, upload_id).await?;
        self.complete_multipart_upload(dst, upload_id, &parts).await
    }

    /// Streams each part of `src` (from byte 0) to S3 in order, returning the
    /// [`CompletedPart`]s (part number, ETag) needed to complete the upload.
    async fn upload_parts(
        &self,
        src: &Source,
        dst: &Object,
        upload_id: &str,
    ) -> Result<Vec<CompletedPart>, S3Err> {
        let mut parts: Vec<CompletedPart> = Vec::new();

        for (part_number, offset, len) in Self::part_plan(src.size) {
            let part = self
                .upload_part(&src.file, dst, upload_id, part_number, offset, len)
                .await?;
            parts.push(part);
        }

        Ok(parts)
    }

    /// Streams a single part (`file[offset..offset+len]`) to S3 and returns the
    /// [`CompletedPart`] describing it. `InvalidResponseErr` if the response
    /// omits the ETag.
    ///
    /// Opening the source file for streaming maps to `LocalIoErr` via
    /// [`errors::map_bytestream_err`], but a failure reading part bytes
    /// *mid-stream* surfaces through the SDK `send()` as a dispatch error mapped by
    /// [`errors::map_sdk_err`] to a (retryable) `ConnectionErr`. This is a known
    /// classification limitation: a mid-stream local disk read error is currently
    /// treated as retryable, unlike the `get()` path.
    async fn upload_part(
        &self,
        src: &File,
        dst: &Object,
        upload_id: &str,
        part_number: i32,
        offset: u64,
        length: u64,
    ) -> Result<CompletedPart, S3Err> {
        let body = ByteStream::read_from()
            .path(src.path())
            .offset(offset)
            .length(Length::Exact(length))
            .build()
            .await
            .map_err(|e| errors::map_bytestream_err("upload_part", dst, src, &e))?;

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
            .map_err(|e| errors::map_sdk_err("upload_part", Some(dst.key.to_string()), e))?;

        let etag = output
            .e_tag()
            .ok_or_else(|| errors::missing_response_field("upload_part", "an etag"))?;
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
            .map_err(|e| {
                errors::map_sdk_err("complete_multipart_upload", Some(obj.key.to_string()), e)
            })?;
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
            .map_err(|e| {
                errors::map_sdk_err("abort_multipart_upload", Some(obj.key.to_string()), e)
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
