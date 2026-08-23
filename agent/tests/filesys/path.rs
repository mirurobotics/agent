// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::{self, dirs, files, path, Atomic, Overwrite, PathExt, WriteOptions};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod exists {
    use super::*;

    #[tokio::test]
    async fn existing_path() {
        let dir = dirs::temp("testing").unwrap();
        assert!(dir.exists());
    }

    #[test]
    fn nonexistent_path() {
        let dir = filesys::Dir::new(PathBuf::from("/nonexistent/path/abc123"));
        assert!(!dir.exists());
    }
}

pub mod try_exists {
    use super::*;

    // standard crates
    use std::os::unix::fs::PermissionsExt;

    // internal crates
    use miru_agent::filesys::FileSysErr;

    #[tokio::test]
    async fn returns_true_for_existing_file() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.to_dir().file("present.txt");
        files::seed(&file, "contents").await;

        assert!(file.try_exists().unwrap());
    }

    #[tokio::test]
    async fn returns_false_for_missing_file_in_existing_dir() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.to_dir().file("absent.txt");

        assert!(!file.try_exists().unwrap());
    }

    #[tokio::test]
    async fn returns_false_when_parent_dir_does_not_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.to_dir().subdir("nope").file("absent.txt");

        assert!(!file.try_exists().unwrap());
    }

    #[tokio::test]
    async fn returns_err_when_parent_dir_is_unreadable() {
        // root ignores permission bits, so the error cannot be provoked
        if nix::unistd::geteuid().is_root() {
            eprintln!("skipping: running as root, permission bits are bypassed");
            return;
        }

        let tmp = dirs::temp("testing").unwrap();
        let locked = tmp.to_dir().subdir("locked");
        dirs::create(&locked).await.unwrap();
        let file = locked.file("secret.txt");
        files::seed(&file, "contents").await;

        dirs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();
        let result = file.try_exists();
        // restore before asserting so a failure cannot leak an unreadable dir
        dirs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        assert!(
            matches!(result, Err(FileSysErr::PathExistenceErr(_))),
            "expected PathExistenceErr, got {result:?}"
        );
    }
}

pub mod write_options {
    use super::*;

    #[test]
    fn default() {
        let opts = WriteOptions::default();
        assert_eq!(opts.overwrite, Overwrite::Deny);
        assert_eq!(opts.atomic, Atomic::Yes);
    }

    #[test]
    fn overwrite_atomic() {
        assert_eq!(WriteOptions::OVERWRITE_ATOMIC.overwrite, Overwrite::Allow);
        assert_eq!(WriteOptions::OVERWRITE_ATOMIC.atomic, Atomic::Yes);
    }

    #[test]
    fn overwrite() {
        assert_eq!(
            WriteOptions::OVERWRITE_NONATOMIC.overwrite,
            Overwrite::Allow
        );
        assert_eq!(WriteOptions::OVERWRITE_NONATOMIC.atomic, Atomic::No);
    }
}

pub mod abs_path {
    // test cases taken from path-clean crate https://github.com/danreeves/path-clean/blob/3876d7cb5367997bcda17ce165bf69c4f434cb93/src/lib.rs#L108
    use super::*;

    #[test]
    fn empty_path_is_current_dir() {
        let dir = filesys::Dir::new(PathBuf::from(""));
        let expected_dir = dirs::current().unwrap();
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
            let dir = filesys::Dir::new(PathBuf::from(test.0));
            assert_eq!(dir.abs_path().unwrap(), PathBuf::from(test.1));
        }
    }

    #[test]
    fn replace_multiple_slashes() {
        let current_dir = dirs::current().unwrap();
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
            let dir = filesys::Dir::new(PathBuf::from(test.0));
            assert_eq!(dir.abs_path().unwrap(), PathBuf::from(test.1));
        }
    }

    #[test]
    fn eliminate_current_dir() {
        let current_dir = dirs::current().unwrap();
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
                filesys::Dir::new(PathBuf::from(test.0)).abs_path().unwrap(),
                PathBuf::from(test.1)
            );
        }
    }

    #[test]
    fn eliminate_parent_dir() {
        let current_dir = dirs::current().unwrap();
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
                filesys::Dir::new(PathBuf::from(test.0)).abs_path().unwrap(),
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
