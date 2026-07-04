//! Remote S3 object storage.
//!
//! This module talks to an S3 bucket over the network. It is distinct from
//! [`crate::disk`], which manages local on-disk device state (device.json,
//! settings.json, ...). Do not conflate the two.
//!
//! A [`Store`] is constructed **only** from caller-supplied temporary
//! credentials (access key id, secret access key, session token) plus a region
//! and bucket name. It never reads ambient AWS configuration (environment
//! variables, `~/.aws`, or EC2/ECS instance metadata): the Miru backend mints
//! short-lived STS credentials and hands them to the agent.
//!
//! Object bodies are streamed to and from disk (never buffered whole in
//! memory) so the agent can move multi-gigabyte artifacts on memory-constrained
//! devices. Uploads larger than [`Options::part_size`] use a multipart upload.
//!
//! [`Store`] is a thin, stateless client: it exposes the S3 multipart
//! primitives ([`Store::create_multipart_upload`], [`Store::upload_part`],
//! [`Store::list_parts`], [`Store::complete_multipart_upload`],
//! [`Store::abort_multipart_upload`]) plus a stateless [`Store::put_multipart`]
//! convenience. Durable, resumable multipart uploads (surviving a power-off)
//! live one layer up in [`uploader::Uploader`], which owns the on-disk upload
//! state and the resume-vs-restart policy.

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
pub mod uploader;

pub use errors::S3Err;
use errors::{InvalidResponseErr, ObjectNotFoundErr};

const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

// S3-defined part sized limits. These are hard limits which we cannot bypass.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB
const MAX_PARTS: u64 = 10_000; // 10,000 parts

/// One already-uploaded part as reported by S3 `ListParts`.
pub struct PartInfo {
    pub part_number: i32,
    pub etag: String,
    pub size: u64,
}

pub struct Config {
    pub creds: Credentials,
    pub region: String,
    pub bucket: String,
}

pub struct Options {
    pub part_size: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            // Miru-chosen default part size. 8 MiB is the sweet spot for getting good
            // performance while minimizing the need to retry upload parts.
            part_size: 8 * 1024 * 1024, // 8 MiB
        }
    }
}

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

impl Default for Credentials {
    fn default() -> Self {
        Self {
            access_key_id: "access-key".to_string(),
            secret_access_key: "secret-key".to_string(),
            session_token: "session-token".to_string(),
        }
    }
}

pub struct Store {
    client: Client,
    bucket: String,
    opts: Options,
}

