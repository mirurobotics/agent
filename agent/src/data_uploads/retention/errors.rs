// internal crates
pub use crate::data_uploads::queue::QueueFullErr;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
pub enum DeleteErr {
    #[error(transparent)]
    QueueFullErr(#[from] QueueFullErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(DeleteErr {
    QueueFullErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});
