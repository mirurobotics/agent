use miru_agent::cache::errors::{CacheElementNotFound, CacheErr};
use miru_agent::crypt::errors::{CryptErr, InvalidJWTErr};
use miru_agent::filesys::errors::{FileSysErr, InvalidDirNameErr};
use miru_agent::storage::errors::StorageErr;

fn cache_err() -> CacheErr {
    CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: "cache miss".to_string(),
        trace: miru_agent::trace!(),
    })
}

fn crypt_err() -> CryptErr {
    CryptErr::InvalidJWTErr(InvalidJWTErr {
        msg: "invalid token".to_string(),
        trace: miru_agent::trace!(),
    })
}

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_agent::trace!(),
    })
}

mod from_conversions {
    use super::*;

    #[test]
    fn cache_err_maps_to_storage_cache_err() {
        let err: StorageErr = cache_err().into();
        assert!(matches!(err, StorageErr::CacheErr(_)));
    }

    #[test]
    fn crypt_err_maps_to_storage_crypt_err() {
        let err: StorageErr = crypt_err().into();
        assert!(matches!(err, StorageErr::CryptErr(_)));
    }

    #[test]
    fn filesys_err_maps_to_storage_filesys_err() {
        let err: StorageErr = filesys_err().into();
        assert!(matches!(err, StorageErr::FileSysErr(_)));
    }
}
