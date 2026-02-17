// internal crates
use miru_agent::filesys::{dir::Dir, path, path::PathExt, Atomic, Overwrite, WriteOptions};
// external crates
use std::path::PathBuf;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod exists {
    use super::*;

    #[tokio::test]
    async fn existing_path() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert!(dir.exists());
    }

    #[test]
    fn nonexistent_path() {
        let dir = Dir::new(PathBuf::from("/nonexistent/path/abc123"));
        assert!(!dir.exists());
    }
}

pub mod write_options {
    use super::*;

    #[test]
    fn default() {
        let opts = WriteOptions::default();
        assert_eq!(opts.overwrite, Overwrite::Deny);
        assert_eq!(opts.atomic, Atomic::No);
    }

    #[test]
    fn overwrite_atomic() {
        assert_eq!(WriteOptions::OVERWRITE_ATOMIC.overwrite, Overwrite::Allow);
        assert_eq!(WriteOptions::OVERWRITE_ATOMIC.atomic, Atomic::Yes);
    }

    #[test]
    fn overwrite() {
        assert_eq!(WriteOptions::OVERWRITE.overwrite, Overwrite::Allow);
        assert_eq!(WriteOptions::OVERWRITE.atomic, Atomic::No);
    }
}

pub mod abs_path {
    // test cases taken from path-clean crate https://github.com/danreeves/path-clean/blob/3876d7cb5367997bcda17ce165bf69c4f434cb93/src/lib.rs#L108
    use super::*;

    #[test]
    fn empty_path_is_current_dir() {
        let dir = Dir::new(PathBuf::from(""));
        let expected_dir = Dir::new_current_dir().unwrap();
        assert_eq!(&dir.abs_path().unwrap(), expected_dir.path());
    }

    #[test]
    fn abs_paths_dont_change() {
        let tests = vec![
            ("/", "/"),
            ("/another/one", "/another/one"),
            ("/bronny/james", "/bronny/james"),
        ];

        for test in tests {
            let dir = Dir::new(PathBuf::from(test.0));
            assert_eq!(dir.abs_path().unwrap(), PathBuf::from(test.1));
        }
    }

    #[test]
    fn replace_multiple_slashes() {
        let current_dir = Dir::new_current_dir().unwrap();
        let current_dir_path_buf = current_dir.path();
        let current_dir_path = current_dir_path_buf.to_string_lossy().into_owned();
        let parent_dir_path = path::clean(current_dir.parent().unwrap().path())
            .to_string_lossy()
            .into_owned();

        let rel_path_to_thing_path = current_dir_path_buf
            .join("path/to/thing")
            .to_string_lossy()
            .into_owned();

        let tests = vec![
            ("/", "/"),
            ("//", "/"),
            ("///", "/"),
            (".//", &current_dir_path),
            ("//..", "/"),
            ("..//", &parent_dir_path),
            ("/..//", "/"),
            ("/.//./", "/"),
            ("././/./", &current_dir_path),
            ("path//to///thing", &rel_path_to_thing_path),
            ("/path//to///thing", "/path/to/thing"),
        ];

        for test in tests {
            let dir = Dir::new(PathBuf::from(test.0));
            assert_eq!(dir.abs_path().unwrap(), PathBuf::from(test.1));
        }
    }

    #[test]
    fn eliminate_current_dir() {
        let current_dir = Dir::new_current_dir().unwrap();
        let current_dir_path_buf = current_dir.path();
        let current_dir_path = current_dir_path_buf.to_string_lossy().into_owned();

        let rel_test_path = current_dir_path_buf
            .join("test")
            .to_string_lossy()
            .into_owned();
        let rel_test_path_path = current_dir_path_buf
            .join("test/path")
            .to_string_lossy()
            .into_owned();

        let tests = vec![
            ("/./", "/"),
            ("./", &current_dir_path),
            ("./test", &rel_test_path),
            ("./test/./path", &rel_test_path_path),
            ("/test/./path/", "/test/path"),
            ("test/path/.", &rel_test_path_path),
        ];

        for test in tests {
            assert_eq!(
                Dir::new(PathBuf::from(test.0)).abs_path().unwrap(),
                PathBuf::from(test.1)
            );
        }
    }

