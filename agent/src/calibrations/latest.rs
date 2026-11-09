// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::disk::{latest_cals::content_key, LatestCalStor};
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
/// entry's data in the content cache under a hash of its key. A report replaces
/// its key's entry and content, and a new key past the caches' capacity is
/// rejected rather than evicting another.
#[derive(Clone, Debug)]
pub struct LatestStore {
    stor: LatestCalStor,
}

impl LatestStore {
    pub fn new(stor: LatestCalStor) -> Self {
        Self { stor }
    }

    /// Make `metadata` its key's entry, with `data` as its content. A new key
    /// past the capacity is rejected rather than evicting another. The content
    /// is written first, so a failed entry write leaves the old entry with the
    /// new content, or a new key's content with no entry, which the startup
    /// prune deletes.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        let key = key(&metadata);
        let index = &self.stor.index;
        let capacity = index.capacity().await?;
        let entries = index.value_map().await?;
        if !entries.contains_key(&key) {
            check_capacity(entries.len(), capacity)?;
        }
        self.stor
            .content
            .write(content_key(&key), data.to_string(), |_, _| false, Overwrite::Allow)
            .await?;
        index
            .write(key, metadata, |_, _| false, Overwrite::Allow)
            .await?;
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
        let Some(entry) = entries.iter().find(|m| m.report_id == report_id) else {
            return Err(not_found(report_id));
        };
        let content = self.stor.content.read_optional(content_key(&key(entry)));
        content.await?.ok_or_else(|| not_found(report_id))
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
