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
    update_rules_fn: Arc<Mutex<ResultFn>>,
    clear_rules_fn: Arc<Mutex<ResultFn>>,
    scan_fn: Arc<Mutex<ResultFn>>,

    // subscriptions; wrapped so `close()` can drop the sender (closing the
    // stream) while the mock stays shared behind an Arc.
    subscribe_tx: Mutex<Option<broadcast::Sender<ScanEvent>>>,
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
            update_rules_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            clear_rules_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            scan_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            subscribe_tx: Mutex::new(Some(subscribe_tx)),
        }
    }

    /// Emit a scan event to all current subscribers. Returns the number of
    /// subscribers that received it (0 if none are listening, or the stream
    /// has been closed).
    pub fn emit(&self, event: ScanEvent) -> usize {
        self.subscribe_tx
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |tx| tx.send(event).unwrap_or(0))
    }

    /// The number of live subscribers to the scan-event stream.
    pub fn subscriber_count(&self) -> usize {
        self.subscribe_tx
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |tx| tx.receiver_count())
    }

    /// Drop the event sender, closing the stream so subscribers observe
    /// `RecvError::Closed`. Models the scanner actor going away while a
    /// consumer is still running.
    pub fn close(&self) {
        self.subscribe_tx.lock().unwrap().take();
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
        Ok(self
            .subscribe_tx
            .lock()
            .unwrap()
            .as_ref()
            .expect("MockScanner::subscribe called after close")
            .subscribe())
    }

    async fn shutdown(&self) -> Result<(), ScanErr> {
        Ok(())
    }

    async fn prune(&self, _before: DateTime<Utc>) -> Result<(), ScanErr> {
        Ok(())
    }
}
