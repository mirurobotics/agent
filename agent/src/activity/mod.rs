// standard library
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// external crates
use tracing::error;

#[derive(Clone, Debug)]
pub struct Tracker {
    last_activity: Arc<AtomicU64>,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            last_activity: Arc::new(AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            )),
        }
    }

    pub fn last_touched(&self) -> u64 {
        self.last_activity.load(Ordering::Relaxed)
    }

    pub fn touch(&self) {
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(e) => {
                error!("Failed to record activity: {:?}", e);
                return;
            }
        };
        self.last_activity.store(now, Ordering::Relaxed);
    }
}
