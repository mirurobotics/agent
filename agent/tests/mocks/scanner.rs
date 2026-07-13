// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::models::{Deployment, UploadRule};
use miru_agent::scan::{ScanErr, ScanEvent, ScannerExt};

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;

type ResultFn = Box<dyn Fn() -> Result<(), ScanErr> + Send + Sync>;
type UpdateRulesCalls = Arc<Mutex<Vec<(Deployment, Vec<UploadRule>)>>>;

/// A test double for [`ScannerExt`] that records `update_rules` / `clear_rules` /
/// `scan` calls and lets a test inject an error result for those methods (mirrors
/// how `MockSyncer` exposes a settable `sync_fn`). All other trait methods return
/// sensible defaults.
pub struct MockScanner {
    update_rules_calls: UpdateRulesCalls,
    clear_rules_calls: AtomicUsize,
    num_scan_calls: AtomicUsize,
    prune_calls: Arc<Mutex<Vec<DateTime<Utc>>>>,
    update_rules_fn: Arc<Mutex<ResultFn>>,
    clear_rules_fn: Arc<Mutex<ResultFn>>,
    scan_fn: Arc<Mutex<ResultFn>>,
    prune_fn: Arc<Mutex<ResultFn>>,

    // subscriptions
    subscribe_tx: broadcast::Sender<ScanEvent>,
}

impl Default for MockScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl MockScanner {
    pub fn new() -> Self {
        let (subscribe_tx, _) = broadcast::channel(256);
        Self {
            update_rules_calls: Arc::new(Mutex::new(Vec::new())),
            clear_rules_calls: AtomicUsize::new(0),
            num_scan_calls: AtomicUsize::new(0),
            prune_calls: Arc::new(Mutex::new(Vec::new())),
            update_rules_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            clear_rules_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            scan_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            prune_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            subscribe_tx,
        }
    }

    /// The recorded `update_rules` calls, in order.
    pub fn update_rules_calls(&self) -> Vec<(Deployment, Vec<UploadRule>)> {
        self.update_rules_calls.lock().unwrap().clone()
    }

    /// The number of `clear_rules` calls.
    pub fn clear_rules_calls(&self) -> usize {
        self.clear_rules_calls.load(Ordering::Relaxed)
    }

    /// The number of `scan` calls.
    pub fn num_scan_calls(&self) -> usize {
        self.num_scan_calls.load(Ordering::Relaxed)
    }

    /// Override the result returned by `update_rules` (the call is still
    /// recorded before the result is produced).
    pub fn set_update_rules<F>(&self, f: F)
    where
        F: Fn() -> Result<(), ScanErr> + Send + Sync + 'static,
    {
        *self.update_rules_fn.lock().unwrap() = Box::new(f);
    }

    /// Override the result returned by `clear_rules` (the call is still counted
    /// before the result is produced).
    pub fn set_clear_rules<F>(&self, f: F)
    where
        F: Fn() -> Result<(), ScanErr> + Send + Sync + 'static,
    {
        *self.clear_rules_fn.lock().unwrap() = Box::new(f);
    }

    /// Override the result returned by `scan` (the call is still counted before
    /// the result is produced).
    pub fn set_scan<F>(&self, f: F)
    where
        F: Fn() -> Result<(), ScanErr> + Send + Sync + 'static,
    {
        *self.scan_fn.lock().unwrap() = Box::new(f);
    }

    /// The recorded `prune` cutoffs, in order.
    pub fn prune_calls(&self) -> Vec<DateTime<Utc>> {
        self.prune_calls.lock().unwrap().clone()
    }

    /// Override the result returned by `prune` (the call is still recorded
    /// before the result is produced).
    pub fn set_prune<F>(&self, f: F)
    where
        F: Fn() -> Result<(), ScanErr> + Send + Sync + 'static,
    {
        *self.prune_fn.lock().unwrap() = Box::new(f);
    }
}

impl ScannerExt for MockScanner {
    async fn clear_rules(&self) -> Result<(), ScanErr> {
        self.clear_rules_calls.fetch_add(1, Ordering::Relaxed);
        (*self.clear_rules_fn.lock().unwrap())()
    }

    async fn update_rules(
        &self,
        deployment: Deployment,
        rules: Vec<UploadRule>,
    ) -> Result<(), ScanErr> {
        self.update_rules_calls
            .lock()
            .unwrap()
            .push((deployment, rules));
        (*self.update_rules_fn.lock().unwrap())()
    }

    async fn scan(&self) -> Result<(), ScanErr> {
        self.num_scan_calls.fetch_add(1, Ordering::Relaxed);
        (*self.scan_fn.lock().unwrap())()
    }

    async fn subscribe(&self) -> Result<broadcast::Receiver<ScanEvent>, ScanErr> {
        Ok(self.subscribe_tx.subscribe())
    }

    async fn shutdown(&self) -> Result<(), ScanErr> {
        Ok(())
    }

    async fn prune(&self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.prune_calls.lock().unwrap().push(before);
        (*self.prune_fn.lock().unwrap())()
    }
}
