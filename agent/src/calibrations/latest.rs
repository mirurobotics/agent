// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::disk::LatestCalStor;
use crate::filesys::Overwrite;
use crate::trace;

// external crates
use tracing::warn;
use uuid::Uuid;

/// Which entries to list. Empty means any; `None` in `role`/`aspect` matches an
/// entry without one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub component: Vec<String>,
    pub role: Vec<Option<String>>,
    pub aspect: Vec<Option<String>>,
}

impl Filter {
    pub fn is_match(&self, metadata: &ReportMetadata) -> bool {
        is_allowed(&self.component, &metadata.component)
            && is_allowed(&self.role, &metadata.role)
            && is_allowed(&self.aspect, &metadata.aspect)
    }
}

/// The newest calibration for each component, role and aspect, kept in the
/// calibration caches: one entry per key in the index cache, and each
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

    /// Make `metadata` its key's entry, with `data` as its content. A new key
    /// past the capacity is rejected rather than evicting another. A failure
    /// part way leaves at most content that no entry names, which the startup
    /// prune deletes.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        let key = key(&metadata);
        let index = &self.stor.index;
        let capacity = index.capacity().await?;
        let mut entries = index.value_map().await?;
        let replaced = entries.remove(&key);
        if replaced.is_none() {
            check_capacity(entries.len(), capacity)?;
        }
        let report_id = metadata.report_id.to_string();
        self.stor
            .content
            .write(report_id, data.to_string(), |_, _| false, Overwrite::Allow)
            .await?;
        index
            .write(key, metadata, |_, _| false, Overwrite::Allow)
            .await?;
        if let Some(replaced) = replaced {
            self.delete_content(replaced.report_id.to_string()).await;
        }
        Ok(())
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr> {
        // not find_where: it bumps each match's last access, rewriting the
        // index once per match
        let mut entries = self.stor.index.values().await?;
        entries.retain(|metadata| filter.is_match(metadata));
        entries.sort_by(|a, b| key_parts(a).cmp(&key_parts(b)));
        Ok(entries)
    }

    /// The data of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn content(&self, report_id: Uuid) -> Result<String, CalibrationsErr> {
        let entries = self.stor.index.values().await?;
        if !entries.iter().any(|m| m.report_id == report_id) {
            return Err(not_found(report_id));
        }
        let content = self.stor.content.read_optional(report_id.to_string());
        content.await?.ok_or_else(|| not_found(report_id))
    }

    /// Delete the content under `content_key`, logging a failure.
    async fn delete_content(&self, content_key: String) {
        if let Err(e) = self.stor.content.delete(content_key.clone()).await {
            warn!("calibrations: failed to delete latest content {content_key}: {e}");
        }
    }
}

/// The index cache's key for `metadata`: its component, role and aspect as
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

/// Reject a new key once the store holds `capacity` keys.
fn check_capacity(size: usize, capacity: usize) -> Result<(), CalibrationsErr> {
    if size < capacity {
        return Ok(());
    }
    let err = CalibrationStoreFullErr {
        capacity,
        trace: trace!(),
    };
    warn!("calibrations: rejecting report: {err}");
    Err(err.into())
}

fn not_found(report_id: Uuid) -> CalibrationsErr {
    CalibrationsErr::NotFoundErr(CalibrationNotFoundErr {
        report_id: report_id.to_string(),
        trace: trace!(),
    })
}
