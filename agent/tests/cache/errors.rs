use miru_agent::cache::errors::{CacheElementNotFound, CacheErr};
use miru_agent::errors::Error;
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

mod cache_element_not_found {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn returns_resource_not_found_code() {
        let err = CacheElementNotFound {
            msg: "test".to_string(),
            trace: miru_agent::trace!(),
        };
        assert_eq!(err.code().as_str(), "resource_not_found");
    }

    #[test]
    fn returns_404_status() {
        let err = CacheElementNotFound {
            msg: "test".to_string(),
            trace: miru_agent::trace!(),
        };
        assert_eq!(err.http_status(), StatusCode::NOT_FOUND);
    }
}
