// internal crates
use crate::errors::Trace;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
#[error("failed to join the blocking file discovery task: {source}")]
pub struct JoinDiscoveryTaskErr {
    pub source: Box<tokio::task::JoinError>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for JoinDiscoveryTaskErr {}

#[derive(Debug, thiserror::Error)]
pub enum UploadErr {
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    JoinDiscoveryTaskErr(JoinDiscoveryTaskErr),
}

crate::impl_error!(UploadErr {
    SendActorMessageErr,
    ReceiveActorMessageErr,
    JoinDiscoveryTaskErr,
});
