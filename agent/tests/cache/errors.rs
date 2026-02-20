use miru_agent::cache::errors::CacheErr;
use miru_agent::filesys::errors::{FileSysErr, InvalidDirNameErr};

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_agent::trace!(),
    })
}

mod from_conversions {
    use super::*;

    #[test]
    fn filesys_err_maps_to_cache_filesys_err() {
        let err: CacheErr = filesys_err().into();
        assert!(matches!(err, CacheErr::FileSysErr(_)));
    }
}
