//! Resumable multipart uploads layered over a thin [`Store`].
//!
//! Multipart uploads can survive a power-off and resume instead of restarting
//! from byte 0. An [`Uploader`] records a small durable `UploadState` JSON
//! (the `upload_id` plus guards) keyed by object key in its `state_dir`. S3
//! `ListParts` is the source of truth for which parts already landed; the local
//! state only carries the `upload_id` and enough metadata to detect a changed
//! source file. The state file is written right after `create_multipart_upload`
//! and deleted on a successful `complete` **and** on an in-process error's
//! abort, so a crash (which runs neither delete) leaves the file behind for the
//! next run to resume from.

// internal crates
use crate::filesys::dir::Dir;
use crate::filesys::file::{sanitize_filename, File};
use crate::filesys::path::PathExt;
use crate::filesys::WriteOptions;

// external crates
use serde::{Deserialize, Serialize};

use super::{S3Err, Store};

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
    completed_parts: Vec<(i32, String)>,
    offset: u64,
    next_part_number: i32,
}

/// Resumable multipart uploads layered over a thin [`Store`]. Owns the durable
/// upload-id state directory and the resume-vs-restart policy.
pub struct Uploader {
    store: Store,
    // Required — resumability is the whole point of this type.
    state_dir: Dir,
}

impl Uploader {
    pub fn new(store: Store, state_dir: Dir) -> Self {
        Self { store, state_dir }
    }

    /// The underlying thin client, for callers needing `get`/`delete`/`exists`.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Uploads `file` to `key`, resuming an interrupted multipart upload if durable
    /// state exists.
    ///
    /// Small files (at or below [`Store::multipart_threshold`]) stream straight
    /// through `put_object` with no durable state. Larger files use a resumable
    /// multipart upload: an existing state file for `key` is consulted (via
    /// `resume_or_restart`) so parts that already landed on S3 are skipped
    /// instead of re-uploaded, otherwise a fresh upload is created. On any
    /// in-process failure during the part loop or completion, the in-progress
    /// upload is aborted (best-effort) and the state file is deleted so S3 does not
    /// retain orphaned parts and the next run starts clean. A crash/power-off runs
    /// neither cleanup path, leaving the state file for resume.
    pub async fn upload(&self, key: &str, file: &File) -> Result<(), S3Err> {
        let size = file.size().await?;

        if size <= self.store.multipart_threshold() {
            return self.store.put_object(key, file).await;
        }

        let part_size = Store::part_size_for(size);
        let state_file = self
            .state_dir
            .file(&format!("{}.json", sanitize_filename(key)));

        // Resolve a starting point: resume an in-progress upload if we have durable
        // state that S3 still recognizes, otherwise start fresh.
        let start = self
            .resume_or_restart(&state_file, key, size, part_size)
            .await?;

        let (upload_id, completed_parts, offset, part_number) = match start {
            Some(rs) => (
                rs.upload_id,
                rs.completed_parts,
                rs.offset,
                rs.next_part_number,
            ),
            None => {
                let upload_id = self.store.create_multipart_upload(key).await?;

                // Persist the durable handle immediately so a crash before completion
                // can resume this exact upload.
                state_file
                    .write_json(
                        &UploadState {
                            upload_id: upload_id.clone(),
                            key: key.to_string(),
                            size,
                            part_size,
                        },
                        WriteOptions::OVERWRITE_ATOMIC,
                    )
                    .await?;

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
                let _ = state_file.delete().await;
                Ok(())
            }
            Err(err) => {
                // Best-effort cleanup: don't mask the original error if the abort
                // itself fails.
                let _ = self.store.abort_multipart_upload(key, &upload_id).await;
                let _ = state_file.delete().await;
                Err(err)
            }
        }
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
                .store
                .abort_multipart_upload(key, &state.upload_id)
                .await;
            let _ = state_file.delete().await;
            return Ok(None);
        }

        let parts = match self.store.list_parts(key, &state.upload_id).await? {
            Some(parts) => parts,
            // The upload expired or was lifecycle-aborted: the id is dead, so drop
            // the state and start fresh.
            None => {
                let _ = state_file.delete().await;
                return Ok(None);
            }
        };

        // Build the contiguous 1..N prefix of fully-landed parts. Interior parts are
        // always exactly `part_size`; a gap or a wrong size stops the run so we never
        // claim a part that isn't safely complete.
        let mut completed_parts: Vec<(i32, String)> = Vec::new();
        let mut next_part_number: i32 = 1;
        loop {
            let Some(part) = parts.iter().find(|p| p.part_number == next_part_number) else {
                break;
            };
            if part.size != part_size {
                break;
            }
            completed_parts.push((next_part_number, part.etag.clone()));
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

    /// Uploads the remaining parts of `file` (starting from `offset` /
    /// `part_number`, with `completed_parts` already landed) and completes the
    /// multipart upload. Split out from [`Self::upload`] so a single `?`
    /// early-return path funnels through one abort site.
    #[allow(clippy::too_many_arguments)]
    async fn upload_parts_and_complete(
        &self,
        key: &str,
        file: &File,
        size: u64,
        part_size: u64,
        upload_id: &str,
        mut completed_parts: Vec<(i32, String)>,
        mut offset: u64,
        mut part_number: i32,
    ) -> Result<(), S3Err> {
        while offset < size {
            let len = part_size.min(size - offset);
            let etag = self
                .store
                .upload_part(key, upload_id, part_number, file, offset, len)
                .await?;
            completed_parts.push((part_number, etag));
            offset += len;
            part_number += 1;
        }

        self.store
            .complete_multipart_upload(key, upload_id, &completed_parts)
            .await
    }
}
