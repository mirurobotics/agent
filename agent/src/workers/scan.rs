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
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Options {
    pub scan_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            scan_interval_secs: 60,
        }
    }
}

pub async fn run<F, Fut, ScannerT: ScannerExt>(
    options: &Options,
    scanner: &ScannerT,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan driver worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(options, scanner, sleep_fn) => {}
    }
}

async fn run_impl<F, Fut, ScannerT: ScannerExt>(
    options: &Options,
    scanner: &ScannerT,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running scan driver worker");

    // drive an initial scan immediately, then on a fixed cadence
    if let Err(e) = scanner.scan().await {
        error!("scan driver: initial scan failed: {e:?}");
    }
    prune_ledger(scanner).await;

    let interval = Duration::from_secs(options.scan_interval_secs.max(0) as u64);
    loop {
        debug!("scan driver: sleeping {interval:?} until next scan");
        sleep_fn(interval).await;
        if let Err(e) = scanner.scan().await {
            error!("scan driver: scan failed, continuing: {e:?}");
        }
        prune_ledger(scanner).await;
    }
}

async fn prune_ledger<ScannerT: ScannerExt>(scanner: &ScannerT) {
    if let Err(e) = scanner.prune().await {
        error!("scan driver: prune failed, continuing: {e:?}");
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // internal crates
    use super::{run, Options};
    use crate::scan::errors::InternalError;
    use crate::scan::{ScanErr, ScanEvent, ScannerExt};

    // external crates
    use tokio::sync::mpsc;
    use tokio::sync::Mutex as TokioMutex;

    // Records each `scan` and `prune` call. `fail_prunes` makes the first N prune
    // calls return an error to prove the loop survives prune failures.
    #[derive(Clone)]
    struct RecordingScanner {
        scans: Arc<Mutex<usize>>,
        prunes: Arc<Mutex<usize>>,
        fail_prunes: usize,
    }

    impl RecordingScanner {
        fn new(fail_prunes: usize) -> Self {
            Self {
                scans: Arc::new(Mutex::new(0)),
                prunes: Arc::new(Mutex::new(0)),
                fail_prunes,
            }
        }

        fn scan_count(&self) -> usize {
            *self.scans.lock().unwrap()
        }

        fn prune_count(&self) -> usize {
            *self.prunes.lock().unwrap()
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

        async fn prune(&self) -> Result<(), ScanErr> {
            let mut prunes = self.prunes.lock().unwrap();
            let idx = *prunes;
            *prunes += 1;
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

    // A driven worker: `sleep_fn` signals a tick then blocks until the test
    // releases it, so the test steps the loop deterministically and observes
    // recorded calls between passes.
    struct Harness {
        scanner: RecordingScanner,
        tick_rx: mpsc::UnboundedReceiver<()>,
        proceed_tx: mpsc::UnboundedSender<()>,
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn spawn(fail_prunes: usize) -> Self {
            let scanner = RecordingScanner::new(fail_prunes);

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
            };
            let scanner_task = scanner.clone();
            let handle = tokio::spawn(async move {
                run(&options, &scanner_task, sleep_fn, shutdown_signal).await;
            });

            Self {
                scanner,
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
    async fn prunes_on_initial_pass_and_each_tick() {
        let mut h = Harness::spawn(0);

        // initial pass: scan then prune, both before the first sleep tick
        h.wait_tick().await;
        assert_eq!(1, h.scanner.scan_count());
        assert_eq!(1, h.scanner.prune_count());

        // drive cadence ticks; each pass scans then prunes exactly once, so the
        // prune count tracks the pass count (N + 1 total after N ticks)
        for i in 0..3 {
            h.proceed();
            h.wait_tick().await;

            let passes = i + 2; // initial pass + (i + 1) cadence passes
            assert_eq!(passes, h.scanner.scan_count());
            assert_eq!(passes, h.scanner.prune_count());
        }

        h.shutdown().await;
    }

    #[tokio::test]
    async fn prune_error_does_not_stop_the_loop() {
        let mut h = Harness::spawn(1); // first prune call fails

        // initial pass records the failing prune but the loop must keep going
        h.wait_tick().await;
        assert_eq!(1, h.scanner.scan_count());
        assert_eq!(1, h.scanner.prune_count());

        // subsequent passes still scan and prune despite the earlier error
        for _ in 0..2 {
            h.proceed();
            h.wait_tick().await;
        }
        assert_eq!(3, h.scanner.scan_count());
        assert_eq!(3, h.scanner.prune_count());

        h.shutdown().await;
    }
}
