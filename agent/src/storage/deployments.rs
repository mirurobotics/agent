// internal crates
use crate::cache::{entry::CacheEntry, file::FileCache};
use crate::models::deployment::{Deployment, DeploymentID};

pub type DeploymentCacheEntry = CacheEntry<DeploymentID, Deployment>;

// the deployment cache is stored in a single file
pub type DeploymentCache = FileCache<DeploymentID, Deployment>;
