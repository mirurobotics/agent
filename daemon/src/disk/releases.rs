// internal crates
use crate::cache;
use crate::models;

pub type Releases = cache::FileCache<models::ReleaseID, models::Release>;
