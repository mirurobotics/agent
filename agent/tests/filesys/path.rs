// standard crates
use std::path::{Path, PathBuf};

// internal crates
use crate::test_utils::filesys::{dirs as test_dirs, files as test_files};
use miru_agent::filesys::{self, dirs, path, Atomic, FileSysErr, Overwrite, PathExt, WriteOptions};

pub mod exists {
    use super::*;

    #[tokio::test]
    async fn existing_path() {
        let dir = test_dirs::temp("testing").unwrap();
        assert!(dir.exists());
    }

    #[test]
    fn nonexistent_path() {
        let dir = filesys::Dir::new(PathBuf::from("nonexistent").join("path").join("abc123"));
        assert!(!dir.exists());
    }
}

pub mod try_exists {
    use super::*;

    #[tokio::test]
    async fn returns_true_for_existing_file() {
        let dir = test_dirs::temp("testing").unwrap();
        let file = dir.to_dir().file("present.txt");
        test_files::seed(&file, "contents").await;

        assert!(file.try_exists().unwrap());
    }

    #[tokio::test]
    async fn returns_false_for_missing_file_in_existing_dir() {
        let dir = test_dirs::temp("testing").unwrap();
        let file = dir.to_dir().file("absent.txt");

        assert!(!file.try_exists().unwrap());
    }

    #[tokio::test]
    async fn returns_false_when_parent_dir_does_not_exist() {
        let dir = test_dirs::temp("testing").unwrap();
        let file = dir.to_dir().subdir("nope").file("absent.txt");

        assert!(!file.try_exists().unwrap());
    }

