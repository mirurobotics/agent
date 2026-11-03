// standard crates
use std::collections::HashSet;

// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::filesys::{
    dirs, files, state_file::StateFile, Dir, File, FileSysErr, PathExt, WriteOptions,
};
use crate::models::{CalibrationUploadStatus, InstanceFormat, Patch};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

/// The newest report for one key (component, role and aspect) and whether it
/// reached Miru.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatestEntry {
    pub metadata: ReportMetadata,
    pub upload_status: CalibrationUploadStatus,
}

/// The latest store's index: one entry per key, in the order keys arrived.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatestIndex {
    /// When the store was last loaded from Miru; `None` until then.
    #[serde(default)]
    pub loaded_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub entries: Vec<LatestEntry>,
}

impl Patch<LatestIndex> for LatestIndex {
    fn patch(&mut self, patch: LatestIndex) {
        *self = patch;
    }
}

pub type LatestIndexFile = StateFile<LatestIndex, LatestIndex>;

/// Which entries to list. Empty means any; `None` in `role`/`aspect` matches an
/// entry without one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LatestFilter {
    pub component: Vec<String>,
    pub role: Vec<Option<String>>,
    pub aspect: Vec<Option<String>>,
}

impl LatestFilter {
    pub fn is_match(&self, metadata: &ReportMetadata) -> bool {
        is_allowed(&self.component, &metadata.component)
            && is_allowed(&self.role, &metadata.role)
            && is_allowed(&self.aspect, &metadata.aspect)
    }
}

/// The entries a filter matched, and whether the store was loaded from Miru.
#[derive(Debug, PartialEq)]
pub struct LatestCalibrations {
    pub is_confirmed: bool,
    pub entries: Vec<LatestEntry>,
}

/// A latest entry's content, as it was reported.
#[derive(Debug, PartialEq)]
pub struct LatestContent {
    pub format: InstanceFormat,
    pub data: Vec<u8>,
}

/// The newest calibration for each key: an index plus one content file per
/// entry, `<contents>/<report_id>`. It never evicts; a report replaces its
/// key's entry, and a new key past the capacity is rejected.
pub struct LatestStore {
    index: LatestIndex,
    index_file: Option<LatestIndexFile>,
    contents: Dir,
    capacity: usize,
}

impl LatestStore {
    /// Without an `index_file` the index lives in memory, so it starts empty at
    /// the next restart.
    pub fn new(index_file: Option<LatestIndexFile>, contents: Dir, capacity: usize) -> Self {
        let index = match &index_file {
            Some(file) => (*file.read()).clone(),
            None => LatestIndex::default(),
        };
        Self {
            index,
            index_file,
            contents,
            capacity,
        }
    }

    /// Ok when the key `metadata` reports already has an entry or there is
    /// room for a new key.
    pub fn verify_capacity(
        &self,
        metadata: &ReportMetadata,
    ) -> Result<(), CalibrationStoreFullErr> {
        let entries = &self.index.entries;
        if entries.len() < self.capacity || entries.iter().any(|e| has_key(e, metadata)) {
            return Ok(());
        }
        let err = CalibrationStoreFullErr {
            capacity: self.capacity,
            trace: trace!(),
        };
        warn!("calibrations: rejecting report: {err}");
        Err(err)
    }

    /// Make `metadata` its key's entry, `queued`, with `data` as its content.
    /// The content file is written before the index and the replaced entry's
    /// content deleted after it, so a crash leaves at most a file no entry
    /// names, which [`Self::prune_orphans`] deletes. On error nothing changed.
    pub async fn put(
        &mut self,
        metadata: ReportMetadata,
        data: &[u8],
    ) -> Result<(), CalibrationsErr> {
        self.verify_capacity(&metadata)?;
        let report_id = metadata.report_id;
        let file = self.content_file(report_id);
        files::write_bytes(&file, data, WriteOptions::OVERWRITE_ATOMIC).await?;
        let mut index = self.index.clone();
        let entry = LatestEntry {
            metadata,
            upload_status: CalibrationUploadStatus::Queued,
        };
        let replaced = replace_or_append(&mut index.entries, entry);
        if let Err(e) = self.persist(index.clone()).await {
            delete(&file).await;
            return Err(e.into());
        }
        self.index = index;
        let replaced_id = replaced.map(|entry| entry.metadata.report_id);
        if let Some(replaced_id) = replaced_id.filter(|id| *id != report_id) {
            delete(&self.content_file(replaced_id)).await;
        }
        Ok(())
    }

