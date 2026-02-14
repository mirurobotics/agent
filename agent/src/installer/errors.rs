use crate::authn::errors::AuthnErr;
use crate::crypt::errors::CryptErr;
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;
use crate::storage::errors::StorageErr;

#[derive(Debug, thiserror::Error)]
pub enum InstallErr {
    #[error(transparent)]
    AuthnErr(AuthnErr),
    #[error(transparent)]
    CryptErr(CryptErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    HTTPErr(HTTPErr),
    #[error(transparent)]
    StorageErr(StorageErr),
}

impl From<AuthnErr> for InstallErr {
    fn from(e: AuthnErr) -> Self {
        Self::AuthnErr(e)
    }
}

impl From<CryptErr> for InstallErr {
    fn from(e: CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<FileSysErr> for InstallErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<HTTPErr> for InstallErr {
    fn from(e: HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<StorageErr> for InstallErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

crate::impl_error!(InstallErr {
    AuthnErr,
    CryptErr,
    FileSysErr,
    HTTPErr,
    StorageErr,
});
