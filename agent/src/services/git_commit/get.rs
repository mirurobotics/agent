use crate::models::git_commit::GitCommit;
use crate::services::errors::ServiceErr;
use crate::storage::GitCommits;

pub async fn get(git_commits: &GitCommits, id: String) -> Result<GitCommit, ServiceErr> {
    let gc = git_commits.read(id).await?;
    Ok(gc)
}
