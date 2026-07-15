// External driver for the `crate::scan` scanner actor. The actor is reactive,
// not self-scheduling: this worker imposes the cadence that drives repeated
// scan passes.

// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::scan::ScannerExt;

// external crates
use chrono::{DateTime, Utc};
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Options {
    pub scan_interval_secs: i64,
    pub ledger_retention_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            scan_interval_secs: 60,
            ledger_retention_secs: 60 * 60 * 24, // 24h; bounds dedup memory, survives reboots/redeploys
        }
    }
}

pub async fn run<F, Fut, N, ScannerT: ScannerExt>(
    options: &Options,
    scanner: &ScannerT,
    sleep_fn: F,
    now_fn: N,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
    N: Fn() -> DateTime<Utc>,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan driver worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(options, scanner, sleep_fn, now_fn) => {}
    }
}

async fn run_impl<F, Fut, N, ScannerT: ScannerExt>(
    options: &Options,
    scanner: &ScannerT,
    sleep_fn: F, // for testing purposes
    now_fn: N,   // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
    N: Fn() -> DateTime<Utc>,
{
    info!("Running scan driver worker");

    // drive an initial scan immediately, then on a fixed cadence
    if let Err(e) = scanner.scan().await {
        error!("scan driver: initial scan failed: {e:?}");
    }
    prune_ledger(scanner, options.ledger_retention_secs, &now_fn).await;

    let interval = Duration::from_secs(options.scan_interval_secs.max(0) as u64);
    loop {
        debug!("scan driver: sleeping {interval:?} until next scan");
        sleep_fn(interval).await;
        if let Err(e) = scanner.scan().await {
            error!("scan driver: scan failed, continuing: {e:?}");
        }
        prune_ledger(scanner, options.ledger_retention_secs, &now_fn).await;
    }
}

async fn prune_ledger<N, ScannerT: ScannerExt>(scanner: &ScannerT, retention_secs: i64, now_fn: &N)
where
    N: Fn() -> DateTime<Utc>,
{
    let before = now_fn() - chrono::Duration::seconds(retention_secs.max(0));
    if let Err(e) = scanner.prune(before).await {
        error!("scan driver: prune failed, continuing: {e:?}");
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // internal crates
    use super::{run, Options};
    use crate::scan::errors::InternalError;
    use crate::scan::{ScanErr, ScanEvent, ScannerExt};

    // external crates
    use chrono::{DateTime, Utc};
    use tokio::sync::mpsc;
    use tokio::sync::Mutex as TokioMutex;

    // A controllable clock holding epoch seconds in a shared atomic, so a test can
    // step time forward independently of wall-clock time. `now_fn()` yields the
    // `Fn() -> DateTime<Utc>` closure the worker's injected `now_fn` expects.
    #[derive(Clone)]
    struct Clock {
        secs: Arc<AtomicI64>,
    }

    impl Clock {
        fn new(start_secs: i64) -> Self {
            Self {
                secs: Arc::new(AtomicI64::new(start_secs)),
            }
        }

        fn now_fn(&self) -> impl Fn() -> DateTime<Utc> {
            let secs = self.secs.clone();
            move || DateTime::from_timestamp(secs.load(Ordering::SeqCst), 0).unwrap()
        }

        fn set(&self, secs: i64) {
            self.secs.store(secs, Ordering::SeqCst);
        }
    }

    // Records each `scan` and `prune(before)` call. `fail_prunes` makes the first
    // N prune calls return an error to prove the loop survives prune failures.
    #[derive(Clone)]
    struct RecordingScanner {
        scans: Arc<Mutex<usize>>,
        prunes: Arc<Mutex<Vec<DateTime<Utc>>>>,
        fail_prunes: usize,
    }

    impl RecordingScanner {
        fn new(fail_prunes: usize) -> Self {
            Self {
                scans: Arc::new(Mutex::new(0)),
                prunes: Arc::new(Mutex::new(Vec::new())),
                fail_prunes,
            }
        }

        fn scan_count(&self) -> usize {
            *self.scans.lock().unwrap()
        }

        fn prune_cutoffs(&self) -> Vec<DateTime<Utc>> {
            self.prunes.lock().unwrap().clone()
        }
    }

    impl ScannerExt for RecordingScanner {
        async fn clear_rules(&self) -> Result<(), ScanErr> {
            unimplemented!()
        }

        async fn update_rules(
            &self,
            _deployment: crate::models::Deployment,
            _rules: Vec<crate::models::UploadRule>,
        ) -> Result<(), ScanErr> {
            unimplemented!()
        }

        async fn scan(&self) -> Result<(), ScanErr> {
            *self.scans.lock().unwrap() += 1;
            Ok(())
        }

        async fn subscribe(&self) -> Result<tokio::sync::broadcast::Receiver<ScanEvent>, ScanErr> {
            unimplemented!()
        }

        async fn shutdown(&self) -> Result<(), ScanErr> {
            unimplemented!()
        }

        async fn prune(&self, before: DateTime<Utc>) -> Result<(), ScanErr> {
            let mut prunes = self.prunes.lock().unwrap();
            let idx = prunes.len();
            prunes.push(before);
            if idx < self.fail_prunes {
                return Err(internal_err());
            }
            Ok(())
        }
    }

    fn internal_err() -> ScanErr {
        ScanErr::InternalError(InternalError {
            message: "boom".to_string(),
            trace: crate::trace!(),
        })
    }

    fn cutoff(now_secs: i64, retention_secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(now_secs - retention_secs, 0).unwrap()
    }

    // A driven worker: `sleep_fn` signals a tick then blocks until the test
    // releases it, so the test steps the loop deterministically and observes
    // recorded calls between passes.
    struct Harness {
        scanner: RecordingScanner,
        clock: Clock,
        tick_rx: mpsc::UnboundedReceiver<()>,
        proceed_tx: mpsc::UnboundedSender<()>,
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn spawn(fail_prunes: usize, start_secs: i64, retention_secs: i64) -> Self {
            let scanner = RecordingScanner::new(fail_prunes);
            let clock = Clock::new(start_secs);

            let (tick_tx, tick_rx) = mpsc::unbounded_channel::<()>();
            let (proceed_tx, proceed_rx) = mpsc::unbounded_channel::<()>();
            let proceed_rx = Arc::new(TokioMutex::new(proceed_rx));

            let sleep_fn = move |_: Duration| {
                let tick_tx = tick_tx.clone();
                let proceed_rx = proceed_rx.clone();
                async move {
                    let _ = tick_tx.send(());
                    let _ = proceed_rx.lock().await.recv().await;
                }
            };

            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let shutdown_signal = Box::pin(async move {
                let _ = shutdown_rx.await;
            });

            let options = Options {
                scan_interval_secs: 60,
                ledger_retention_secs: retention_secs,
            };
            let now_fn = clock.now_fn();
            let scanner_task = scanner.clone();
            let handle = tokio::spawn(async move {
                run(&options, &scanner_task, sleep_fn, now_fn, shutdown_signal).await;
            });

            Self {
                scanner,
                clock,
                tick_rx,
                proceed_tx,
                shutdown_tx,
                handle,
            }
        }

        // Wait until the worker parks in `sleep_fn`, i.e. the pending pass
        // (scan + prune) has fully completed and been recorded.
        async fn wait_tick(&mut self) {
            self.tick_rx.recv().await.unwrap();
        }

        // Release one parked `sleep_fn` so the next pass runs.
        fn proceed(&self) {
            self.proceed_tx.send(()).unwrap();
        }

        async fn shutdown(self) {
            let _ = self.shutdown_tx.send(());
            // unblock the parked sleep so `run`'s select! can observe shutdown
            let _ = self.proceed_tx.send(());
            self.handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn prunes_on_initial_pass_and_each_tick_tracking_clock() {
        let retention = 3600_i64;
        let mut h = Harness::spawn(0, 10_000, retention);

        // initial pass: scan then prune, both before the first sleep tick
        h.wait_tick().await;
        assert_eq!(1, h.scanner.scan_count());
        let cutoffs = h.scanner.prune_cutoffs();
        assert_eq!(1, cutoffs.len());
        assert_eq!(cutoff(10_000, retention), cutoffs[0]);

        // drive cadence ticks, advancing the clock before each pass; each recorded
        // cutoff must track the moving clock (before == now - retention)
        let times = [10_050_i64, 10_120, 10_200];
        for (i, &t) in times.iter().enumerate() {
            h.clock.set(t);
            h.proceed();
            h.wait_tick().await;

            let passes = i + 2; // initial pass + (i + 1) cadence passes
            assert_eq!(passes, h.scanner.scan_count());
            let cutoffs = h.scanner.prune_cutoffs();
            assert_eq!(passes, cutoffs.len());
            assert_eq!(cutoff(t, retention), cutoffs[passes - 1]);
        }

        h.shutdown().await;
    }

    #[tokio::test]
    async fn prune_error_does_not_stop_the_loop() {
        let mut h = Harness::spawn(1, 10_000, 3600); // first prune call fails

        // initial pass records the failing prune but the loop must keep going
        h.wait_tick().await;
        assert_eq!(1, h.scanner.scan_count());
        assert_eq!(1, h.scanner.prune_cutoffs().len());

        // subsequent passes still scan and prune despite the earlier error
        for _ in 0..2 {
            h.proceed();
            h.wait_tick().await;
        }
        assert_eq!(3, h.scanner.scan_count());
        assert_eq!(3, h.scanner.prune_cutoffs().len());

        h.shutdown().await;
    }
}
