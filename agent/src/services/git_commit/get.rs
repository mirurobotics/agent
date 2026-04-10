// internal crates
use crate::filesys;
use crate::models;
use crate::services::{backend::BackendFetcher, errors::ServiceErr};
use crate::storage;

// external crates
use tracing::error;

pub async fn get<B: BackendFetcher>(
    git_commits: &storage::GitCommits,
    backend: &B,
    id: String,
) -> Result<models::GitCommit, ServiceErr> {
    let cached = git_commits.read_optional(id.clone()).await?;
    if let Some(gc) = cached {
        return Ok(gc);
    }
    let backend_gc = backend.fetch_git_commit(&id).await?;
    let storage_gc = models::GitCommit::from(backend_gc);
    cache_git_commit(git_commits, storage_gc.clone()).await;
    Ok(storage_gc)
}

async fn cache_git_commit(git_commits: &storage::GitCommits, storage_gc: models::GitCommit) {
    let id = storage_gc.id.clone();
    if let Err(e) = git_commits
        .write(
            id.clone(),
            storage_gc,
            |_, _| false,
            filesys::Overwrite::Allow,
        )
        .await
    {
        error!("failed to cache git_commit {}: {e}", id);
    }
}
