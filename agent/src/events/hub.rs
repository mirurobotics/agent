// internal crates
use crate::events::{
    errors::{EventsErr, ReceiveActorMessageErr, SendActorMessageErr},
    model::{Event, EventArgs},
    store::{EventStore, DEFAULT_MAX_RETAINED},
};
use crate::filesys;
use crate::trace;

// external crates
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::error;

pub struct SpawnOptions {
    pub buffer_size: usize,
    pub max_retained: usize,
    pub broadcast_capacity: usize,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            buffer_size: 64,
            max_retained: DEFAULT_MAX_RETAINED,
            broadcast_capacity: 256,
        }
    }
}

// ================================= COMMANDS ====================================== //
enum Command {
    Shutdown {
        respond_to: oneshot::Sender<Result<(), EventsErr>>,
    },
    Publish {
        event: EventArgs,
        respond_to: oneshot::Sender<Result<Event, EventsErr>>,
    },
    ReplayAfter {
        cursor: u64,
        respond_to: oneshot::Sender<Result<Vec<Event>, EventsErr>>,
    },
}

// =================================== WORKER ====================================== //
struct Worker {
    store: EventStore,
    broadcast_tx: broadcast::Sender<Event>,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if respond_to.send(Ok(())).is_err() {
                        error!("EventHub worker failed to send shutdown response");
                    }
                    break;
                }
                Command::Publish { event, respond_to } => {
                    let result = self.store.append(event).await;
                    if let Ok(ref event) = result {
                        // broadcast synchronously with append
                        let _ = self.broadcast_tx.send(event.clone());
                    }
                    if respond_to.send(result).is_err() {
                        error!("EventHub worker failed to send publish response");
                    }
                }
                Command::ReplayAfter { cursor, respond_to } => {
                    if respond_to.send(self.store.replay_after(cursor)).is_err() {
                        error!("EventHub worker failed to send replay_after response");
                    }
                }
            }
        }
    }
}

// ================================ EVENT HUB (HANDLE) ============================= //
#[derive(Debug, Clone)]
pub struct EventHub {
    sender: mpsc::Sender<Command>,
    broadcast_tx: broadcast::Sender<Event>,
}

impl EventHub {
    pub async fn spawn(
        log_file: filesys::File,
        opts: SpawnOptions,
    ) -> Result<(Self, JoinHandle<()>), EventsErr> {
        let store = EventStore::init(log_file, opts.max_retained).await?;
        let (broadcast_tx, _) = broadcast::channel(opts.broadcast_capacity);
        let (sender, receiver) = mpsc::channel(opts.buffer_size);

        let worker = Worker {
            store,
            broadcast_tx: broadcast_tx.clone(),
            receiver,
        };
        let handle = tokio::spawn(worker.run());

        Ok((
            Self {
                sender,
                broadcast_tx,
            },
            handle,
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.broadcast_tx.subscribe()
    }

    pub async fn publish(&self, event: EventArgs) -> Result<Event, EventsErr> {
        self.send_command(|tx| Command::Publish {
            event,
            respond_to: tx,
        })
        .await?
    }

    pub async fn replay_after(&self, cursor: u64) -> Result<Vec<Event>, EventsErr> {
        self.send_command(|tx| Command::ReplayAfter {
            cursor,
            respond_to: tx,
        })
        .await?
    }

    pub async fn try_publish(&self, event: EventArgs) {
        if let Err(e) = self.publish(event).await {
            error!("failed to publish event: {e}");
        }
    }

    pub async fn shutdown(&self) -> Result<(), EventsErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await?
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, EventsErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            EventsErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            EventsErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}
