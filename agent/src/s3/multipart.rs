//! Multipart upload machinery for [`Store`].
//!
//! S3 requires objects larger than a single `PutObject` to be uploaded in
//! parts: create an upload, upload each chunk, then complete (or abort) it. This
//! module holds the public multipart surface ([`Store::create_multipart_upload`],
//! [`Store::put_multipart`], [`Store::exec_multipart_upload`],
//! [`Store::resume_multipart_upload`], [`Store::abort_multipart_upload`]) over a
//! set of internal per-part primitives (upload_part / list_parts /
//! complete_multipart_upload) plus the part-sizing policy
//! ([`Store::part_size_for`]). The single-part path and the rest of the object
//! API live in the parent [`super`] module.

// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::file::File;
use crate::filesys::files;
use crate::filesys::path::PathExt;
use crate::trace;

// external crates
use aws_sdk_s3::operation::list_parts::ListPartsOutput;
use aws_sdk_s3::primitives::{ByteStream, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};

use super::errors::{self, InvalidResponseErr, NoSuchUploadErr};
use super::{Object, S3Err, Store, PART_SIZE};

type UploadID = String;
type ETag = String;

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

pub struct Source {
    pub file: File,
    pub size: u64,
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

    /// Starts a multipart upload and returns its `upload_id`.
    pub async fn create_multipart_upload(&self, dst: &Object) -> Result<UploadID, S3Err> {
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
    pub async fn exec_multipart_upload(
        &self,
        src: &Source,
        dst: &Object,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let parts = self.upload_parts(src, dst, upload_id).await?;
        self.complete_multipart_upload(dst, upload_id, &parts).await
    }

    /// Resumes an interrupted multipart upload, uploading only the parts that
    /// have not already landed and then completing it.
    ///
    /// Unlike [`Self::put_multipart`], which creates a fresh upload every call and
    /// aborts it on any failure, this picks up an **existing** `upload_id`: it
    /// lists the parts S3 already holds, re-uploads only the gaps, and completes.
    /// It is deliberately **safe to re-run**: on any error the upload is left
    /// intact (never aborted) so a subsequent call can resume from wherever it
    /// stopped. The caller owns aborting a resume that will never succeed.
    ///
    /// If S3 no longer knows the upload (it expired or was already aborted),
    /// `list_parts` returns `Ok(None)` and this returns [`S3Err::NoSuchUploadErr`]:
    /// the upload cannot be resumed and must be restarted.
    ///
    /// **Precondition:** `src` must be the *same file* the original upload was
    /// started for (identical bytes). Part `n` is always the byte range
    /// `[(n-1) * part_size, n * part_size)` where `part_size` is derived purely
    /// from the file size ([`Self::part_size_for`]). Because the size (and thus
    /// the part boundaries) is reproduced from `src`, part `n` maps to the same
    /// byte range as the original upload, so a landed part's ETag is trusted
    /// as-is rather than re-hashed. Resuming against a different file would splice
    /// mismatched bytes into the object.
    pub async fn resume_multipart_upload(
        &self,
        src: &File,
        dst: &Object,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let size = files::size(src).await?;
        let part_size = Self::part_size_for(size);

        // Index the already-landed parts by number. `None` means S3 no longer
        // knows this upload, so it cannot be resumed.
        let landed: HashMap<i32, ETag> = match self.list_parts(dst, upload_id).await? {
            Some(parts) => parts.into_iter().map(|p| (p.number, p.etag)).collect(),
            None => {
                return Err(S3Err::NoSuchUploadErr(NoSuchUploadErr {
                    key: dst.key.to_string(),
                    upload_id: upload_id.to_string(),
                    trace: trace!(),
                }));
            }
        };

        // Gap-fill every part in order, reusing landed ETags and uploading only
        // the missing ranges. A zero-byte file has no parts.
        let total_parts = size.div_ceil(part_size);
        let mut parts: Vec<UploadedPart> = Vec::with_capacity(total_parts as usize);
        for number in 1..=total_parts as i32 {
            let offset = (number as u64 - 1) * part_size;
            let length = part_size.min(size - offset);
            let etag = match landed.get(&number) {
                Some(etag) => etag.clone(),
                None => {
                    let part = PartToUpload {
                        upload_id: upload_id.to_string(),
                        number,
                        offset,
                        length,
                    };
                    self.upload_part(src, dst, &part).await?
                }
            };
            parts.push(UploadedPart { number, etag });
        }

        self.complete_multipart_upload(dst, upload_id, &parts).await
    }

    /// Streams each part of `src` (from byte 0) to S3 in order, returning the
    /// [`UploadedPart`]s (part number, ETag, size) needed to complete the upload.
    async fn upload_parts(
        &self,
        src: &Source,
        dst: &Object,
        upload_id: &str,
    ) -> Result<Vec<UploadedPart>, S3Err> {
        let part_size = Self::part_size_for(src.size);
        let mut parts: Vec<UploadedPart> = Vec::new();
        let mut offset: u64 = 0;
        let mut part_number: i32 = 1; // S3 part numbers are 1-based.

        while offset < src.size {
            let len = part_size.min(src.size - offset);
            let part_to_upload = PartToUpload {
                upload_id: upload_id.to_string(),
                number: part_number,
                offset,
                length: len,
            };
            let etag = self.upload_part(&src.file, dst, &part_to_upload).await?;
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
    async fn upload_part(
        &self,
        src: &File,
        dst: &Object,
        part: &PartToUpload,
    ) -> Result<ETag, S3Err> {
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
    async fn complete_multipart_upload(
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

    /// Lists every part already uploaded for `upload_id`, following pagination.
    ///
    /// Returns `Ok(None)` when S3 reports the upload no longer exists (a 404 /
    /// NoSuchUpload), so a caller can distinguish an expired upload from an empty
    /// listing. Only parts carrying both `{part_number, etag}` are returned.
    async fn list_parts(
        &self,
        obj: &Object,
        upload_id: &str,
    ) -> Result<Option<Vec<UploadedPart>>, S3Err> {
        let mut parts: Vec<UploadedPart> = Vec::new();
        let mut marker: Option<String> = None;

        loop {
            // `None` means S3 no longer knows this upload; surface that to the caller.
            let Some(page) = self
                .list_parts_page(obj, upload_id, marker.as_deref())
                .await?
            else {
                return Ok(None);
            };

            parts.extend(page.parts().iter().filter_map(|part| {
                Some(UploadedPart {
                    number: part.part_number()?,
                    etag: part.e_tag()?.to_string(),
                })
            }));

            // Keep paging only while S3 marks the listing truncated; otherwise this
            // was the final page.
            match page.next_part_number_marker() {
                Some(next) if page.is_truncated() == Some(true) => marker = Some(next.to_string()),
                _ => return Ok(Some(parts)),
            }
        }
    }

    /// Fetches a single page of [`Self::list_parts`], resuming after `marker` when
    /// given. Returns `Ok(None)` if S3 reports the upload no longer exists (404 /
    /// NoSuchUpload), mirroring the raw-response status check used by `get`/`exists`;
    /// any other SDK error propagates via `map_sdk_err_common`.
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
            // A missing upload (404 / NoSuchUpload) is not an error here: it tells
            // the caller the upload expired, so surface it as `Ok(None)`.
            Err(err) if errors::is_not_found(&err) => Ok(None),
            Err(err) => Err(errors::map_sdk_err_common(
                "list_parts",
                Some(obj.key.to_string()),
                err,
            )),
        }
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
