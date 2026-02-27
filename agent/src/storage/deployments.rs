// internal crates
use crate::cache;
use crate::models;

pub type DplEntry = cache::CacheEntry<models::DeploymentID, models::Deployment>;
pub type Deployments = cache::FileCache<models::DeploymentID, models::Deployment>;

pub fn is_dirty(old: Option<&DplEntry>, new: &models::Deployment) -> bool {
    let old = match old {
        Some(old) => old,
        None => return true,
    };
    old.is_dirty
        || old.value.activity_status != new.activity_status
        || old.value.error_status != new.error_status
}
