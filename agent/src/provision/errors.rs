// internal crates
use crate::authn;
use crate::crypt;
use crate::errors::Trace;
use crate::filesys;
use crate::http;
use crate::logs;
use crate::storage::StorageErr;

#[derive(Debug, thiserror::Error)]
#[error("Missing environment variable: {name}")]
pub struct MissingEnvVarErr {
    pub name: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MissingEnvVarErr {}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionErr {
    #[error(transparent)]
    MissingEnvVarErr(MissingEnvVarErr),
    #[error(transparent)]
    AuthnErr(authn::AuthnErr),
    #[error(transparent)]
    CryptErr(crypt::CryptErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    HTTPErr(http::HTTPErr),
    #[error(transparent)]
    LogsErr(logs::LogsErr),
    #[error(transparent)]
    StorageErr(StorageErr),
}

impl From<authn::AuthnErr> for ProvisionErr {
    fn from(e: authn::AuthnErr) -> Self {
        Self::AuthnErr(e)
    }
}

impl From<crypt::CryptErr> for ProvisionErr {
    fn from(e: crypt::CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<filesys::FileSysErr> for ProvisionErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<http::HTTPErr> for ProvisionErr {
    fn from(e: http::HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<logs::LogsErr> for ProvisionErr {
    fn from(e: logs::LogsErr) -> Self {
        Self::LogsErr(e)
    }
}

impl From<StorageErr> for ProvisionErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

crate::impl_error!(ProvisionErr {
    MissingEnvVarErr,
    AuthnErr,
    CryptErr,
    FileSysErr,
    HTTPErr,
    LogsErr,
    StorageErr,
});

#[cfg(test)]
mod tests {
    use super::*;

    mod from_conversions {
        use super::*;

        #[test]
        fn from_authn_err() {
            let err = authn::AuthnErr::TimestampConversionErr(
                crate::authn::errors::TimestampConversionErr {
                    msg: "test".to_string(),
                    trace: crate::trace!(),
                },
            );
            let install_err = ProvisionErr::from(err);
            assert!(matches!(install_err, ProvisionErr::AuthnErr(_)));
        }

        #[test]
        fn from_crypt_err() {
            let err = crypt::CryptErr::InvalidJWTErr(crate::crypt::errors::InvalidJWTErr {
                msg: "test".to_string(),
                trace: crate::trace!(),
            });
            let install_err = ProvisionErr::from(err);
            assert!(matches!(install_err, ProvisionErr::CryptErr(_)));
        }

        #[test]
        fn from_filesys_err() {
            let err =
                filesys::FileSysErr::InvalidDirNameErr(crate::filesys::errors::InvalidDirNameErr {
                    name: "test".to_string(),
                    trace: crate::trace!(),
                });
            let install_err = ProvisionErr::from(err);
            assert!(matches!(install_err, ProvisionErr::FileSysErr(_)));
        }

        #[test]
        fn from_http_err() {
            let err = http::HTTPErr::MockErr(crate::http::errors::MockErr {
                is_network_conn_err: false,
            });
            let install_err = ProvisionErr::from(err);
            assert!(matches!(install_err, ProvisionErr::HTTPErr(_)));
        }

        #[test]
        fn from_storage_err() {
            let err = StorageErr::DeviceNotActivatedErr(crate::storage::DeviceNotActivatedErr {
                msg: "test".to_string(),
                trace: crate::trace!(),
            });
            let install_err = ProvisionErr::from(err);
            assert!(matches!(install_err, ProvisionErr::StorageErr(_)));
        }

        #[test]
        fn from_logs_err() {
            let err = logs::LogsErr::ReloadFailed("test".to_string());
            let install_err = ProvisionErr::from(err);
            assert!(matches!(install_err, ProvisionErr::LogsErr(_)));
        }
    }
}
