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
pub enum ScanErr {
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    InvalidRule(InvalidRule),
    #[error(transparent)]
    FileSysErr(crate::filesys::FileSysErr),
}

impl From<SendActorMessageErr> for ScanErr {
    fn from(e: SendActorMessageErr) -> Self {
        Self::SendActorMessageErr(e)
    }
}

impl From<ReceiveActorMessageErr> for ScanErr {
    fn from(e: ReceiveActorMessageErr) -> Self {
        Self::ReceiveActorMessageErr(e)
    }
}

impl From<crate::filesys::FileSysErr> for ScanErr {
    fn from(e: crate::filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(ScanErr {
    SendActorMessageErr,
    ReceiveActorMessageErr,
    InvalidRule,
    FileSysErr,
});
