// internal crates
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::cache::errors::CacheErr;
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::GitCommit;
use miru_agent::services::git_commit::{self as git_cmt_svc, GitCommitFetcher};
use miru_agent::services::ServiceErr;
use miru_agent::storage::GitCommits;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

// external crates
use backend_api::models as backend_client;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct StubGitCommitFetcher {
    result: Mutex<Option<Result<backend_client::GitCommit, ServiceErr>>>,
    call_count: AtomicUsize,
}

impl StubGitCommitFetcher {
    fn ok(gc: backend_client::GitCommit) -> Self {
        Self {
            result: Mutex::new(Some(Ok(gc))),
            call_count: AtomicUsize::new(0),
        }
    }
    fn err(e: ServiceErr) -> Self {
        Self {
            result: Mutex::new(Some(Err(e))),
            call_count: AtomicUsize::new(0),
        }
    }
    #[allow(dead_code)]
    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl GitCommitFetcher for StubGitCommitFetcher {
    async fn fetch_git_commit(&self, _id: &str) -> Result<backend_client::GitCommit, ServiceErr> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.result
            .lock()
            .unwrap()
            .take()
            .expect("stub called more times than canned results provided")
    }
}

struct PanicGitCommitFetcher;
impl GitCommitFetcher for PanicGitCommitFetcher {
    async fn fetch_git_commit(&self, _id: &str) -> Result<backend_client::GitCommit, ServiceErr> {
        panic!("backend should not be called on cache hit")
    }
}

async fn setup(name: &str) -> (filesys::Dir, GitCommits) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
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

        let result = git_cmt_svc::get(&stor, None::<&PanicGitCommitFetcher>, "gc_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "gc_1");
        assert_eq!(result.sha, "abc123");
        assert_eq!(result.message, "initial commit");
    }

    #[tokio::test]
    async fn not_found_returns_error() {
        let (_dir, stor) = setup("get_gc_not_found").await;

        let result = git_cmt_svc::get(
            &stor,
            None::<&PanicGitCommitFetcher>,
            "nonexistent".to_string(),
        )
        .await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_git_commit_fallback {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, stor) = setup("fb_gc_cache_hit").await;
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

        let result = git_cmt_svc::get(&stor, Some(&PanicGitCommitFetcher), "gc_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "gc_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, stor) = setup("fb_gc_backend_hit").await;
        let backend_gc = backend_client::GitCommit {
            id: "gc_1".to_string(),
            sha: "abc123".to_string(),
            message: "hi".to_string(),
            ..Default::default()
        };
        let stub = StubGitCommitFetcher::ok(backend_gc);

        let result = git_cmt_svc::get(&stor, Some(&stub), "gc_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "gc_1");
        assert_eq!(result.sha, "abc123");
        assert_eq!(stub.calls(), 1);

        // Second call with PanicGitCommitFetcher must succeed (proves cache).
        let result2 = git_cmt_svc::get(&stor, Some(&PanicGitCommitFetcher), "gc_1".to_string())
            .await
            .unwrap();
        assert_eq!(result2.id, "gc_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_404_returns_not_found() {
        let (_dir, stor) = setup("fb_gc_404").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubGitCommitFetcher::err(err);

        let result = git_cmt_svc::get(&stor, Some(&stub), "gc_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_500_returns_error() {
        let (_dir, stor) = setup("fb_gc_500").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubGitCommitFetcher::err(err);

        let result = git_cmt_svc::get(&stor, Some(&stub), "gc_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, stor) = setup("fb_gc_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubGitCommitFetcher::err(err);

        let result = git_cmt_svc::get(&stor, Some(&stub), "gc_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }

    #[tokio::test]
    async fn cache_miss_token_err_returns_not_found() {
        let (_dir, stor) = setup("fb_gc_token").await;
        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubGitCommitFetcher::err(err);

        let result = git_cmt_svc::get(&stor, Some(&stub), "gc_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_non_authn_sync_err_propagates() {
        let (_dir, stor) = setup("fb_gc_sync_err").await;
        let err = ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
            is_network_conn_err: false,
        }));
        let stub = StubGitCommitFetcher::err(err);

        let result = git_cmt_svc::get(&stor, Some(&stub), "gc_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::MockErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_no_backend_returns_not_found() {
        let (_dir, stor) = setup("fb_gc_no_backend").await;

        let result =
            git_cmt_svc::get(&stor, None::<&StubGitCommitFetcher>, "gc_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }
}
