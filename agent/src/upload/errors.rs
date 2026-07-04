// internal crates
use crate::errors::Trace;
use crate::models::UploadCollectionID;

// reuse the actor-channel errors exactly as sync/errors.rs does
pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
#[error(
    "invalid upload rule replacement: collection id changed from '{existing_upload_collection_id}' to '{replacement_upload_collection_id}'"
)]
pub struct InvalidRule {
    pub existing_upload_collection_id: UploadCollectionID,
    pub replacement_upload_collection_id: UploadCollectionID,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidRule {}

#[derive(Debug, thiserror::Error)]
pub enum UploadErr {
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    InvalidRule(InvalidRule),
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
    InvalidRule,
});
