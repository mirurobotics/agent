// internal crates
use crate::authn;
use crate::filesys;
use crate::http;
use crate::storage;

#[derive(Debug, thiserror::Error)]
pub enum UpgradeErr {
    #[error(transparent)]
    StorageErr(#[from] storage::StorageErr),
    #[error(transparent)]
    HTTPErr(#[from] http::HTTPErr),
    #[error(transparent)]
    AuthnErr(#[from] authn::AuthnErr),
    #[error(transparent)]
    FileSysErr(#[from] filesys::FileSysErr),
}
