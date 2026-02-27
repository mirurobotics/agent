use crate::models;
use crate::services::errors::ServiceErr;
use crate::storage;

pub async fn get(
    git_commits: &storage::GitCommits,
    id: String,
) -> Result<models::GitCommit, ServiceErr> {
    let gc = git_commits.read(id).await?;
    Ok(gc)
}
