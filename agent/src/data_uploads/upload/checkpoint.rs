// standard crates
use std::future::Future;

// internal crates
use crate::data_uploads::upload::job::Checkpoint;

// external crates
use tokio::sync::{mpsc, oneshot};

/// Where an in-flight attempt records upload state that a later attempt or
/// process needs. `save` returns once the checkpoint is persisted (or can no
/// longer be), so the caller can rely on it before its next step, e.g. the
/// upload id is on disk before any bytes go to the bucket.
pub trait Checkpointer: Send + Sync {
    fn save(&self, checkpoint: Checkpoint) -> impl Future<Output = ()> + Send;
}

/// A checkpoint plus the channel the receiver acknowledges it on.
pub(crate) type CheckpointMsg = (Checkpoint, oneshot::Sender<()>);

/// Sends checkpoints to the uploader actor, which owns the queue file, and
/// waits for it to persist each one.
#[derive(Clone)]
pub struct ChannelCheckpointer {
    sender: mpsc::Sender<CheckpointMsg>,
}

impl ChannelCheckpointer {
    pub(crate) fn new(sender: mpsc::Sender<CheckpointMsg>) -> Self {
        Self { sender }
    }
}

impl Checkpointer for ChannelCheckpointer {
    async fn save(&self, checkpoint: Checkpoint) {
        let (ack_tx, ack_rx) = oneshot::channel();
        // A closed channel means the attempt is being torn down; nothing is
        // left to persist to, and the attempt is about to be dropped anyway.
        if self.sender.send((checkpoint, ack_tx)).await.is_err() {
            return;
        }
        let _ = ack_rx.await;
    }
}

/// Discards checkpoints. For callers that don't persist upload state.
pub struct NoopCheckpointer;

impl Checkpointer for NoopCheckpointer {
    async fn save(&self, _checkpoint: Checkpoint) {}
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    #[tokio::test]
    async fn channel_checkpointer_waits_for_the_ack() {
        let (tx, mut rx) = mpsc::channel(1);
        let checkpointer = ChannelCheckpointer::new(tx);
        let receiver = tokio::spawn(async move {
            let (checkpoint, ack) = rx.recv().await.unwrap();
            ack.send(()).unwrap();
            checkpoint
        });

        checkpointer.save(Checkpoint::Transferred).await;

        assert_eq!(receiver.await.unwrap(), Checkpoint::Transferred);
    }

    #[tokio::test]
    async fn channel_checkpointer_returns_when_the_receiver_is_gone() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        ChannelCheckpointer::new(tx)
            .save(Checkpoint::TransferCleared)
            .await;
    }

    #[tokio::test]
    async fn channel_checkpointer_returns_when_the_ack_is_dropped() {
        let (tx, mut rx) = mpsc::channel(1);
        let receiver = tokio::spawn(async move {
            let _ = rx.recv().await;
        });
        ChannelCheckpointer::new(tx)
            .save(Checkpoint::UploadCleared)
            .await;
        receiver.await.unwrap();
    }

    #[tokio::test]
    async fn noop_checkpointer_discards() {
        NoopCheckpointer.save(Checkpoint::Transferred).await;
    }
}
