use crate::authn::errors::AuthnErr;
use crate::crypt::errors::CryptErr;
use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;
use crate::storage::errors::StorageErr;

#[derive(Debug, thiserror::Error)]
#[error("Missing environment variable: {name}")]
pub struct MissingEnvVarErr {
    pub name: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MissingEnvVarErr {}

#[derive(Debug, thiserror::Error)]
pub enum InstallErr {
    #[error(transparent)]
    MissingEnvVarErr(MissingEnvVarErr),
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
    MissingEnvVarErr,
    AuthnErr,
    CryptErr,
    FileSysErr,
    HTTPErr,
    StorageErr,
});

#[cfg(test)]
mod tests {
    use super::*;

    mod from_conversions {
        use super::*;

        #[test]
        fn from_authn_err() {
            let err =
                AuthnErr::TimestampConversionErr(crate::authn::errors::TimestampConversionErr {
                    msg: "test".to_string(),
                    trace: crate::trace!(),
                });
            let install_err = InstallErr::from(err);
            assert!(matches!(install_err, InstallErr::AuthnErr(_)));
        }

        #[test]
        fn from_crypt_err() {
            let err = CryptErr::InvalidJWTErr(crate::crypt::errors::InvalidJWTErr {
                msg: "test".to_string(),
                trace: crate::trace!(),
            });
            let install_err = InstallErr::from(err);
            assert!(matches!(install_err, InstallErr::CryptErr(_)));
        }

        #[test]
        fn from_filesys_err() {
            let err = FileSysErr::InvalidDirNameErr(crate::filesys::errors::InvalidDirNameErr {
                name: "test".to_string(),
                trace: crate::trace!(),
            });
            let install_err = InstallErr::from(err);
            assert!(matches!(install_err, InstallErr::FileSysErr(_)));
        }

        #[test]
        fn from_http_err() {
            let err = HTTPErr::MockErr(crate::http::errors::MockErr {
                is_network_connection_error: false,
            });
            let install_err = InstallErr::from(err);
            assert!(matches!(install_err, InstallErr::HTTPErr(_)));
        }

        #[test]
        fn from_storage_err() {
            let err =
                StorageErr::DeviceNotActivatedErr(crate::storage::errors::DeviceNotActivatedErr {
                    msg: "test".to_string(),
                    trace: crate::trace!(),
                });
            let install_err = InstallErr::from(err);
            assert!(matches!(install_err, InstallErr::StorageErr(_)));
        }
    }
}
