// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::sync::{
    errors::SyncErr,
    syncer::{State, SyncEvent, SyncerExt},
};

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::watch;

type GetSyncStateFn = Box<dyn Fn() -> State + Send + Sync>;
type SyncFn = Box<dyn Fn() -> Result<(), SyncErr> + Send + Sync>;

pub struct MockSyncer {
    pub last_attempted_sync_at: Arc<Mutex<DateTime<Utc>>>,
    pub num_sync_calls: AtomicUsize,
    pub get_sync_state_fn: Arc<Mutex<GetSyncStateFn>>,
    pub sync_fn: Arc<Mutex<SyncFn>>,

    // subscriptions
    pub subscribe_rx: watch::Receiver<SyncEvent>,
    pub subscribe_tx: watch::Sender<SyncEvent>,
}

impl Default for MockSyncer {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSyncer {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(SyncEvent::SyncSuccess);

        Self {
            last_attempted_sync_at: Arc::new(Mutex::new(DateTime::<Utc>::UNIX_EPOCH)),
            num_sync_calls: AtomicUsize::new(0),
            get_sync_state_fn: Arc::new(Mutex::new(Box::new(|| State {
                last_attempted_sync_at: DateTime::<Utc>::UNIX_EPOCH,
                last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
                cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
                err_streak: 0,
            }))),
            sync_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),

            // subscriptions
            subscribe_rx: rx,
            subscribe_tx: tx,
        }
    }

    pub fn set_state(&self, state: State) {
        *self.get_sync_state_fn.lock().unwrap() = Box::new(move || state.clone());
    }

    pub fn get_transmitter(&self) -> watch::Sender<SyncEvent> {
        self.subscribe_tx.clone()
    }

    pub fn set_sync<F>(&self, sync_fn: F)
    where
        F: Fn() -> Result<(), SyncErr> + Send + Sync + 'static,
    {
        *self.sync_fn.lock().unwrap() = Box::new(sync_fn);
    }

    pub fn num_sync_calls(&self) -> usize {
        self.num_sync_calls.load(Ordering::Relaxed)
    }
}

impl SyncerExt for MockSyncer {
    async fn shutdown(&self) -> Result<(), SyncErr> {
        Ok(())
    }

    async fn get_sync_state(&self) -> Result<State, SyncErr> {
        Ok((*self.get_sync_state_fn.lock().unwrap())())
    }

    async fn is_in_cooldown(&self) -> Result<bool, SyncErr> {
        let state = self.get_sync_state().await.unwrap();
        Ok(state.is_in_cooldown())
    }

    async fn get_cooldown_ends_at(&self) -> Result<DateTime<Utc>, SyncErr> {
        let state = self.get_sync_state().await.unwrap();
        Ok(state.cooldown_ends_at)
    }

    async fn get_last_attempted_sync_at(&self) -> Result<DateTime<Utc>, SyncErr> {
        let state = self.get_sync_state().await.unwrap();
        Ok(state.last_attempted_sync_at)
    }

    async fn sync(&self) -> Result<(), SyncErr> {
        *self.last_attempted_sync_at.lock().unwrap() = Utc::now();
        self.num_sync_calls.fetch_add(1, Ordering::Relaxed);
        (*self.sync_fn.lock().unwrap())()
    }

    async fn sync_if_not_in_cooldown(&self) -> Result<(), SyncErr> {
        let state = self.get_sync_state().await.unwrap();
        if !state.is_in_cooldown() {
            self.sync().await?;
        }
        Ok(())
    }

    async fn subscribe(&self) -> Result<watch::Receiver<SyncEvent>, SyncErr> {
        Ok(self.subscribe_rx.clone())
    }
}
