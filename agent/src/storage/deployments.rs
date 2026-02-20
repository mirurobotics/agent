// internal crates
use crate::cache::{entry::CacheEntry, file::FileCache};
use crate::models::deployment::{Deployment, DeploymentID};

pub type DplEntry = CacheEntry<DeploymentID, Deployment>;
pub type Deployments = FileCache<DeploymentID, Deployment>;

pub fn is_dirty(old: Option<&DplEntry>, new: &Deployment) -> bool {
    let old = match old {
        Some(old) => old,
        None => return true,
    };
    old.is_dirty
        || old.value.activity_status != new.activity_status
        || old.value.error_status != new.error_status
}
