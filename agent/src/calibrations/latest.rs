// standard crates
use std::collections::{BTreeMap, HashSet};

// internal crates
use crate::calibrations::{
    cleanup,
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::filesys::{files, state_file::StateFile, Dir, File, FileSysErr, WriteOptions};
use crate::models::{CalibrationUploadStatus, InstanceFormat, Patch};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

/// The most keys a [`LatestStore`] holds unless told otherwise.
pub const DEFAULT_CAPACITY: usize = 1_000;

/// What a latest entry is the newest report for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LatestKey {
    pub component: String,
    pub role: Option<String>,
    pub aspect: Option<String>,
}

impl From<&ReportMetadata> for LatestKey {
    fn from(metadata: &ReportMetadata) -> Self {
        Self {
            component: metadata.component.clone(),
            role: metadata.role.clone(),
            aspect: metadata.aspect.clone(),
        }
    }
}

/// The newest report for one key and whether it reached Miru.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatestEntry {
    pub metadata: ReportMetadata,
    pub is_uploaded: bool,
}

impl LatestEntry {
    /// Uploaded once marked, queued while the upload queue holds the report,
    /// and dropped once the queue let it go without uploading it.
    fn upload_status(&self, queued: &HashSet<Uuid>) -> CalibrationUploadStatus {
        if self.is_uploaded {
            CalibrationUploadStatus::Uploaded
        } else if queued.contains(&self.metadata.report_id) {
            CalibrationUploadStatus::Queued
        } else {
            CalibrationUploadStatus::Dropped
        }
    }
}

/// The latest store's index: one entry per key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatestIndex {
    /// When the store was last loaded from Miru; `None` until then.
    #[serde(default)]
    pub loaded_at: Option<DateTime<Utc>>,
    #[serde(with = "entry_list")]
    pub entries: BTreeMap<LatestKey, LatestEntry>,
}

/// Saves the index's entries as a list, since a JSON object's keys are
/// strings, and keys each one by its metadata on load.
mod entry_list {
    // standard crates
    use std::collections::BTreeMap;

    // internal crates
    use super::{LatestEntry, LatestKey};

    // external crates
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        entries: &BTreeMap<LatestKey, LatestEntry>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(entries.values())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<LatestKey, LatestEntry>, D::Error> {
        let entries = Vec::<LatestEntry>::deserialize(deserializer)?;
        let keyed = entries
            .into_iter()
            .map(|e| (LatestKey::from(&e.metadata), e));
        Ok(keyed.collect())
    }
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
    pub entries: Vec<ListedEntry>,
}

/// A latest entry as listed, with its upload status.
#[derive(Debug, PartialEq)]
pub struct ListedEntry {
    pub metadata: ReportMetadata,
    pub upload_status: CalibrationUploadStatus,
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
        if entries.len() < self.capacity || entries.contains_key(&LatestKey::from(metadata)) {
            return Ok(());
        }
        let err = CalibrationStoreFullErr {
            capacity: self.capacity,
            trace: trace!(),
        };
        warn!("calibrations: rejecting report: {err}");
        Err(err)
    }

    /// Make `metadata` its key's entry, not uploaded, with `data` as its content.
    /// The content file is written before the index and the replaced entry's
    /// content deleted after it, so a crash leaves at most a file no entry
    /// names, which [`Self::prune_orphans`] deletes. On error nothing changed.
    pub async fn put(
        &mut self,
        metadata: ReportMetadata,
        data: &[u8],
    ) -> Result<(), CalibrationsErr> {
        self.verify_capacity(&metadata)?;
        let file = self.content_file(metadata.report_id);
        files::write_bytes(&file, data, WriteOptions::OVERWRITE_ATOMIC).await?;
        let mut index = self.index.clone();
        let entry = LatestEntry {
            metadata,
            is_uploaded: false,
        };
        let replaced = index
            .entries
            .insert(LatestKey::from(&entry.metadata), entry);
        if let Err(e) = self.persist(index.clone()).await {
            cleanup::delete(&file).await;
            return Err(e.into());
        }
        self.index = index;
        if let Some(replaced) = replaced {
            cleanup::delete(&self.content_file(replaced.metadata.report_id)).await;
        }
        Ok(())
    }

    /// Mark the entry for `report_id` uploaded. A newer report may have
    /// replaced it, or `reset` wiped the store; then this does nothing. A
    /// failed index write is logged; a restart before the next write loses
    /// the mark, and the entry is then listed as dropped.
    pub async fn mark_uploaded(&mut self, report_id: Uuid) {
        let mut entries = self.index.entries.values_mut();
        let Some(entry) = entries.find(|e| e.metadata.report_id == report_id) else {
            return;
        };
        entry.is_uploaded = true;
        let index = self.index.clone();
        if let Err(e) = self.persist(index).await {
            warn!("calibrations: failed to mark report {report_id} uploaded: {e}");
        }
    }

    /// The entries `filter` matches, in key order. `queued` holds the
    /// `report_id` of every report still in the upload queue.
    pub fn list(&self, filter: &LatestFilter, queued: &HashSet<Uuid>) -> LatestCalibrations {
        let entries = self.index.entries.values();
        let matched = entries.filter(|e| filter.is_match(&e.metadata));
        let listed = matched.map(|e| ListedEntry {
            metadata: e.metadata.clone(),
            upload_status: e.upload_status(queued),
        });
        LatestCalibrations {
            is_confirmed: self.index.loaded_at.is_some(),
            entries: listed.collect(),
        }
    }

    /// The content of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content file exists.
    pub async fn content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr> {
        let mut entries = self.index.entries.values();
        let Some(entry) = entries.find(|e| e.metadata.report_id == report_id) else {
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
        if self.index_file.is_none() {
            return;
        }
        let entries = self.index.entries.values();
        let named = entries.map(|e| e.metadata.report_id.to_string());
        cleanup::prune(&self.contents, &named.collect()).await;
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

fn not_found(report_id: Uuid) -> CalibrationsErr {
    CalibrationsErr::NotFoundErr(CalibrationNotFoundErr {
        report_id: report_id.to_string(),
        trace: trace!(),
    })
}
