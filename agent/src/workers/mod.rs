// standard crates
use std::future;

// internal crates
use crate::sync::syncer::SyncEvent;

// external crates
use tokio::sync::watch;

pub mod mqtt;
pub mod poller;
// The scan bridge worker is exercised by its own tests but is not yet wired
// into app::run; the app integration (spawn order, shutdown) lands separately.
#[allow(dead_code)]
pub mod scan_bridge;
pub mod token_refresh;

/// Wait for the next sync event. A missing subscription remains pending so
/// workers can disable this branch without preventing their other work.
async fn next_sync_event(subscriber: &mut Option<watch::Receiver<SyncEvent>>) -> Option<SyncEvent> {
    let Some(subscriber) = subscriber.as_mut() else {
        return future::pending().await;
    };

    if subscriber.changed().await.is_err() {
        return None;
    }

    let event = subscriber.borrow_and_update().clone();
    Some(event)
}

#[cfg(test)]
mod next_sync_event {
    use super::*;

    #[tokio::test]
    async fn returns_next_event() {
        let (tx, rx) = watch::channel(SyncEvent::SyncSuccess);
        let mut subscriber = Some(rx);
        tx.send(SyncEvent::CooldownEnd(
            crate::sync::syncer::CooldownEnd::SyncSuccess,
        ))
        .unwrap();

        assert!(matches!(
            next_sync_event(&mut subscriber).await,
            Some(SyncEvent::CooldownEnd(
                crate::sync::syncer::CooldownEnd::SyncSuccess
            ))
        ));
    }

    #[tokio::test]
    async fn returns_none_when_stream_ends() {
        let (tx, rx) = watch::channel(SyncEvent::SyncSuccess);
        let mut subscriber = Some(rx);
        drop(tx);

        assert!(next_sync_event(&mut subscriber).await.is_none());
    }

    #[tokio::test]
    async fn missing_subscription_remains_pending() {
        let mut subscriber = None;

        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            next_sync_event(&mut subscriber),
        )
        .await
        .is_err());
    }
}
