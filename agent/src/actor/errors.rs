// internal crates
use crate::errors::Trace;

#[derive(Debug, thiserror::Error)]
#[error("failed to send actor message: {source:?}")]
pub struct SendActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SendActorMessageErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to receive actor message: {source}")]
pub struct ReceiveActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReceiveActorMessageErr {}

/// Why a [`super::Mailbox::request`] got no response: the actor had stopped
/// before the command was sent, or stopped without answering it.
#[derive(Debug, thiserror::Error)]
pub enum ActorErr {
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(ActorErr {
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
