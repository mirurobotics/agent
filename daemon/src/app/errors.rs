// internal crates
use crate::authn;
use crate::disk;
use crate::filesys;
use crate::http;

#[derive(Debug, thiserror::Error)]
pub enum UpgradeErr {
    #[error(transparent)]
    DiskErr(#[from] disk::DiskErr),
    #[error(transparent)]
    HTTPErr(#[from] http::HTTPErr),
    #[error(transparent)]
    AuthnErr(#[from] authn::AuthnErr),
    #[error(transparent)]
    FileSysErr(#[from] filesys::FileSysErr),
}