    /// Set the upload status of the entry for `report_id`. A newer report may
    /// have replaced it, or `reset` wiped the store; then this does nothing. A
    /// failed index write is logged, and the next write saves the status.
    pub async fn set_upload_status(&mut self, report_id: Uuid, status: CalibrationUploadStatus) {
        let entries = &mut self.index.entries;
        let Some(entry) = entries
            .iter_mut()
            .find(|e| e.metadata.report_id == report_id)
        else {
            return;
        };
        entry.upload_status = status;
        let index = self.index.clone();
        if let Err(e) = self.persist(index).await {
            warn!("calibrations: failed to save the upload status of report {report_id}: {e}");
        }
    }

    /// The entries `filter` matches, in index order.
    pub fn list(&self, filter: &LatestFilter) -> LatestCalibrations {
        let entries = self.index.entries.iter();
        let matched = entries.filter(|e| filter.is_match(&e.metadata));
        LatestCalibrations {
            is_confirmed: self.index.loaded_at.is_some(),
            entries: matched.cloned().collect(),
        }
    }

    /// The content of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content file exists.
    pub async fn content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr> {
        let entries = &self.index.entries;
        let Some(entry) = entries.iter().find(|e| e.metadata.report_id == report_id) else {
            return Err(not_found(report_id));
        };
        match files::read_bytes(&self.content_file(report_id)).await {
            Ok(data) => Ok(LatestContent {
                format: entry.metadata.format,
                data,
            }),
            Err(FileSysErr::PathDoesNotExistErr(_)) => Err(not_found(report_id)),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete content files no entry names, left by a crash during
    /// [`Self::put`]. Failures are logged and retried on the next boot. An
    /// in-memory store prunes nothing: the index file it couldn't open may
    /// still name these files.
    pub async fn prune_orphans(&self) {
        if self.index_file.is_none() || !self.contents.exists() {
            return;
        }
        let listing = match dirs::files(&self.contents).await {
            Ok(listing) => listing,
            Err(e) => {
                warn!("calibrations: failed to list latest content files: {e}");
                return;
            }
        };
        let entries = self.index.entries.iter();
        let named: HashSet<String> = entries.map(|e| e.metadata.report_id.to_string()).collect();
        for file in listing {
            if file.name().is_ok_and(|name| named.contains(name)) {
                continue;
            }
            warn!("calibrations: deleting latest content file {file}, which no entry names");
            delete(&file).await;
        }
    }

    /// Write `index` to the index file, if there is one.
    async fn persist(&mut self, index: LatestIndex) -> Result<(), FileSysErr> {
        match &mut self.index_file {
            Some(file) => file.write(index).await,
            None => Ok(()),
        }
    }

    fn content_file(&self, report_id: Uuid) -> File {
        self.contents.file(&report_id.to_string())
    }
}

/// Whether `value` is in `allowed`, where an empty list allows any value.
fn is_allowed<T: PartialEq>(allowed: &[T], value: &T) -> bool {
    allowed.is_empty() || allowed.contains(value)
}

/// Whether `entry` is for the key `metadata` reports.
fn has_key(entry: &LatestEntry, metadata: &ReportMetadata) -> bool {
    let key = &entry.metadata;
    key.component == metadata.component
        && key.role == metadata.role
        && key.aspect == metadata.aspect
}

/// Put `entry` in its key's place, or last for a new key, and return the entry
/// it replaced.
fn replace_or_append(entries: &mut Vec<LatestEntry>, entry: LatestEntry) -> Option<LatestEntry> {
    match entries.iter_mut().find(|e| has_key(e, &entry.metadata)) {
        Some(slot) => Some(std::mem::replace(slot, entry)),
        None => {
            entries.push(entry);
            None
        }
    }
}

fn not_found(report_id: Uuid) -> CalibrationsErr {
    CalibrationsErr::NotFoundErr(CalibrationNotFoundErr {
        report_id: report_id.to_string(),
        trace: trace!(),
    })
}

/// Delete `file`, logging a failure. A file that is already gone is fine.
async fn delete(file: &File) {
    if let Err(e) = files::delete(file).await {
        warn!("calibrations: failed to delete latest content file {file}: {e}");
    }
}
