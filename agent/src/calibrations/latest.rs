// standard crates
use std::collections::HashSet;

// internal crates
use crate::cache::CacheErr;
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::disk::CalibrationStor;
use crate::filesys::Overwrite;
use crate::models::{CalibrationUploadStatus, InstanceFormat};
use crate::trace;

// external crates
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

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

/// The newest calibration for each component, role and aspect, kept in the
/// calibration caches: one entry per key in the metadata cache, and each
/// entry's data in the content cache under its `report_id`. A report replaces
/// its key's entry, and a new key past the caches' capacity is rejected rather
/// than evicting another.
pub struct LatestStore {
    stor: CalibrationStor,
}

impl LatestStore {
    pub fn new(stor: CalibrationStor) -> Self {
        Self { stor }
    }

    /// Ok when the key `metadata` reports already has an entry or there is
    /// room for a new key.
    pub async fn verify_capacity(&self, metadata: &ReportMetadata) -> Result<(), CalibrationsErr> {
        let entries = self.stor.meta.entry_map().await?;
        let capacity = self.stor.meta.capacity();
        if entries.len() < capacity || entries.contains_key(&key(metadata)) {
            return Ok(());
        }
        let err = CalibrationStoreFullErr {
            capacity,
            trace: trace!(),
        };
        warn!("calibrations: rejecting report: {err}");
        Err(err.into())
    }

    /// Make `metadata` its key's entry, not uploaded, with `data` as its
    /// content. The content is written before the entry and the replaced
    /// entry's content deleted after it, so a failure leaves at most content
    /// that no entry names. On error the key's entry is unchanged.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        let key = key(&metadata);
        let replaced = self.stor.meta.entry_map().await?.remove(&key);
        let content_key = metadata.report_id.to_string();
        let content = data.to_string();
        self.stor
            .content
            .write(content_key.clone(), content, |_, _| false, Overwrite::Allow)
            .await?;
        let entry = LatestEntry {
            metadata,
            is_uploaded: false,
        };
        let meta = &self.stor.meta;
        if let Err(e) = meta.write(key, entry, |_, _| false, Overwrite::Allow).await {
            self.delete_content(content_key).await;
            return Err(e.into());
        }
        if let Some(replaced) = replaced {
            let replaced_id = replaced.value.metadata.report_id;
            self.delete_content(replaced_id.to_string()).await;
        }
        Ok(())
    }

    /// Mark the entry for `report_id` uploaded. A newer report may have
    /// replaced it; then this does nothing. A failure is logged, and the entry
    /// then lists as dropped.
    pub async fn mark_uploaded(&self, report_id: Uuid) {
        if let Err(e) = self.try_mark_uploaded(report_id).await {
            warn!("calibrations: failed to mark report {report_id} uploaded: {e}");
        }
    }

    async fn try_mark_uploaded(&self, report_id: Uuid) -> Result<(), CacheErr> {
        let entries = self.stor.meta.value_map().await?;
        let mut entries = entries.into_iter();
        let Some((key, mut entry)) = entries.find(|(_, e)| e.metadata.report_id == report_id)
        else {
            return Ok(());
        };
        entry.is_uploaded = true;
        let meta = &self.stor.meta;
        meta.write(key, entry, |_, _| false, Overwrite::Allow).await
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    /// `queued` holds the `report_id` of every report still in the upload
    /// queue.
    pub async fn list(
        &self,
        filter: &LatestFilter,
        queued: &HashSet<Uuid>,
    ) -> Result<LatestCalibrations, CalibrationsErr> {
        let mut entries = self.stor.meta.values().await?;
        entries.retain(|e| filter.is_match(&e.metadata));
        entries.sort_by(|a, b| key_parts(&a.metadata).cmp(&key_parts(&b.metadata)));
        let listed = entries.into_iter().map(|e| ListedEntry {
            upload_status: e.upload_status(queued),
            metadata: e.metadata,
        });
        Ok(LatestCalibrations {
            // nothing loads the store from Miru yet
            is_confirmed: false,
            entries: listed.collect(),
        })
    }

    /// The content of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr> {
        let entries = self.stor.meta.values().await?;
        let mut entries = entries.into_iter();
        let Some(entry) = entries.find(|e| e.metadata.report_id == report_id) else {
            return Err(not_found(report_id));
        };
        let content = self.stor.content.read_optional(report_id.to_string());
        let Some(data) = content.await? else {
            return Err(not_found(report_id));
        };
        Ok(LatestContent {
            format: entry.metadata.format,
            data: data.into_bytes(),
        })
    }

    /// Delete the content under `content_key`, logging a failure.
    async fn delete_content(&self, content_key: String) {
        if let Err(e) = self.stor.content.delete(content_key.clone()).await {
            warn!("calibrations: failed to delete latest content {content_key}: {e}");
        }
    }
}

/// The metadata cache's key for `metadata`: its component, role and aspect as
/// a JSON array, so no two keys collide.
fn key(metadata: &ReportMetadata) -> String {
    serde_json::json!(key_parts(metadata)).to_string()
}

fn key_parts(metadata: &ReportMetadata) -> (&String, &Option<String>, &Option<String>) {
    (&metadata.component, &metadata.role, &metadata.aspect)
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
