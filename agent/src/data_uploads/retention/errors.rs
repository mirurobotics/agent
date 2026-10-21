// internal crates
pub use crate::queue::QueueFullErr;

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

impl From<crate::actor::ActorErr> for DeleteErr {
    fn from(e: crate::actor::ActorErr) -> Self {
        match e {
            crate::actor::ActorErr::SendActorMessageErr(e) => Self::SendActorMessageErr(e),
            crate::actor::ActorErr::ReceiveActorMessageErr(e) => Self::ReceiveActorMessageErr(e),
        }
    }
}

crate::impl_error!(DeleteErr {
    QueueFullErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});
