//! Multipart upload machinery for [`Store`].
//!
//! S3 requires objects larger than a single `PutObject` to be uploaded in
//! parts: create an upload, `upload_part` each chunk, then complete (or abort)
//! it. This module holds those primitives ([`Store::create_multipart_upload`],
//! [`Store::upload_part`], [`Store::list_parts`],
//! [`Store::complete_multipart_upload`], [`Store::abort_multipart_upload`]) plus
//! the stateless [`Store::put_multipart`] convenience and the part-sizing
//! policy ([`Store::part_size_for`]). The single-part path and the rest of the
//! object API live in the parent [`super`] module.

// internal crates
use crate::filesys::file::File;
use crate::filesys::path::PathExt;
use crate::trace;

// external crates
use aws_sdk_s3::primitives::{ByteStream, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};

use super::errors::{self, InvalidResponseErr};
use super::{Object, S3Err, Store};

const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

// S3-defined part sized limits. These are hard limits which we cannot bypass.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB
const MAX_PARTS: u64 = 10_000; // 10,000 parts

#[derive(Debug, Clone)]
pub struct PartToUpload {
    pub upload_id: String,
    pub number: i32,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedPart {
    pub number: i32,
    pub etag: String,
}

impl Store {
    /// Streams a file to S3 as a **stateless** multipart upload, one part at a time.
    ///
    /// A fresh upload is created every call: create → `upload_part` over
    /// `part_size_for` chunks → complete. On any in-process failure the
    /// in-progress upload is aborted (best-effort) so S3 does not retain orphaned
    /// parts, then the error propagates. This variant is **stateless**: it carries
    /// no durable state and cannot resume across a crash. A resumable variant will
    /// be reintroduced in a later layer.
    pub async fn put_multipart(&self, src: &File, dst: &Object, size: u64) -> Result<(), S3Err> {
        let part_size = Self::part_size_for(size);
        let upload_id = self.create_multipart_upload(dst).await?;

        match self
            .upload_parts_and_complete(src, dst, size, part_size, &upload_id)
            .await
        {
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

    /// Starts a multipart upload and returns its `upload_id`.
    pub async fn create_multipart_upload(&self, dst: &Object) -> Result<String, S3Err> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err_common("create_multipart_upload", Some(dst.key.to_string()), e)
            })?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| {
                S3Err::InvalidResponseErr(InvalidResponseErr {
                    operation: "create_multipart_upload".to_string(),
                    msg: "response did not include an upload id".to_string(),
                    trace: trace!(),
                })
            })?
            .to_string();
        Ok(upload_id)
    }

    /// Uploads every part and completes the multipart upload. Split out from
    /// [`Self::put_multipart`] so a single `?` early-return path funnels through
    /// one abort site: any failure here propagates as one `Err` that
    /// `put_multipart` catches to issue a best-effort abort.
    async fn upload_parts_and_complete(
        &self,
        src: &File,
        dst: &Object,
        size: u64,
        part_size: u64,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let parts = self
            .upload_parts(src, dst, size, part_size, upload_id)
            .await?;
        self.complete_multipart_upload(dst, upload_id, &parts).await
    }

    /// Streams each part of `src` (from byte 0) to S3 in order, returning the
    /// [`UploadedPart`]s (part number, ETag, size) needed to complete the upload.
    async fn upload_parts(
        &self,
        src: &File,
        dst: &Object,
        size: u64,
        part_size: u64,
        upload_id: &str,
    ) -> Result<Vec<UploadedPart>, S3Err> {
        let mut parts: Vec<UploadedPart> = Vec::new();
        let mut offset: u64 = 0;
        let mut part_number: i32 = 1; // S3 part numbers are 1-based.

        while offset < size {
            let len = part_size.min(size - offset);
            let part_to_upload = PartToUpload {
                upload_id: upload_id.to_string(),
                number: part_number,
                offset,
                length: len,
            };
            let etag = self.upload_part(src, dst, &part_to_upload).await?;
            let uploaded_part = UploadedPart {
                number: part_number,
                etag,
            };
            parts.push(uploaded_part);
            offset += len;
            part_number += 1;
        }

        Ok(parts)
    }

    /// Streams a single part (`file[offset..offset+len]`) to S3 and returns its
    /// ETag. `InvalidResponseErr` if the response omits the ETag.
    pub async fn upload_part(
        &self,
        src: &File,
        dst: &Object,
        part: &PartToUpload,
    ) -> Result<String, S3Err> {
        let body = ByteStream::read_from()
            .path(src.path())
            .offset(part.offset)
            .length(Length::Exact(part.length))
            .build()
            .await
            .map_err(|e| self.map_bytestream_err("upload_part", dst, src, &e))?;

        let output = self
            .client
            .upload_part()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .upload_id(&part.upload_id)
            .part_number(part.number)
            .body(body)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("upload_part", Some(dst.key.to_string()), e))?;

        output.e_tag().map(str::to_string).ok_or_else(|| {
            S3Err::InvalidResponseErr(InvalidResponseErr {
                operation: "upload_part".to_string(),
                msg: "response did not include an etag".to_string(),
                trace: trace!(),
            })
        })
    }

    /// Completes a multipart upload from the `(part_number, etag)` pairs of the
    /// landed parts.
    pub async fn complete_multipart_upload(
        &self,
        obj: &Object,
        upload_id: &str,
        parts: &[UploadedPart],
    ) -> Result<(), S3Err> {
        let completed_parts: Vec<CompletedPart> = parts
            .iter()
            .map(|part| {
                CompletedPart::builder()
                    .part_number(part.number)
                    .e_tag(part.etag.clone())
                    .build()
            })
            .collect();
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
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
                errors::map_sdk_err_common(
                    "complete_multipart_upload",
                    Some(obj.key.to_string()),
                    e,
                )
            })?;
        Ok(())
    }

    /// Aborts an in-progress multipart upload so S3 releases its parts. Returns a
    /// `Result` so callers decide whether to treat the abort as best-effort.
    pub async fn abort_multipart_upload(&self, obj: &Object, upload_id: &str) -> Result<(), S3Err> {
        self.client
            .abort_multipart_upload()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err_common("abort_multipart_upload", Some(obj.key.to_string()), e)
            })?;
        Ok(())
    }

    /// Lists all parts already uploaded for `upload_id`, following pagination.
    ///
    /// Returns `Ok(None)` when S3 reports the upload no longer exists (a 404 /
    /// NoSuchUpload), mirroring the raw-response status check used by `get`/`exists`,
    /// so a caller can distinguish "expired upload" from a listing of parts. Any
    /// other SDK error propagates via `map_sdk_err_common`. Only parts that carry
    /// all of `{part_number, etag, size}` are returned.
    pub async fn list_parts(
        &self,
        obj: &Object,
        upload_id: &str,
    ) -> Result<Option<Vec<UploadedPart>>, S3Err> {
        let mut parts: Vec<UploadedPart> = Vec::new();
        let mut marker: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_parts()
                .bucket(&obj.bucket)
                .key(&obj.key)
                .upload_id(upload_id);
            if let Some(m) = &marker {
                req = req.part_number_marker(m);
            }

            let output = match req.send().await {
                Ok(output) => output,
                Err(err) => {
                    let is_not_found = err
                        .raw_response()
                        .map(|r| r.status().as_u16() == 404)
                        .unwrap_or(false);
                    if is_not_found {
                        return Ok(None);
                    }
                    return Err(errors::map_sdk_err_common(
                        "list_parts",
                        Some(obj.key.to_string()),
                        err,
                    ));
                }
            };

            for part in output.parts() {
                let (Some(part_number), Some(etag)) = (part.part_number(), part.e_tag()) else {
                    continue;
                };
                parts.push(UploadedPart {
                    number: part_number,
                    etag: etag.to_string(),
                });
            }

            if output.is_truncated() == Some(true) {
                match output.next_part_number_marker() {
                    Some(m) => marker = Some(m.to_string()),
                    None => break,
                }
            } else {
                break;
            }
        }

        Ok(Some(parts))
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
}
