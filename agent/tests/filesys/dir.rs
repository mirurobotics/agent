// standard crates
use std::{env, path::PathBuf};

// internal crates
use miru_agent::filesys::{self, FileSysErr, PathExt};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod display {
    use super::*;

    #[test]
    fn absolute_path() {
        let dir = filesys::Dir::new(PathBuf::from("/tmp").join("test-dir"));
        assert_eq!(dir.path(), &PathBuf::from("/tmp").join("test-dir"));
    }

    #[test]
    fn relative_path() {
        let dir = filesys::Dir::new(PathBuf::from("relative").join("dir"));
        assert_eq!(dir.path(), &PathBuf::from("relative").join("dir"));
    }
}

pub mod name {
    use super::*;

    pub mod success {
        use super::*;

        #[test]
        fn basic_names() {
            let dir = filesys::Dir::new(PathBuf::from("lebron").join("james"));
            assert_eq!(dir.name().unwrap(), "james");

            let dir = filesys::Dir::new(PathBuf::from("lebron").join("james").join(""));
            assert_eq!(dir.name().unwrap(), "james");
        }

        #[test]
        fn with_special_characters() {
            let dir = filesys::Dir::new(PathBuf::from("path").join("my-dir_123"));
            assert_eq!(dir.name().unwrap(), "my-dir_123");

            let dir = filesys::Dir::new(PathBuf::from("path").join("dir.with.dots"));
            assert_eq!(dir.name().unwrap(), "dir.with.dots");

            let dir = filesys::Dir::new(PathBuf::from("path").join("dir with spaces"));
            assert_eq!(dir.name().unwrap(), "dir with spaces");
        }

        #[test]
        fn with_unicode() {
            let dir = filesys::Dir::new(PathBuf::from("path").join("目录"));
            assert_eq!(dir.name().unwrap(), "目录");

            let dir = filesys::Dir::new(PathBuf::from("path").join("привет"));
            assert_eq!(dir.name().unwrap(), "привет");

            let dir = filesys::Dir::new(PathBuf::from("path").join("🦀"));
            assert_eq!(dir.name().unwrap(), "🦀");
        }
    }

    pub mod failure {
        use super::*;

        #[test]
        fn root_directory() {
            let dir = filesys::Dir::new(PathBuf::from("/"));
            assert!(matches!(
                dir.name().unwrap_err(),
                FileSysErr::UnknownDirNameErr { .. }
            ));
        }

        #[test]
        fn empty_path() {
            let dir = filesys::Dir::new("");
            assert!(matches!(
                dir.name().unwrap_err(),
                FileSysErr::UnknownDirNameErr { .. }
            ));
        }
    }
}

pub mod parent {
    use super::*;

    pub mod success {
        use super::*;

        #[test]
        fn simple() {
            let dir = filesys::Dir::new(PathBuf::from("path").join("dir"));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "path");
        }

        #[test]
        fn with_trailing_separator() {
            let dir = filesys::Dir::new(PathBuf::from("path").join("dir").join(""));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "path");
        }

        #[test]
        fn with_trailing_separator_and_dot() {
            let dir = filesys::Dir::new(PathBuf::from("path").join("dir").join("."));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "path");
        }

        #[test]
        fn with_trailing_separator_and_dot_dot() {
            let dir =
                filesys::Dir::new(PathBuf::from("bronny").join("james").join("jr").join(".."));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "bronny");
        }

        #[test]
        fn empty_path() {
            let dir = filesys::Dir::new("");

            let current_dir_path = env::current_dir().unwrap();
            let expected = current_dir_path.parent().unwrap();
            assert_eq!(dir.parent().unwrap().path(), expected);
        }
    }

    pub mod failure {
        use super::*;

        #[test]
        fn root_directory() {
            let dir = filesys::Dir::new(PathBuf::from("/"));
            assert!(matches!(
                dir.parent().unwrap_err(),
                FileSysErr::UnknownParentDirForDirErr { .. }
            ));
        }
    }
}

pub mod valid_dir_name {
    use super::*;

    pub mod success {
        use super::*;

        #[test]
        fn basic() {
            let dir_name = "is_valid_dir_name";
            assert!(filesys::Dir::is_valid_dir_name(dir_name));
            filesys::Dir::assert_valid_dir_name(dir_name).unwrap();
        }

        #[test]
        fn exact_max_length() {
            let dir_name = "a".repeat(255); // A string with 255 characters
            assert!(filesys::Dir::is_valid_dir_name(&dir_name));
            filesys::Dir::assert_valid_dir_name(&dir_name).unwrap();
        }

        #[test]
        fn contains_special_characters() {
            let special_chars = "!@#$%^&*()";
            for special_char in special_chars.chars() {
                let dir_name = format!("is_valid_dir_name{special_char}");
                assert!(filesys::Dir::is_valid_dir_name(&dir_name));
                filesys::Dir::assert_valid_dir_name(&dir_name).unwrap();
            }
        }

        #[test]
        fn contains_leading_trailing_spaces() {
            let dir_name = "  is_valid_dir_name  ";
            assert!(filesys::Dir::is_valid_dir_name(dir_name));
            filesys::Dir::assert_valid_dir_name(dir_name).unwrap();
        }
    }

    pub mod failure {
        use super::*;

