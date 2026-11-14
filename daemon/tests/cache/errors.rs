// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_daemon::cache::errors::CacheElementNotFound;
use miru_daemon::cache::CacheErr;
use miru_daemon::errors::Code;
use miru_daemon::filesys::errors::InvalidDirNameErr;
use miru_daemon::filesys::FileSysErr;

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_daemon::trace!(),
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

mod cache_element_not_found {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn cache_element_not_found_error_trait_surface() {
        let err = CacheElementNotFound {
            msg: "test".to_string(),
            trace: miru_daemon::trace!(),
        };
        assert_error(
            &err,
            Expected::new(Code::ResourceNotFound, StatusCode::NOT_FOUND),
        );
    }
}