    #[test]
    fn returns_err_when_path_is_invalid() {
        // An embedded NUL makes metadata fail with InvalidInput on every
        // platform. chmod 000 is Unix-only, and a file occupying the parent
        // is NotFound on Windows.
        let result = filesys::File::new("invalid\0path").try_exists();
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
    // Relative cases from path-clean
    // (https://github.com/danreeves/path-clean). Rooted `/` and `\\` tables
    // stay per-OS: a `/` literal is not absolute on Windows.
    use super::*;

    fn current() -> filesys::Dir {
        dirs::current().unwrap()
    }

    fn host_root() -> PathBuf {
        current().path().ancestors().last().unwrap().to_path_buf()
    }

    fn abs(input: impl AsRef<Path>) -> PathBuf {
        filesys::Dir::new(input.as_ref()).abs_path().unwrap()
    }

    #[test]
    fn empty_path_is_current_dir() {
        assert_eq!(&abs(""), current().path());
    }

    #[test]
    fn abs_paths_dont_change() {
        let root = host_root();
        for expected in [
            root.clone(),
            root.join("another").join("one"),
            root.join("bronny").join("james"),
        ] {
            assert_eq!(abs(&expected), expected);
        }
    }

    #[test]
    fn replace_multiple_slashes() {
        let cwd = current();
        let cwd_path = cwd.path();
        assert_eq!(abs(".//"), cwd_path.clone());
        assert_eq!(abs("..//"), path::clean(cwd.parent().unwrap().path()));
        assert_eq!(abs("././/./"), cwd_path.clone());
        assert_eq!(abs("path//to///thing"), cwd_path.join("path/to/thing"));
    }

    #[test]
    fn eliminate_current_dir() {
        let cwd = current().path().clone();
        assert_eq!(abs("./"), cwd);
        assert_eq!(abs("./test"), cwd.join("test"));
        assert_eq!(abs("./test/./path"), cwd.join("test/path"));
        assert_eq!(abs("test/path/."), cwd.join("test/path"));
    }

    #[test]
    fn eliminate_parent_dir() {
        let cwd = current();
        let cwd_path = cwd.path();
        let parent = path::clean(cwd.parent().unwrap().path());
        let grandparent = path::clean(cwd.parent().unwrap().parent().unwrap().path());

        assert_eq!(abs("test/.."), cwd_path.clone());
        assert_eq!(abs("test/path/.."), cwd_path.join("test"));
        assert_eq!(abs("test/../path"), cwd_path.join("path"));
        assert_eq!(abs("test/path/../../"), cwd_path.clone());
        assert_eq!(abs("test/path/../../.."), parent.clone());
        assert_eq!(abs("test/path/../../../.."), grandparent);
        assert_eq!(
            abs("test/path/../../another/path"),
            cwd_path.join("another/path")
        );
        assert_eq!(
            abs("test/path/../../another/path/.."),
            cwd_path.join("another")
        );
        assert_eq!(abs("../test"), parent.join("test"));
        assert_eq!(abs("../test/"), parent.join("test"));
        assert_eq!(abs("../test/path"), parent.join("test/path"));
        assert_eq!(abs("../test/.."), parent);
    }

    #[cfg(unix)]
    #[test]
    fn unix_rooted_abs_paths() {
        let cases = [
            ("/", "/"),
            ("//", "/"),
            ("///", "/"),
            ("//..", "/"),
            ("/..//", "/"),
            ("/.//./", "/"),
            ("/path//to///thing", "/path/to/thing"),
            ("/./", "/"),
            ("/test/./path/", "/test/path"),
            ("/..", "/"),
            ("/../test", "/test"),
            ("/test/../path", "/path"),
            ("/test/path/../../..", "/"),
            ("/test/path/../../../..", "/"),
        ];
        for (input, expected) in cases {
            assert_eq!(abs(input), PathBuf::from(expected));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_rooted_abs_paths() {
        let root = host_root();
        let root_str = root.to_string_lossy();
        let cases = [
            (format!(r"{root_str}\\"), root.clone()),
            (format!(r"{root_str}.\\.\"), root.clone()),
            (
                format!(r"{root_str}path\\to\.\thing\..\file"),
                root.join("path").join("to").join("file"),
            ),
            (format!(r"{root_str}..\\..\path"), root.join("path")),
        ];
        for (input, expected) in cases {
            assert_eq!(abs(input), expected);
        }
    }
}

pub mod clean {
    // Relative cases from path-clean
    // (https://github.com/danreeves/path-clean). `/`-rooted literals are
    // Unix path syntax (`//` is a UNC prefix on Windows), so those tables
    // stay unix-gated.
    use super::*;

    fn cleaned(input: &str) -> PathBuf {
        path::clean(PathBuf::from(input))
    }

    #[test]
    fn empty_path_is_current_dir() {
        assert_eq!(cleaned(""), PathBuf::from("."));
    }

    #[test]
    fn clean_paths_dont_change() {
        for input in [".", ".."] {
            assert_eq!(cleaned(input), PathBuf::from(input));
        }
    }

    #[test]
    fn replace_multiple_slashes() {
        let cases = [
            (".//", "."),
            ("..//", ".."),
            ("././/./", "."),
            ("path//to///thing", "path/to/thing"),
        ];
        for (input, expected) in cases {
            assert_eq!(cleaned(input), PathBuf::from(expected));
        }
    }

    #[test]
    fn eliminate_current_dir() {
        let cases = [
            ("./", "."),
            ("./test", "test"),
            ("./test/./path", "test/path"),
            ("test/path/.", "test/path"),
        ];
        for (input, expected) in cases {
            assert_eq!(cleaned(input), PathBuf::from(expected));
        }
    }

    #[test]
    fn eliminate_parent_dir() {
        let cases = [
            ("test/..", "."),
            ("test/path/..", "test"),
            ("test/../path", "path"),
            ("test/path/../../", "."),
            ("test/path/../../..", ".."),
            ("test/path/../../../..", "../.."),
            ("test/path/../../another/path", "another/path"),
            ("test/path/../../another/path/..", "another"),
            ("../test", "../test"),
            ("../test/", "../test"),
            ("../test/path", "../test/path"),
            ("../test/..", ".."),
        ];
        for (input, expected) in cases {
            assert_eq!(cleaned(input), PathBuf::from(expected));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_rooted_paths() {
        let cases = [
            ("/", "/"),
            ("//", "/"),
            ("///", "/"),
            ("//..", "/"),
            ("/..//", "/"),
            ("/.//./", "/"),
            ("/path//to///thing", "/path/to/thing"),
            ("/./", "/"),
            ("/test/./path/", "/test/path"),
            ("/..", "/"),
            ("/../test", "/test"),
            ("/test/../path", "/path"),
            ("/test/path/../../..", "/"),
            ("/test/path/../../../..", "/"),
        ];
        for (input, expected) in cases {
            assert_eq!(cleaned(input), PathBuf::from(expected));
        }
    }
}
