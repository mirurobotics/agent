//! Plumbing shared by the agent's actors: a task that owns some state, takes
//! commands over an `mpsc` channel and answers each over a `oneshot`.
//!
//! The worker side implements [`Actor`] and uses [`serve_next`] to wait for
//! work, and [`serve_until`] to keep answering commands while something slow
//! (an upload, a backoff) runs. The handle side holds a [`Mailbox`] and calls
//! [`Mailbox::request`].
//!
//! Usage:
//!
//! ```ignore
//! let (mailbox, commands) = actor::channel("uploader", 64);
//! tokio::spawn(Worker { commands, .. }.run());
//! let len = mailbox.request("len", |respond_to| Command::Len { respond_to }).await?;
//! ```

pub mod errors;

pub use self::errors::{ActorErr, ReceiveActorMessageErr, SendActorMessageErr};

// standard crates
use std::fmt;
use std::future::Future;

// internal crates
use crate::trace;

// external crates
use tokio::sync::{mpsc, oneshot};
use tracing::error;

/// Whether an actor keeps running after handling a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Shutdown,
}

/// The worker side of an actor.
#[allow(async_fn_in_trait)]
pub trait Actor {
    type Command;

    fn commands(&mut self) -> &mut mpsc::Receiver<Self::Command>;

    async fn handle(&mut self, cmd: Self::Command) -> Flow;
}

/// Handle `cmd`, as received from [`Actor::commands`]. `None` means every
/// [`Mailbox`] was dropped, which counts as a shutdown.
pub async fn serve<A: Actor>(actor: &mut A, cmd: Option<A::Command>) -> Flow {
    match cmd {
        Some(cmd) => actor.handle(cmd).await,
        None => Flow::Shutdown,
    }
}

/// Wait for the next command and handle it.
pub async fn serve_next<A: Actor>(actor: &mut A) -> Flow {
    let cmd = actor.commands().recv().await;
    serve(actor, cmd).await
}

/// Drive `fut` to completion while handling commands, so callers are answered
/// while it runs. `None` when a command shut the actor down; `fut` is then
/// dropped. `fut` must not borrow the actor.
pub async fn serve_until<A: Actor, T>(actor: &mut A, fut: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            result = &mut fut => return Some(result),
            // recv is cancel-safe: losing the race never drops a command, and
            // a received command is handled outside the race
            cmd = actor.commands().recv() => {
                if let Flow::Shutdown = serve(actor, cmd).await {
                    return None;
                }
            }
        }
    }
}

/// Send `value` to whoever sent the `op` command. A caller that stopped
/// waiting is logged, never an error for the actor.
pub fn respond<T>(label: &str, op: &str, respond_to: oneshot::Sender<T>, value: T) {
    if respond_to.send(value).is_err() {
        error!("{label}: failed to send the {op} response");
    }
}

/// The handle side of an actor: sends it commands and waits for the answers.
pub struct Mailbox<C> {
    label: &'static str,
    sender: mpsc::Sender<C>,
}

/// A [`Mailbox`] and the command receiver for the actor it reaches. `label`
/// prefixes the mailbox's log lines.
pub fn channel<C>(label: &'static str, buffer_size: usize) -> (Mailbox<C>, mpsc::Receiver<C>) {
    let (sender, receiver) = mpsc::channel(buffer_size);
    (Mailbox { label, sender }, receiver)
}

impl<C> fmt::Debug for Mailbox<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mailbox")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<C: Send + Sync + 'static> Mailbox<C> {
    /// Send the `op` command built by `make_cmd` and wait for its response.
    pub async fn request<R>(
        &self,
        op: &str,
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> C,
    ) -> Result<R, ActorErr> {
        let label = self.label;
        let (respond_to, response) = oneshot::channel();
        self.sender.send(make_cmd(respond_to)).await.map_err(|e| {
            error!("{label}: failed to send the {op} command: {e}");
            ActorErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        response.await.map_err(|e| {
            error!("{label}: failed to receive the {op} response: {e}");
            ActorErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}
