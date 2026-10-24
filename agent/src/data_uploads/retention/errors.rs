// internal crates
pub use crate::queue::{EnqueueErr, QueueFullErr};

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
pub enum DeleteErr {
    #[error(transparent)]
    EnqueueErr(#[from] EnqueueErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(DeleteErr {
    EnqueueErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});