    #[test]
    fn eliminate_parent_dir() {
        let current_dir = Dir::new_current_dir().unwrap();
        let current_dir_path_buf = current_dir.path();
        let current_dir_path = current_dir_path_buf.to_string_lossy().into_owned();
        let parent_dir = current_dir.parent().unwrap();
        let parent_dir_path_buf = parent_dir.path();
        let parent_dir_path = path::clean(parent_dir_path_buf)
            .to_string_lossy()
            .into_owned();
        let grandparent_dir_path =
            path::clean(current_dir.parent().unwrap().parent().unwrap().path())
                .to_string_lossy()
                .into_owned();

        let rel_test_path = current_dir_path_buf
            .join("test")
            .to_string_lossy()
            .into_owned();
        let rel_path_path = current_dir_path_buf
            .join("path")
            .to_string_lossy()
            .into_owned();
        let rel_another_path = current_dir_path_buf
            .join("another")
            .to_string_lossy()
            .into_owned();
        let rel_another_path_path = current_dir_path_buf
            .join("another/path")
            .to_string_lossy()
            .into_owned();
        let rel_parent_test_path = parent_dir_path_buf
            .join("test")
            .to_string_lossy()
            .into_owned();
        let rel_parent_test_path_path = parent_dir_path_buf
            .join("test/path")
            .to_string_lossy()
            .into_owned();

        let tests = vec![
            ("/..", "/"),
            ("/../test", "/test"),
            ("test/..", &current_dir_path),
            ("test/path/..", &rel_test_path),
            ("test/../path", &rel_path_path),
            ("/test/../path", "/path"),
            ("test/path/../../", &current_dir_path),
            ("test/path/../../..", &parent_dir_path),
            ("/test/path/../../..", "/"),
            ("/test/path/../../../..", "/"),
            ("test/path/../../../..", &grandparent_dir_path),
            ("test/path/../../another/path", &rel_another_path_path),
            ("test/path/../../another/path/..", &rel_another_path),
            ("../test", &rel_parent_test_path),
            ("../test/", &rel_parent_test_path),
            ("../test/path", &rel_parent_test_path_path),
            ("../test/..", &parent_dir_path),
        ];

        for test in tests {
            assert_eq!(
                Dir::new(PathBuf::from(test.0)).abs_path().unwrap(),
                PathBuf::from(test.1)
            );
        }
    }
}

pub mod clean {
    // test cases taken from path-clean crate https://github.com/danreeves/path-clean/blob/3876d7cb5367997bcda17ce165bf69c4f434cb93/src/lib.rs#L108
    use super::*;

    #[test]
    fn empty_path_is_current_dir() {
        assert_eq!(path::clean(PathBuf::from("")), PathBuf::from("."));
    }

    #[test]
    fn clean_paths_dont_change() {
        let tests = vec![(".", "."), ("..", ".."), ("/", "/")];

        for test in tests {
            assert_eq!(path::clean(PathBuf::from(test.0)), PathBuf::from(test.1));
        }
    }

    #[test]
    fn replace_multiple_slashes() {
        let tests = vec![
            ("/", "/"),
            ("//", "/"),
            ("///", "/"),
            (".//", "."),
            ("//..", "/"),
            ("..//", ".."),
            ("/..//", "/"),
            ("/.//./", "/"),
            ("././/./", "."),
            ("path//to///thing", "path/to/thing"),
            ("/path//to///thing", "/path/to/thing"),
        ];

        for test in tests {
            assert_eq!(path::clean(PathBuf::from(test.0)), PathBuf::from(test.1));
        }
    }

    #[test]
    fn eliminate_current_dir() {
        let tests = vec![
            ("./", "."),
            ("/./", "/"),
            ("./test", "test"),
            ("./test/./path", "test/path"),
            ("/test/./path/", "/test/path"),
            ("test/path/.", "test/path"),
        ];

        for test in tests {
            assert_eq!(path::clean(PathBuf::from(test.0)), PathBuf::from(test.1));
        }
    }

    #[test]
    fn eliminate_parent_dir() {
        let tests = vec![
            ("/..", "/"),
            ("/../test", "/test"),
            ("test/..", "."),
            ("test/path/..", "test"),
            ("test/../path", "path"),
            ("/test/../path", "/path"),
            ("test/path/../../", "."),
            ("test/path/../../..", ".."),
            ("/test/path/../../..", "/"),
            ("/test/path/../../../..", "/"),
            ("test/path/../../../..", "../.."),
            ("test/path/../../another/path", "another/path"),
            ("test/path/../../another/path/..", "another"),
            ("../test", "../test"),
            ("../test/", "../test"),
            ("../test/path", "../test/path"),
            ("../test/..", ".."),
        ];

        for test in tests {
            assert_eq!(path::clean(PathBuf::from(test.0)), PathBuf::from(test.1));
        }
    }
}