impl Store {
    /// Builds a client from caller-supplied temporary credentials. No network
    /// I/O happens here; the first request is made lazily on the first call.
    pub fn new(cfg: Config, opts: Options) -> Self {
        let s3creds = AwsCredentials::new(
            cfg.creds.access_key_id,
            cfg.creds.secret_access_key,
            Some(cfg.creds.session_token),
            None,
            "miru-agent",
        );
        let s3cfg = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region))
            .credentials_provider(s3creds)
            .build();
        Self {
            client: Client::from_conf(s3cfg),
            bucket: cfg.bucket,
            opts,
        }
    }

    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) in place of the default HTTPS connector, so tests
    /// serve canned responses without touching the network. Credentials are
    /// static dummies since no real signing endpoint is contacted.
    #[cfg(feature = "test")]
    pub fn from_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
        cfg: Config,
        opts: Options,
    ) -> Self {
        let s3creds = AwsCredentials::new("test-access-key", "test-secret-key", None, None, "test");
        let s3cfg = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region))
            .credentials_provider(s3creds)
            // Path-style URLs (`/<bucket>/<key>`) make the replayed request URIs
            // deterministic and readable, so tests can assert on the exact path.
            .force_path_style(true)
            .http_client(http_client)
            .build();
        Self {
            client: Client::from_conf(s3cfg),
            bucket: cfg.bucket,
            opts,
        }
    }

    /// The size (in bytes) at or below which [`Self::put`] uses a single
    /// `PutObject` instead of a multipart upload.
    pub fn multipart_threshold(&self) -> u64 {
        self.opts.part_size
    }

    /// Creates or overwrites an object by streaming a file off disk.
    ///
    /// The whole file is never held in memory: files at or below the single-PUT
    /// threshold stream through one `PutObject`, and larger files stream part-by-part
    /// through a (stateless) multipart upload (see [`Self::put_multipart`]).
    pub async fn put(&self, key: &str, file: &File) -> Result<(), S3Err> {
        let size = file.size().await?;

        if size > self.opts.part_size {
            self.put_multipart(key, file, size).await
        } else {
            self.put_object(key, file).await
        }
    }

    /// Streams a file to S3 as a single-part upload.
    pub async fn put_object(&self, key: &str, file: &File) -> Result<(), S3Err> {
        let body = ByteStream::from_path(file.path())
            .await
            .map_err(|e| self.map_bytestream_err("put_object", key, file, &e))?;
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

    /// Streams a file to S3 as a **stateless** multipart upload, one part at a time.
    ///
    /// A fresh upload is created every call: create → `upload_part` over
    /// `part_size_for` chunks → complete. On any in-process failure the
    /// in-progress upload is aborted (best-effort) so S3 does not retain orphaned
    /// parts, then the error propagates. This carries **no durable state** and
    /// cannot resume across a crash; for resumable uploads use
    /// [`uploader::Uploader`].
    pub async fn put_multipart(&self, key: &str, file: &File, size: u64) -> Result<(), S3Err> {
        let part_size = Self::part_size_for(size);
        let upload_id = self.create_multipart_upload(key).await?;

        match self
            .upload_parts_and_complete(key, file, size, part_size, &upload_id)
            .await
        {
            Ok(()) => Ok(()),
            Err(err) => {
                // Best-effort cleanup: don't mask the original error if the abort
                // itself fails.
                let _ = self.abort_multipart_upload(key, &upload_id).await;
                Err(err)
            }
        }
    }

    /// Uploads every part of `file` (from byte 0) and completes the multipart
    /// upload. Split out from [`Self::put_multipart`] so a single `?` early-return
    /// path funnels through one abort site.
    async fn upload_parts_and_complete(
        &self,
        key: &str,
        file: &File,
        size: u64,
        part_size: u64,
        upload_id: &str,
    ) -> Result<(), S3Err> {
        let mut parts: Vec<(i32, String)> = Vec::new();
        let mut offset: u64 = 0;
        let mut part_number: i32 = 1; // S3 part numbers are 1-based.

        while offset < size {
            let len = part_size.min(size - offset);
            let etag = self
                .upload_part(key, upload_id, part_number, file, offset, len)
                .await?;
            parts.push((part_number, etag));
            offset += len;
            part_number += 1;
        }

        self.complete_multipart_upload(key, upload_id, &parts).await
    }

    /// Starts a multipart upload and returns its `upload_id`.
    pub async fn create_multipart_upload(&self, key: &str) -> Result<String, S3Err> {
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

    /// Streams a single part (`file[offset..offset+len]`) to S3 and returns its
    /// ETag. `InvalidResponseErr` if the response omits the ETag.
    pub async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        file: &File,
        offset: u64,
        len: u64,
    ) -> Result<String, S3Err> {
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

        part.e_tag().map(str::to_string).ok_or_else(|| {
            S3Err::InvalidResponseErr(InvalidResponseErr {
                operation: "upload_part".to_string(),
                msg: "response did not include an etag".to_string(),
                trace: trace!(),
            })
        })
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
        key: &str,
        upload_id: &str,
    ) -> Result<Option<Vec<PartInfo>>, S3Err> {
        let mut parts: Vec<PartInfo> = Vec::new();
        let mut marker: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_parts()
                .bucket(&self.bucket)
                .key(key)
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
                        Some(key.to_string()),
                        err,
                    ));
                }
            };

            for part in output.parts() {
                let (Some(part_number), Some(etag), Some(size)) =
                    (part.part_number(), part.e_tag(), part.size())
                else {
                    continue;
                };
                parts.push(PartInfo {
                    part_number,
                    etag: etag.to_string(),
                    size: size as u64,
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

    /// Completes a multipart upload from the `(part_number, etag)` pairs of the
    /// landed parts.
    pub async fn complete_multipart_upload(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(i32, String)],
    ) -> Result<(), S3Err> {
        let completed_parts: Vec<CompletedPart> = parts
            .iter()
            .map(|(part_number, etag)| {
                CompletedPart::builder()
                    .part_number(*part_number)
                    .e_tag(etag)
                    .build()
            })
            .collect();
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

    /// Aborts an in-progress multipart upload so S3 releases its parts. Returns a
    /// `Result` so callers decide whether to treat the abort as best-effort.
    pub async fn abort_multipart_upload(&self, key: &str, upload_id: &str) -> Result<(), S3Err> {
        self.client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err_common("abort_multipart_upload", Some(key.to_string()), e)
            })?;
        Ok(())
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

    /// Streams an object's body to a destination file. A missing object maps to
    /// [`S3Err::ObjectNotFoundErr`]. The body is copied through a bounded buffer rather
    /// than collected into memory.
    pub async fn get(&self, key: &str, dest: &File) -> Result<(), S3Err> {
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

    /// Deletes an object. Idempotent per S3 semantics (deleting a missing key still
    /// returns success).
    pub async fn delete(&self, key: &str) -> Result<(), S3Err> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("delete_object", Some(key.to_string()), e))?;
        Ok(())
    }

    /// Returns `true` if the object exists (HEAD 200), `false` on a 404. Other errors
    /// propagate.
    pub async fn exists(&self, key: &str) -> Result<bool, S3Err> {
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

    /// Maps a filesystem I/O error hit while streaming an object body (opening the
    /// source, creating the destination, or copying) into an `S3Err`.
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

    /// Maps a [`ByteStream`] construction error (e.g. the file could not be opened or
    /// read) into an `S3Err`.
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
