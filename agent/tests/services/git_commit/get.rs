use miru_agent::filesys::dir::Dir;
use miru_agent::filesys::Overwrite;
use miru_agent::models::GitCommit;
use miru_agent::services::errors::ServiceErr;
use miru_agent::services::git_commit as git_cmt_svc;
use miru_agent::storage::GitCommits;

async fn setup(name: &str) -> (Dir, GitCommits) {
    let dir = Dir::create_temp_dir(name).await.unwrap();
    let (stor, _) = GitCommits::spawn(16, dir.file("git_commits.json"), 1000)
        .await
        .unwrap();
    (dir, stor)
}

pub mod get_git_commit {
    use super::*;

    #[tokio::test]
    async fn returns_git_commit_by_id() {
        let (_dir, stor) = setup("get_gc_by_id").await;
        let gc = GitCommit {
            id: "gc_1".to_string(),
            sha: "abc123".to_string(),
            message: "initial commit".to_string(),
            ..Default::default()
        };
        stor.write(
            "gc_1".to_string(),
            gc.clone(),
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = git_cmt_svc::get(&stor, "gc_1".to_string()).await.unwrap();
        assert_eq!(result.id, "gc_1");
        assert_eq!(result.sha, "abc123");
        assert_eq!(result.message, "initial commit");
    }

    #[tokio::test]
    async fn not_found_returns_error() {
        let (_dir, stor) = setup("get_gc_not_found").await;

        let result = git_cmt_svc::get(&stor, "nonexistent".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}
