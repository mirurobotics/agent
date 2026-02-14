// standard library
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

// internal crates
use miru_agent::errors::Error;

#[derive(Debug, thiserror::Error)]
#[error("MockMiruError")]
pub struct MockMiruError {
    network_err: bool,
}

impl MockMiruError {
    pub fn new(network_err: bool) -> Self {
        Self { network_err }
    }
}

impl Error for MockMiruError {
    fn is_network_connection_error(&self) -> bool {
        self.network_err
    }
}

// ================================== SLEEP ===================================== //
pub struct SleepController {
    target: Arc<AtomicBool>,
    last_known: Arc<AtomicBool>,
    is_sleeping: Arc<AtomicBool>,
    attempted_sleeps: Arc<Mutex<Vec<Duration>>>,
    completed_sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl Default for SleepController {
    fn default() -> Self {
        Self::new()
    }
}

impl SleepController {
    pub fn new() -> Self {
        Self {
            target: Arc::new(AtomicBool::new(true)),
            last_known: Arc::new(AtomicBool::new(false)),
            is_sleeping: Arc::new(AtomicBool::new(false)),
            attempted_sleeps: Arc::new(Mutex::new(Vec::new())),
            completed_sleeps: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn await_sleep(&self) {
        self.is_sleeping.store(true, Ordering::Relaxed);
        while self.is_sleeping.load(Ordering::Relaxed) {
            tokio::task::yield_now().await;
        }
    }

    pub async fn release(&self) {
        // the thread is sleeping if the target state equals the actual state. To
        // release it we just flip the target state
        self.target
            .store(!self.last_known.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    pub fn sleep_fn(
        &self,
    ) -> impl Fn(Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let attempted_sleeps = self.attempted_sleeps.clone();
        let completed_sleeps = self.completed_sleeps.clone();
        let is_sleeping = self.is_sleeping.clone();
        let target = self.target.clone();
        let last_known = self.last_known.clone();

        move |wait| {
            attempted_sleeps.lock().unwrap().push(wait);
            let completed_sleeps = completed_sleeps.clone();
            let is_sleeping = is_sleeping.clone();
            let target = target.clone();
            let last_known = last_known.clone();
            // the thread is sleeping if the target state equals the actual state. To
            // signal that the thread has begun its sleep, we set the actual state to
            // the target state.
            last_known.store(target.load(Ordering::Relaxed), Ordering::Relaxed);

            Box::pin(async move {
                while target.load(Ordering::Relaxed) == last_known.load(Ordering::Relaxed) {
                    is_sleeping.store(false, Ordering::Relaxed);
                    tokio::task::yield_now().await;
                }
                completed_sleeps.lock().unwrap().push(wait);
            })
        }
    }

    pub fn get_attempted_sleeps(&self) -> Vec<Duration> {
        self.attempted_sleeps.lock().unwrap().clone()
    }

    pub fn get_completed_sleeps(&self) -> Vec<Duration> {
        self.completed_sleeps.lock().unwrap().clone()
    }

    pub fn get_last_attempted_sleep(&self) -> Option<Duration> {
        self.attempted_sleeps.lock().unwrap().last().copied()
    }

    pub fn get_last_completed_sleep(&self) -> Option<Duration> {
        self.completed_sleeps.lock().unwrap().last().copied()
    }
}
