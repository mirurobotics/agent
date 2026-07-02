// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::models::UploadRule;
use miru_agent::upload::{ScanOutcome, UploadErr, UploaderExt};

type ScanFn = Box<dyn Fn() -> Result<ScanOutcome, UploadErr> + Send + Sync>;

pub struct MockUploader {
    pub num_scan_calls: AtomicUsize,
    pub scan_fn: Arc<Mutex<ScanFn>>,
    pub rules: Arc<Mutex<Vec<UploadRule>>>,
}

impl Default for MockUploader {
    fn default() -> Self {
        Self::new()
    }
}

impl MockUploader {
    pub fn new() -> Self {
        Self {
            num_scan_calls: AtomicUsize::new(0),
            scan_fn: Arc::new(Mutex::new(Box::new(|| {
                Ok(ScanOutcome::Completed(Vec::new()))
            }))),
            rules: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_scan<F>(&self, scan_fn: F)
    where
        F: Fn() -> Result<ScanOutcome, UploadErr> + Send + Sync + 'static,
    {
        *self.scan_fn.lock().unwrap() = Box::new(scan_fn);
    }

    pub fn num_scan_calls(&self) -> usize {
        self.num_scan_calls.load(Ordering::Relaxed)
    }
}

impl UploaderExt for MockUploader {
    async fn shutdown(&self) -> Result<(), UploadErr> {
        Ok(())
    }

    async fn update_rules(&self, rules: Vec<UploadRule>) -> Result<(), UploadErr> {
        *self.rules.lock().unwrap() = rules;
        Ok(())
    }

    async fn get_rules(&self) -> Result<Vec<UploadRule>, UploadErr> {
        Ok(self.rules.lock().unwrap().clone())
    }

    async fn scan(&self) -> Result<ScanOutcome, UploadErr> {
        self.num_scan_calls.fetch_add(1, Ordering::Relaxed);
        (*self.scan_fn.lock().unwrap())()
    }
}
