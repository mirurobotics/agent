use crate::cache::file::FileCache;
use crate::models::git_commit::{GitCommit, GitCommitID};

pub type GitCommits = FileCache<GitCommitID, GitCommit>;
