use crate::cache;
use crate::models;

pub type GitCommits = cache::FileCache<models::GitCommitID, models::GitCommit>;
