// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

// internal crates
use backend_api::models as backend_client;
use miru_agent::services::{BackendFetcher, ServiceErr};

type FetchHook = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct StubBackend {
    before_fetch: Option<FetchHook>,
    deployment_result: Mutex<Option<Result<backend_client::Deployment, ServiceErr>>>,
    release_result: Mutex<Option<Result<backend_client::Release, ServiceErr>>>,
    git_commit_result: Mutex<Option<Result<backend_client::GitCommit, ServiceErr>>>,
    file_rule_result: Mutex<Option<Result<backend_client::BaseFileRule, ServiceErr>>>,
    deployment_calls: AtomicUsize,
    release_calls: AtomicUsize,
    git_commit_calls: AtomicUsize,
    file_rule_calls: AtomicUsize,
}

impl StubBackend {
    pub fn new() -> Self {
        Self {
            before_fetch: None,
            deployment_result: Mutex::new(None),
            release_result: Mutex::new(None),
            git_commit_result: Mutex::new(None),
            file_rule_result: Mutex::new(None),
            deployment_calls: AtomicUsize::new(0),
            release_calls: AtomicUsize::new(0),
            git_commit_calls: AtomicUsize::new(0),
            file_rule_calls: AtomicUsize::new(0),
        }
    }
    pub fn with_deployment(self, r: Result<backend_client::Deployment, ServiceErr>) -> Self {
        *self.deployment_result.lock().unwrap() = Some(r);
        self
    }
    pub fn with_release(self, r: Result<backend_client::Release, ServiceErr>) -> Self {
        *self.release_result.lock().unwrap() = Some(r);
        self
    }
    pub fn with_git_commit(self, r: Result<backend_client::GitCommit, ServiceErr>) -> Self {
        *self.git_commit_result.lock().unwrap() = Some(r);
        self
    }
    pub fn with_file_rule(self, r: Result<backend_client::BaseFileRule, ServiceErr>) -> Self {
        *self.file_rule_result.lock().unwrap() = Some(r);
        self
    }
    /// Runs `hook` at the start of every fetch, e.g. to change cache state between
    /// a service's cache read and its cache write.
    pub fn before_fetch<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.before_fetch = Some(Box::new(move || Box::pin(hook())));
        self
    }
    async fn run_before_fetch(&self) {
        if let Some(hook) = &self.before_fetch {
            hook().await;
        }
    }
    pub fn deployment_calls(&self) -> usize {
        self.deployment_calls.load(Ordering::SeqCst)
    }
    pub fn release_calls(&self) -> usize {
        self.release_calls.load(Ordering::SeqCst)
    }
    pub fn git_commit_calls(&self) -> usize {
        self.git_commit_calls.load(Ordering::SeqCst)
    }
    pub fn file_rule_calls(&self) -> usize {
        self.file_rule_calls.load(Ordering::SeqCst)
    }
}

impl BackendFetcher for StubBackend {
    async fn fetch_deployment(&self, _id: &str) -> Result<backend_client::Deployment, ServiceErr> {
        self.run_before_fetch().await;
        self.deployment_calls.fetch_add(1, Ordering::SeqCst);
        self.deployment_result
            .lock()
            .unwrap()
            .take()
            .expect("StubBackend: no canned deployment response")
    }
    async fn fetch_release(&self, _id: &str) -> Result<backend_client::Release, ServiceErr> {
        self.run_before_fetch().await;
        self.release_calls.fetch_add(1, Ordering::SeqCst);
        self.release_result
            .lock()
            .unwrap()
            .take()
            .expect("StubBackend: no canned release response")
    }
    async fn fetch_git_commit(&self, _id: &str) -> Result<backend_client::GitCommit, ServiceErr> {
        self.run_before_fetch().await;
        self.git_commit_calls.fetch_add(1, Ordering::SeqCst);
        self.git_commit_result
            .lock()
            .unwrap()
            .take()
            .expect("StubBackend: no canned git_commit response")
    }
    async fn fetch_file_rule(&self, _id: &str) -> Result<backend_client::BaseFileRule, ServiceErr> {
        self.run_before_fetch().await;
        self.file_rule_calls.fetch_add(1, Ordering::SeqCst);
        self.file_rule_result
            .lock()
            .unwrap()
            .take()
            .expect("StubBackend: no canned file_rule response")
    }
}

pub struct PanicBackend;
impl BackendFetcher for PanicBackend {
    async fn fetch_deployment(&self, _id: &str) -> Result<backend_client::Deployment, ServiceErr> {
        panic!("PanicBackend::fetch_deployment called — backend should not be consulted")
    }
    async fn fetch_release(&self, _id: &str) -> Result<backend_client::Release, ServiceErr> {
        panic!("PanicBackend::fetch_release called — backend should not be consulted")
    }
    async fn fetch_git_commit(&self, _id: &str) -> Result<backend_client::GitCommit, ServiceErr> {
        panic!("PanicBackend::fetch_git_commit called — backend should not be consulted")
    }
    async fn fetch_file_rule(&self, _id: &str) -> Result<backend_client::BaseFileRule, ServiceErr> {
        panic!("PanicBackend::fetch_file_rule called — backend should not be consulted")
    }
}
