// internal crates
use crate::disk;
use crate::models;
use crate::services::{backend::BackendFetcher, errors::ServiceErr};

// external crates
use tracing::error;

pub async fn get<B: BackendFetcher>(
    git_commits: &disk::GitCommits,
    backend: &B,
    id: String,
) -> Result<models::GitCommit, ServiceErr> {
    if let Some(git_commit) = git_commits.read_optional(id.clone()).await? {
        return Ok(git_commit);
    }
    let git_commit = models::GitCommit::from(backend.fetch_git_commit(&id).await?);
    if let Err(e) = git_commits
        .write_if_absent(git_commit.id.clone(), git_commit.clone(), |_, _| false)
        .await
    {
        error!("failed to cache git commit {}: {e}", git_commit.id);
    }
    Ok(git_commit)
}
