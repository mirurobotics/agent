//! Remote S3 object storage.
//!
//! This module talks to an S3 bucket over the network. It is distinct from
//! [`crate::disk`], which manages local on-disk device state (device.json,
//! settings.json, ...). Do not conflate the two.
//!
//! An [`S3Store`] is constructed **only** from caller-supplied temporary
//! credentials (access key id, secret access key, session token) plus a region
//! and bucket name. It never reads ambient AWS configuration (environment
//! variables, `~/.aws`, or EC2/ECS instance metadata): the Miru backend mints
//! short-lived STS credentials and hands them to the agent.
//!
//! Object bodies are streamed to and from disk (never buffered whole in
//! memory) so the agent can move multi-gigabyte artifacts on memory-constrained
//! devices. Uploads larger than [`DEFAULT_MULTIPART_THRESHOLD`] use a multipart
//! upload.

// internal crates
use crate::filesys::file::File;
use crate::filesys::path::PathExt;
use crate::trace;

// external crates
use aws_sdk_s3::config::{BehaviorVersion, Credentials as AwsCredentials, Region};
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::{ByteStream, ByteStreamError, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::Client;

pub mod errors;

pub use errors::S3Err;
use errors::{InvalidResponseErr, ObjectNotFoundErr};

const PART_SIZE: u64 = 8 * 1024 * 1024;
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
const MAX_PARTS: u64 = 10_000;

/// Files larger than this stream through a multipart upload; files at or below
/// it go through a single `PutObject`.
const DEFAULT_MULTIPART_THRESHOLD: u64 = 8 * 1024 * 1024;

/// Caller-supplied temporary AWS credentials (from an STS AssumeRole).
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

/// An async S3 client scoped to a single bucket.
pub struct S3Store {
    client: Client,
    bucket: String,
    single_put_threshold: u64,
}

impl S3Store {
    /// Builds a client from caller-supplied temporary credentials. No network
    /// I/O happens here; the first request is made lazily on the first call.
    pub fn new(creds: Credentials, region: String, bucket: String) -> Self {
        let credentials = AwsCredentials::new(
            creds.access_key_id,
            creds.secret_access_key,
            Some(creds.session_token),
            None,
            "miru-agent",
        );
        let config = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(credentials)
            .build();
        Self {
            client: Client::from_conf(config),
            bucket,
            single_put_threshold: DEFAULT_MULTIPART_THRESHOLD,
        }
    }

    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) in place of the default HTTPS connector, so tests
    /// serve canned responses without touching the network. Credentials are
    /// static dummies since no real signing endpoint is contacted.
    #[cfg(feature = "test")]
    pub fn with_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
        region: String,
        bucket: String,
    ) -> Self {
        let credentials =
            AwsCredentials::new("test-access-key", "test-secret-key", None, None, "test");
        let config = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(credentials)
            // Path-style URLs (`/<bucket>/<key>`) make the replayed request URIs
            // deterministic and readable, so tests can assert on the exact path.
            .force_path_style(true)
            .http_client(http_client)
            .build();
        Self {
            client: Client::from_conf(config),
            bucket,
            single_put_threshold: DEFAULT_MULTIPART_THRESHOLD,
        }
    }

    /// Test-only seam to force the multipart path with small fixtures by
    /// lowering the single-PUT threshold (e.g. to 0).
    #[cfg(feature = "test")]
    pub fn set_single_put_threshold(&mut self, bytes: u64) {
        self.single_put_threshold = bytes;
    }

    /// Creates or overwrites an object by streaming a file off disk.
    ///
    /// The whole file is never held in memory: files at or below the
    /// single-PUT threshold stream through one `PutObject`, and larger files
    /// stream part-by-part through a multipart upload (see
    /// [`Self::put_object_multipart`]).
    pub async fn put_file(&self, key: &str, file: File) -> Result<(), S3Err> {
        let size = file.size().await?;

        if size > self.single_put_threshold {
            return self.put_file_multipart(key, &file, size).await;
        }

        let body = ByteStream::from_path(file.path())
            .await
            .map_err(|e| self.map_bytestream_err("put_object", key, &file, &e))?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("put_object", Some(key.to_string()), e))?;
        Ok(())
    }

    /// Streams a file to S3 as a multipart upload, one part at a time. On any
    /// failure during the part loop or completion, the in-progress upload is
    /// aborted (best-effort) so S3 does not retain orphaned parts.
    async fn put_file_multipart(&self, key: &str, file: &File, size: u64) -> Result<(), S3Err> {
        let part_size = Self::part_size_for(size);

        let created = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err_common("create_multipart_upload", Some(key.to_string()), e)
            })?;
        let upload_id = created.upload_id().ok_or_else(|| {
            S3Err::InvalidResponseErr(InvalidResponseErr {
                operation: "create_multipart_upload".to_string(),
                msg: "response did not include an upload id".to_string(),
                trace: trace!(),
            })
        })?;

        match self
            .upload_parts_and_complete(key, file, size, part_size, upload_id)
            .await
        {
            Ok(()) => Ok(()),
            Err(err) => {
                // Best-effort cleanup: don't mask the original error if the
                // abort itself fails.
                let _ = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(upload_id)
                    .send()
                    .await;
                Err(err)
            }
        }
    }

    /// Uploads every part of `file` and completes the multipart upload. Split
    /// out from [`Self::put_object_multipart`] so a single `?` early-return path
    /// funnels through one abort site.
    async fn upload_parts_and_complete(
        &self,
        key: &str,
        file: &File,
        size: u64,
        part_size: u64,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let mut completed_parts: Vec<CompletedPart> = Vec::new();
        let mut offset: u64 = 0;
        // S3 part numbers are 1-based.
        let mut part_number: i32 = 1;

        while offset < size {
            let len = part_size.min(size - offset);
            let body = ByteStream::read_from()
                .path(file.path())
                .offset(offset)
                .length(Length::Exact(len))
                .build()
                .await
                .map_err(|e| self.map_bytestream_err("upload_part", key, file, &e))?;

            let part = self
                .client
                .upload_part()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(upload_id)
                .part_number(part_number)
                .body(body)
                .send()
                .await
                .map_err(|e| errors::map_sdk_err_common("upload_part", Some(key.to_string()), e))?;

            completed_parts.push(
                CompletedPart::builder()
                    .part_number(part_number)
                    .set_e_tag(part.e_tag().map(str::to_string))
                    .build(),
            );

            offset += len;
            part_number += 1;
        }

        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();
        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err_common("complete_multipart_upload", Some(key.to_string()), e)
            })?;
        Ok(())
    }

    /// Picks a part size that keeps the part count within S3's 10,000-part
    /// limit. Uses the fixed [`PART_SIZE`] until a file is large enough to need
    /// more than 10,000 such parts, then grows the part size to `ceil(size /
    /// 10_000)` (never below the 5 MiB floor).
    fn part_size_for(size: u64) -> u64 {
        if size.div_ceil(PART_SIZE) <= MAX_PARTS {
            PART_SIZE
        } else {
            size.div_ceil(MAX_PARTS).max(MIN_PART_SIZE)
        }
    }

    /// Streams an object's body to a destination file. A missing object maps to
    /// [`S3Err::ObjectNotFoundErr`]. The body is copied through a bounded buffer
    /// rather than collected into memory.
    pub async fn get_object(&self, key: &str, dest: &File) -> Result<(), S3Err> {
        let output = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(err) => {
                let is_not_found = err
                    .raw_response()
                    .map(|r| r.status().as_u16() == 404)
                    .unwrap_or(false)
                    || matches!(err.as_service_error(), Some(GetObjectError::NoSuchKey(_)));
                if is_not_found {
                    return Err(S3Err::ObjectNotFoundErr(ObjectNotFoundErr {
                        key: key.to_string(),
                        trace: trace!(),
                    }));
                }
                return Err(errors::map_sdk_err_common(
                    "get_object",
                    Some(key.to_string()),
                    err,
                ));
            }
        };

        let mut reader = output.body.into_async_read();
        let mut file = tokio::fs::File::create(dest.path())
            .await
            .map_err(|e| self.map_body_io_err("get_object", key, dest, e))?;
        tokio::io::copy(&mut reader, &mut file)
            .await
            .map_err(|e| self.map_body_io_err("get_object", key, dest, e))?;
        Ok(())
    }

    /// Deletes an object. Idempotent per S3 semantics (deleting a missing key
    /// still returns success).
    pub async fn delete_object(&self, key: &str) -> Result<(), S3Err> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("delete_object", Some(key.to_string()), e))?;
        Ok(())
    }

    /// Returns `true` if the object exists (HEAD 200), `false` on a 404. Other
    /// errors propagate.
    pub async fn object_exists(&self, key: &str) -> Result<bool, S3Err> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                let is_not_found = err
                    .raw_response()
                    .map(|r| r.status().as_u16() == 404)
                    .unwrap_or(false);
                if is_not_found {
                    Ok(false)
                } else {
                    Err(errors::map_sdk_err_common(
                        "head_object",
                        Some(key.to_string()),
                        err,
                    ))
                }
            }
        }
    }

    /// Maps a filesystem I/O error hit while streaming an object body (opening
    /// the source, creating the destination, or copying) into an `S3Err`.
    fn map_body_io_err(
        &self,
        operation: &str,
        key: &str,
        file: &File,
        err: std::io::Error,
    ) -> S3Err {
        S3Err::InvalidResponseErr(InvalidResponseErr {
            operation: operation.to_string(),
            msg: format!("filesystem I/O error for object '{key}' at path '{file}': {err}"),
            trace: trace!(),
        })
    }

    /// Maps a [`ByteStream`] construction error (e.g. the file could not be
    /// opened or read) into an `S3Err`.
    fn map_bytestream_err(
        &self,
        operation: &str,
        key: &str,
        file: &File,
        err: &ByteStreamError,
    ) -> S3Err {
        S3Err::InvalidResponseErr(InvalidResponseErr {
            operation: operation.to_string(),
            msg: format!("failed to open '{file}' for streaming object '{key}': {err}"),
            trace: trace!(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_size_uses_fixed_size_below_the_part_ceiling() {
        // A file that fits in ≤ 10,000 fixed-size parts keeps the fixed size.
        assert_eq!(S3Store::part_size_for(0), PART_SIZE);
        assert_eq!(S3Store::part_size_for(PART_SIZE), PART_SIZE);
        assert_eq!(S3Store::part_size_for(PART_SIZE * MAX_PARTS), PART_SIZE);
    }

    #[test]
    fn part_size_grows_to_stay_under_the_part_ceiling() {
        // One byte past the fixed-size ceiling forces a larger part size so the
        // count stays ≤ 10,000.
        let size = PART_SIZE * MAX_PARTS + 1;
        let part = S3Store::part_size_for(size);
        assert!(part > PART_SIZE);
        assert!(size.div_ceil(part) <= MAX_PARTS);
    }

    #[test]
    fn part_size_never_drops_below_the_minimum() {
        // Pathological: a size that would compute a sub-5-MiB part is floored at
        // the S3 minimum. `ceil(size / 10_000)` < 5 MiB when size is small, but
        // such sizes take the fixed-size branch; to hit the floor directly we
        // check the max() guard holds at the branch boundary.
        assert!(S3Store::part_size_for(u64::MAX) >= MIN_PART_SIZE);
    }
}