        #[test]
        fn empty_string() {
            let dir_name = "";
            assert!(!filesys::Dir::is_valid_dir_name(dir_name));
            assert!(matches!(
                filesys::Dir::assert_valid_dir_name(dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }

        #[test]
        fn contains_slash() {
            let dir_name = "invalid/dir_name";
            assert!(!filesys::Dir::is_valid_dir_name(dir_name));
            assert!(matches!(
                filesys::Dir::assert_valid_dir_name(dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }

        #[test]
        fn contains_null_byte() {
            let dir_name = "invalid\0dir_name";
            assert!(!filesys::Dir::is_valid_dir_name(dir_name));
            assert!(matches!(
                filesys::Dir::assert_valid_dir_name(dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }

        #[test]
        fn exceeds_max_length() {
            let dir_name = "a".repeat(256); // A string with 256 characters
            assert!(!filesys::Dir::is_valid_dir_name(&dir_name));
            assert!(matches!(
                filesys::Dir::assert_valid_dir_name(&dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }
    }
}

mod subdir {
    use super::*;

    #[test]
    fn basic() {
        let dir = filesys::Dir::new(PathBuf::from("path").join("dir"));
        let subdir = dir.subdir(PathBuf::from("subdir"));
        assert_eq!(subdir.path(), &dir.path().join("subdir"));
        assert_eq!(subdir.name().unwrap(), "subdir");
    }

    #[test]
    fn nested_subdir_leading_slash() {
        let base_path = PathBuf::from("base").join("path");
        let base_dir = filesys::Dir::new(base_path);
        let nested_path = PathBuf::from("root").join("subdir");
        let nested = base_dir.subdir(nested_path);

        let expected_path = PathBuf::from("base")
            .join("path")
            .join("root")
            .join("subdir");
        assert_eq!(nested.path(), &expected_path);
        assert_eq!(nested.name().unwrap(), "subdir");
    }

    #[test]
    fn nested_subdirs() {
        let base_path = PathBuf::from("base").join("path");
        let base_dir = filesys::Dir::new(base_path);
        let nested = base_dir.subdir("level1").subdir("level2").subdir("level3");

        let expected_path = PathBuf::from("base")
            .join("path")
            .join("level1")
            .join("level2")
            .join("level3");
        assert_eq!(nested.path(), &expected_path);
        assert_eq!(nested.name().unwrap(), "level3");
    }

    #[test]
    fn with_spaces() {
        let dir = filesys::Dir::new(PathBuf::from("test"));
        let subdir = dir.subdir("space folder");
        assert_eq!(subdir.path(), &PathBuf::from("test").join("space folder"));
        assert_eq!(subdir.name().unwrap(), "space folder");

        let subdir = dir.subdir("hyphen-folder");
        assert_eq!(subdir.path(), &PathBuf::from("test").join("hyphen-folder"));
        assert_eq!(subdir.name().unwrap(), "hyphen-folder");
    }

    #[test]
    fn with_empty_path() {
        let dir = filesys::Dir::new(PathBuf::from("test"));
        let subdir = dir.subdir("");
        assert_eq!(subdir.path(), &PathBuf::from("test").join(""));
    }

    #[test]
    fn with_absolute_path_component() {
        let dir = filesys::Dir::new(PathBuf::from("test"));
        let path_component = PathBuf::from("absolute").join("path");
        let subdir = dir.subdir(path_component);
        assert_eq!(
            subdir.path(),
            &PathBuf::from("test").join("absolute").join("path")
        );
    }

    #[test]
    fn with_dot_paths() {
        let dir = filesys::Dir::new(PathBuf::from("test"));
        let subdir = dir.subdir(".");
        assert_eq!(subdir.path(), &PathBuf::from("test").join("."));

        let subdir = dir.subdir("..");
        assert_eq!(subdir.path(), &PathBuf::from("test").join(".."));
    }
}

mod file {
    use super::*;

    #[test]
    fn filename_with_leading_slash() {
        let dir_path = PathBuf::from("tmp").join("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());
        let file = dir.file("/root/test.txt");
        assert_eq!(file.path(), &dir_path.join("root").join("test.txt"));
    }

    #[test]
    fn abs_dir() {
        let dir_path = PathBuf::from("tmp").join("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());
        let file = dir.file("test.txt");
        assert_eq!(file.path(), &dir_path.join("test.txt"));
    }

    #[test]
    fn nested_file_path() {
        let dir_path = PathBuf::from("base").join("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());
        let file = dir.file("nested/folder/test.txt");
        assert_eq!(
            file.path(),
            &dir_path.join("nested").join("folder").join("test.txt")
        );
    }

    #[test]
    fn special_characters() {
        let dir_path = PathBuf::from("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());

        // Test spaces in filename
        let file = dir.file("my file.txt");
        assert_eq!(file.path(), &dir_path.join("my file.txt"));

        // Test dots in filename
        let file = dir.file("test.multiple.dots.txt");
        assert_eq!(file.path(), &dir_path.join("test.multiple.dots.txt"));

        // Test hyphens and underscores
        let file = dir.file("my-file_name.txt");
        assert_eq!(file.path(), &dir_path.join("my-file_name.txt"));
    }

    #[test]
    fn empty_filename() {
        let dir_path = PathBuf::from("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());
        let file = dir.file("");
        assert_eq!(file.path(), &dir_path.join(""));
    }

    #[test]
    fn with_different_extensions() {
        let dir_path = PathBuf::from("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());

        // No extension
        let file = dir.file("filename");
        assert_eq!(file.path(), &dir_path.join("filename"));

        // Common extensions
        let file = dir.file("image.png");
        assert_eq!(file.path(), &dir_path.join("image.png"));

        // Hidden file (Unix-style)
        let file = dir.file(".hidden");
        assert_eq!(file.path(), &dir_path.join(".hidden"));
    }

    #[test]
    fn with_unicode_filename() {
        let dir_path = PathBuf::from("test_dir");
        let dir = filesys::Dir::new(dir_path.clone());

        let file = dir.file("文件.txt");
        assert_eq!(file.path(), &dir_path.join("文件.txt"));

        let file = dir.file("🦀rust.rs");
        assert_eq!(file.path(), &dir_path.join("🦀rust.rs"));
    }
}
