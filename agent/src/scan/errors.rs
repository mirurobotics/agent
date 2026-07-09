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
#[error("duplicate upload collection id in rule set: '{collection_id}'")]
pub struct DuplicateCollectionID {
    pub collection_id: UploadCollectionID,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DuplicateCollectionID {}

#[derive(Debug, thiserror::Error)]
pub enum ScanErr {
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    InvalidRule(InvalidRule),
    #[error(transparent)]
    DuplicateCollectionID(DuplicateCollectionID),
    #[error(transparent)]
    FileSysErr(crate::filesys::FileSysErr),
    #[error(transparent)]
    CacheErr(crate::cache::CacheErr),
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

impl From<crate::cache::CacheErr> for ScanErr {
    fn from(e: crate::cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

crate::impl_error!(ScanErr {
    SendActorMessageErr,
    ReceiveActorMessageErr,
    InvalidRule,
    DuplicateCollectionID,
    FileSysErr,
    CacheErr,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace;

    fn boxed_source() -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(std::io::Error::other("boom"))
    }

    #[test]
    fn from_send_actor_message_err() {
        let err = SendActorMessageErr {
            source: boxed_source(),
            trace: trace!(),
        };
        assert!(matches!(
            ScanErr::from(err),
            ScanErr::SendActorMessageErr(_)
        ));
    }

    #[test]
    fn from_receive_actor_message_err() {
        let err = ReceiveActorMessageErr {
            source: boxed_source(),
            trace: trace!(),
        };
        assert!(matches!(
            ScanErr::from(err),
            ScanErr::ReceiveActorMessageErr(_)
        ));
    }

    #[test]
    fn from_filesys_err() {
        let err = crate::filesys::FileSysErr::PathDoesNotExistErr(
            crate::filesys::errors::PathDoesNotExistErr {
                path: std::path::PathBuf::from("/nope"),
                trace: trace!(),
            },
        );
        assert!(matches!(ScanErr::from(err), ScanErr::FileSysErr(_)));
    }

    #[test]
    fn from_cache_err() {
        let err = crate::cache::CacheErr::CacheElementNotFound(
            crate::cache::errors::CacheElementNotFound {
                msg: "missing".to_string(),
                trace: trace!(),
            },
        );
        assert!(matches!(ScanErr::from(err), ScanErr::CacheErr(_)));
    }
}
