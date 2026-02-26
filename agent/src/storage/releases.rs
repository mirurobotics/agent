use crate::cache::file::FileCache;
use crate::models::release::{Release, ReleaseID};

pub type Releases = FileCache<ReleaseID, Release>;
