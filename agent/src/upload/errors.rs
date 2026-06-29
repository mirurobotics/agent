// reuse the actor-channel errors exactly as sync/errors.rs does
pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
pub enum UploadErr {
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

impl From<SendActorMessageErr> for UploadErr {
    fn from(e: SendActorMessageErr) -> Self {
        Self::SendActorMessageErr(e)
    }
}

impl From<ReceiveActorMessageErr> for UploadErr {
    fn from(e: ReceiveActorMessageErr) -> Self {
        Self::ReceiveActorMessageErr(e)
    }
}

crate::impl_error!(UploadErr {
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
