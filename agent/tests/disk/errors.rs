// internal crates
use miru_agent::cache::errors::CacheElementNotFound;
use miru_agent::cache::CacheErr;
use miru_agent::crypt::errors::InvalidJWTErr;
use miru_agent::crypt::CryptErr;
use miru_agent::disk::errors::ResolveDeviceIDErr;
use miru_agent::disk::DiskErr;
use miru_agent::filesys::errors::InvalidDirNameErr;
use miru_agent::filesys::FileSysErr;

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
        let err: DiskErr = cache_err().into();
        assert!(matches!(err, DiskErr::CacheErr(_)));
    }

    #[test]
    fn crypt_err_maps_to_storage_crypt_err() {
        let err: DiskErr = crypt_err().into();
        assert!(matches!(err, DiskErr::CryptErr(_)));
    }

    #[test]
    fn filesys_err_maps_to_storage_filesys_err() {
        let err: DiskErr = filesys_err().into();
        assert!(matches!(err, DiskErr::FileSysErr(_)));
    }
}

mod display {
    use super::*;

    #[test]
    fn resolve_device_id_err_includes_both_sources() {
        let err = ResolveDeviceIDErr {
            device_file_err: Box::new(filesys_err()),
            jwt_err: Box::new(crypt_err()),
            trace: miru_agent::trace!(),
        };
        let rendered = format!("{err}");
        assert!(rendered.contains("device file error"));
        assert!(rendered.contains("jwt error"));
        // confirm both source error displays are interpolated
        assert!(rendered.contains(&format!("{}", filesys_err())));
        assert!(rendered.contains(&format!("{}", crypt_err())));

        // ensure the wrapped variant routes through Display via transparent
        let storage: DiskErr = DiskErr::ResolveDeviceIDErr(Box::new(ResolveDeviceIDErr {
            device_file_err: Box::new(filesys_err()),
            jwt_err: Box::new(crypt_err()),
            trace: miru_agent::trace!(),
        }));
        let storage_rendered = format!("{storage}");
        assert!(storage_rendered.contains("device file error"));
    }
}
