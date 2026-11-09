// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::disk::LatestCalStor;
use crate::filesys::Overwrite;
use crate::models::InstanceFormat;
use crate::trace;

// external crates
use tracing::warn;
use uuid::Uuid;

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
    pub entries: Vec<ReportMetadata>,
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
#[derive(Clone, Debug)]
pub struct LatestStore {
    stor: LatestCalStor,
}

impl LatestStore {
    pub fn new(stor: LatestCalStor) -> Self {
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

    /// Make `metadata` its key's entry, with `data` as its content. The content
    /// is written before the entry and the replaced entry's content deleted
    /// after it, so a failure leaves at most content that no entry names. On
    /// error the key's entry is unchanged.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        let key = key(&metadata);
        let replaced = self.stor.meta.entry_map().await?.remove(&key);
        let content_key = metadata.report_id.to_string();
        let content = data.to_string();
        self.stor
            .content
            .write(content_key.clone(), content, |_, _| false, Overwrite::Allow)
            .await?;
        let meta = &self.stor.meta;
        if let Err(e) = meta
            .write(key, metadata, |_, _| false, Overwrite::Allow)
            .await
        {
            self.delete_content(content_key).await;
            return Err(e.into());
        }
        if let Some(replaced) = replaced {
            let replaced_id = replaced.value.report_id;
            self.delete_content(replaced_id.to_string()).await;
        }
        Ok(())
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub async fn list(&self, filter: &LatestFilter) -> Result<LatestCalibrations, CalibrationsErr> {
        let mut entries = self.stor.meta.values().await?;
        entries.retain(|metadata| filter.is_match(metadata));
        entries.sort_by(|a, b| key_parts(a).cmp(&key_parts(b)));
        Ok(LatestCalibrations {
            // nothing loads the store from Miru yet
            is_confirmed: false,
            entries,
        })
    }

    /// The content of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr> {
        let entries = self.stor.meta.values().await?;
        let mut entries = entries.into_iter();
        let Some(metadata) = entries.find(|m| m.report_id == report_id) else {
            return Err(not_found(report_id));
        };
        let content = self.stor.content.read_optional(report_id.to_string());
        let Some(data) = content.await? else {
            return Err(not_found(report_id));
        };
        Ok(LatestContent {
            format: metadata.format,
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
