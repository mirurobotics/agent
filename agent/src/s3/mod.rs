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
//! ## Resumable multipart uploads
//!
//! Multipart uploads can survive a power-off and resume instead of restarting
//! from byte 0, but only when [`Config::upload_state_dir`] is `Some`. In that
//! mode each in-progress upload records a small durable `UploadState` JSON
//! (the `upload_id` plus guards) keyed by object key. S3 `ListParts` is the
//! source of truth for which parts already landed; the local state only carries
//! the `upload_id` and enough metadata to detect a changed source file. The
//! state file is written right after `create_multipart_upload` and deleted on a
//! successful `complete` **and** on `abort`, so a crash (which runs neither
//! delete) leaves the file behind for the next run to resume from. When
//! `upload_state_dir` is `None` the behavior is stateless: create a fresh upload
//! every time and abort on error.

// internal crates
use crate::filesys::dir::Dir;
use crate::filesys::file::{sanitize_filename, File};
use crate::filesys::path::PathExt;
use crate::filesys::WriteOptions;
use crate::trace;

// external crates
use aws_sdk_s3::config::{BehaviorVersion, Credentials as AwsCredentials, Region};
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::{ByteStream, ByteStreamError, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::Client;
use serde::{Deserialize, Serialize};

pub mod errors;

pub use errors::S3Err;
use errors::{InvalidResponseErr, ObjectNotFoundErr};

const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

// S3-defined part sized limits. These are hard limits which we cannot bypass.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB
const MAX_PARTS: u64 = 10_000; // 10,000 parts

/// Durable handle to an in-progress multipart upload, persisted so a reboot can
/// resume instead of restarting. S3 (ListParts) is the source of truth for which
/// parts landed; this only records the upload_id plus guards to detect a changed
/// source file.
#[derive(Debug, Serialize, Deserialize)]
struct UploadState {
    upload_id: String,
    key: String,
    size: u64,
    part_size: u64,
}

/// In-memory starting point for an upload derived from `ListParts`: which parts
/// already landed (a contiguous 1..N prefix), where to resume streaming from, and
/// the next part number to upload.
struct ResumeState {
    upload_id: String,
    completed_parts: Vec<CompletedPart>,
    offset: u64,
    next_part_number: i32,
}

pub struct Config {
    pub creds: Credentials,
    pub region: String,
    pub bucket: String,
    /// When `Some`, in-progress multipart uploads are persisted here (keyed by
    /// object key) so a reboot can resume them; when `None`, uploads are
    /// stateless (fresh every time, aborted on error).
    pub upload_state_dir: Option<Dir>,
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
    upload_state_dir: Option<Dir>,
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
            upload_state_dir: cfg.upload_state_dir,
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
            upload_state_dir: cfg.upload_state_dir,
        }
    }

    /// Creates or overwrites an object by streaming a file off disk.
    ///
    /// The whole file is never held in memory: files at or below the single-PUT
    /// threshold stream through one `PutObject`, and larger files stream part-by-part
    /// through a multipart upload (see `put_multipart`).
    pub async fn put(&self, key: &str, file: File) -> Result<(), S3Err> {
        let size = file.size().await?;

        if size > self.opts.part_size {
            self.put_multipart(key, &file, size).await
        } else {
            self.put_singlepart(key, &file).await
        }
    }

    /// Streams a file to S3 as a single-part upload.
    async fn put_singlepart(&self, key: &str, file: &File) -> Result<(), S3Err> {
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

    /// Streams a file to S3 as a multipart upload, one part at a time.
    ///
    /// When `upload_state_dir` is configured this is resumable: an existing state
    /// file for `key` is consulted (via [`Self::resume_or_restart`]) so parts that
    /// already landed on S3 are skipped instead of re-uploaded. Otherwise a fresh
    /// upload is created every time. On any in-process failure during the part loop
    /// or completion, the in-progress upload is aborted (best-effort) and the state
    /// file is deleted so S3 does not retain orphaned parts and the next run starts
    /// clean. A crash/power-off runs neither cleanup path, leaving the state file for
    /// resume.
    async fn put_multipart(&self, key: &str, file: &File, size: u64) -> Result<(), S3Err> {
        let part_size = Self::part_size_for(size);
        let state_file = self.upload_state_file(key);

        // Resolve a starting point: resume an in-progress upload if we have durable
        // state that S3 still recognizes, otherwise start fresh.
        let start = match &state_file {
            Some(f) => self.resume_or_restart(f, key, size, part_size).await?,
            None => None,
        };

        let (upload_id, completed_parts, offset, part_number) = match start {
            Some(rs) => (
                rs.upload_id,
                rs.completed_parts,
                rs.offset,
                rs.next_part_number,
            ),
            None => {
                let created = self
                    .client
                    .create_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .send()
                    .await
                    .map_err(|e| {
                        errors::map_sdk_err_common(
                            "create_multipart_upload",
                            Some(key.to_string()),
                            e,
                        )
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

                // Persist the durable handle immediately so a crash before completion
                // can resume this exact upload.
                if let Some(f) = &state_file {
                    f.write_json(
                        &UploadState {
                            upload_id: upload_id.clone(),
                            key: key.to_string(),
                            size,
                            part_size,
                        },
                        WriteOptions::OVERWRITE_ATOMIC,
                    )
                    .await?;
                }

                // S3 part numbers are 1-based.
                (upload_id, Vec::new(), 0, 1)
            }
        };

        match self
            .upload_parts_and_complete(
                key,
                file,
                size,
                part_size,
                &upload_id,
                completed_parts,
                offset,
                part_number,
            )
            .await
        {
            Ok(()) => {
                // Success: the upload is complete, so the durable handle is no longer
                // needed. Best-effort delete (a leftover file only wastes a resume
                // attempt on the next run).
                if let Some(f) = &state_file {
                    let _ = f.delete().await;
                }
                Ok(())
            }
            Err(err) => {
                // Best-effort cleanup: don't mask the original error if the
                // abort itself fails.
                let _ = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                if let Some(f) = &state_file {
                    let _ = f.delete().await;
                }
                Err(err)
            }
        }
    }

    /// The state file for `key`, or `None` when no `upload_state_dir` is configured.
    ///
    /// `sanitize_filename` can map distinct keys onto the same filename; the
    /// `UploadState.key` field is verified on load, so a collision is treated as
    /// "no resumable state" (fresh upload, overwriting the file).
    fn upload_state_file(&self, key: &str) -> Option<File> {
        self.upload_state_dir
            .as_ref()
            .map(|dir| dir.file(&format!("{}.json", sanitize_filename(key))))
    }

    /// Decides whether an in-progress multipart upload can be resumed from durable
    /// state, and if so where to resume from.
    ///
    /// Returns `Ok(None)` (caller starts fresh) when there is no state file, when
    /// the state is unreadable/corrupt, when it describes a different source file
    /// (key/size/part_size mismatch), or when S3 no longer recognizes the upload
    /// (`ListParts` 404 / NoSuchUpload). An incompatible state also triggers a
    /// best-effort abort of the stale upload. A bad local hint never hard-fails an
    /// upload; only genuine S3/network errors propagate.
    async fn resume_or_restart(
        &self,
        state_file: &File,
        key: &str,
        size: u64,
        part_size: u64,
    ) -> Result<Option<ResumeState>, S3Err> {
        if !state_file.exists() {
            return Ok(None);
        }

        // A corrupt/unreadable state file is just a bad local hint: drop it and
        // start fresh rather than failing the upload.
        let state: UploadState = match state_file.read_json().await {
            Ok(state) => state,
            Err(_) => {
                let _ = state_file.delete().await;
                return Ok(None);
            }
        };

        // A changed source file invalidates every recorded part (part N no longer
        // maps to the same byte range). Abort the stale upload and start fresh.
        if state.key != key || state.size != size || state.part_size != part_size {
            let _ = self
                .client
                .abort_multipart_upload()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(&state.upload_id)
                .send()
                .await;
            let _ = state_file.delete().await;
            return Ok(None);
        }

        let parts = match self.list_parts(key, &state.upload_id).await? {
            Some(parts) => parts,
            // The upload expired or was lifecycle-aborted: the id is dead, so drop
            // the state and start fresh.
            None => {
                let _ = state_file.delete().await;
                return Ok(None);
            }
        };

        // Build the contiguous 1..N prefix of fully-landed parts. Interior parts are
        // always exactly `part_size`; a gap, a missing e_tag, or a wrong size stops
        // the run so we never claim a part that isn't safely complete.
        let mut completed_parts: Vec<CompletedPart> = Vec::new();
        let mut next_part_number: i32 = 1;
        loop {
            let Some(part) = parts
                .iter()
                .find(|p| p.part_number() == Some(next_part_number))
            else {
                break;
            };
            let Some(e_tag) = part.e_tag() else { break };
            if part.size() != Some(part_size as i64) {
                break;
            }
            completed_parts.push(
                CompletedPart::builder()
                    .part_number(next_part_number)
                    .e_tag(e_tag)
                    .build(),
            );
            next_part_number += 1;
        }

        let landed = (next_part_number - 1) as u64;
        let offset = (landed * part_size).min(size);
        Ok(Some(ResumeState {
            upload_id: state.upload_id,
            completed_parts,
            offset,
            next_part_number,
        }))
    }

    /// Lists all parts already uploaded for `upload_id`, following pagination.
    ///
    /// Returns `Ok(None)` when S3 reports the upload no longer exists (a 404 /
    /// NoSuchUpload), mirroring the raw-response status check used by `get`/`exists`.
    /// Any other SDK error propagates via `map_sdk_err_common`.
    async fn list_parts(
        &self,
        key: &str,
        upload_id: &str,
    ) -> Result<Option<Vec<aws_sdk_s3::types::Part>>, S3Err> {
        let mut parts: Vec<aws_sdk_s3::types::Part> = Vec::new();
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

            parts.extend(output.parts().iter().cloned());

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

    /// Uploads the remaining parts of `file` (starting from `offset` /
    /// `part_number`, with `completed_parts` already landed) and completes the
    /// multipart upload. Split out from [`Self::put_multipart`] so a single `?`
    /// early-return path funnels through one abort site.
    #[allow(clippy::too_many_arguments)]
    async fn upload_parts_and_complete(
        &self,
        key: &str,
        file: &File,
        size: u64,
        part_size: u64,
        upload_id: &str,
        mut completed_parts: Vec<CompletedPart>,
        mut offset: u64,
        mut part_number: i32,
    ) -> Result<(), S3Err> {
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

    /// Picks a part size that keeps the part count within S3's 10,000-part limit. Uses
    /// the fixed [`PART_SIZE`] until a file is large enough to need more than 10,000
    /// such parts, then grows the part size to `ceil(size / 10_000)` (never below the 5
    /// MiB floor).
    fn part_size_for(size: u64) -> u64 {
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
