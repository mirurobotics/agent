// standard library
use std::path::Path;
use std::sync::Mutex;

// internal crates
use super::errors::{CursorExpiredErr, EventErr};
use super::model::Envelope;
use super::store::Store;

// external crates
use tokio::sync::broadcast;
use tracing::error;

const DEFAULT_MAX_RETAINED: usize = 10_000;
const DEFAULT_BROADCAST_CAPACITY: usize = 256;

/// Central coordinator for the event subsystem. Combines a durable NDJSON store
/// with an in-memory broadcast channel for low-latency live streaming.
///
/// The store is behind a `Mutex` because it is only accessed from the syncer
/// (single-threaded actor) and the SSE handler (read path). Contention is
/// minimal in practice.
pub struct EventHub {
    store: Mutex<Store>,
    broadcast_tx: broadcast::Sender<Envelope>,
}

impl std::fmt::Debug for EventHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventHub")
            .field("store", &"<locked>")
            .finish()
    }
}

impl EventHub {
    /// Initialize the event hub. Creates the parent directory and recovers
    /// state from any existing log.
    pub fn init(log_path: &Path, meta_path: &Path) -> Result<Self, EventErr> {
        Self::init_with_capacity(
            log_path,
            meta_path,
            DEFAULT_MAX_RETAINED,
            DEFAULT_BROADCAST_CAPACITY,
        )
    }

    pub fn init_with_capacity(
        log_path: &Path,
        meta_path: &Path,
        max_retained: usize,
        broadcast_capacity: usize,
    ) -> Result<Self, EventErr> {
        let store = Store::init(log_path, meta_path, max_retained)?;
        let (broadcast_tx, _) = broadcast::channel(broadcast_capacity);

        Ok(Self {
            store: Mutex::new(store),
            broadcast_tx,
        })
    }

    /// Assign a monotonic ID, persist the event, and broadcast to live
    /// subscribers. Returns the envelope with the assigned ID.
    pub fn publish(&self, envelope: Envelope) -> Result<Envelope, EventErr> {
        let envelope = {
            let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            store.append(envelope)?
        };

        // broadcast to live subscribers; it's fine if nobody is listening
        let _ = self.broadcast_tx.send(envelope.clone());

        Ok(envelope)
    }

    /// Replay events with id > `after`, up to `limit`. Returns envelopes in
    /// ascending ID order.
    ///
    /// Returns `CursorExpiredErr` if `after` is older than the earliest
    /// retained event (and `after` is not 0, which means "from the beginning").
    pub fn replay_after(&self, after: u64, limit: usize) -> Result<Vec<Envelope>, EventErr> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());

        // cursor 0 means "from the beginning"
        if after > 0 {
            if let Some(earliest) = store.earliest_id() {
                // replay_after returns events with id > after, so a cursor is only
                // expired if the next required event was already compacted away.
                if after.saturating_add(1) < earliest {
                    return Err(CursorExpiredErr {
                        cursor: after,
                        earliest,
                    }
                    .into());
                }
            }
        }

        Ok(store.replay_after(after, limit)?)
    }

    /// Subscribe to live events. Returns a broadcast receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<Envelope> {
        self.broadcast_tx.subscribe()
    }

    /// The next event ID that will be assigned.
    pub fn next_id(&self) -> u64 {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.next_id()
    }

    /// The earliest retained event ID, if any.
    pub fn earliest_id(&self) -> Option<u64> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.earliest_id()
    }

    /// Publish an event, logging errors instead of propagating them. Useful in
    /// fire-and-forget contexts where the caller does not want to fail on event
    /// emission.
    pub fn try_publish(&self, envelope: Envelope) {
        if let Err(e) = self.publish(envelope) {
            error!("failed to publish event: {e}");
        }
    }
}
