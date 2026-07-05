// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::{self, file, FileSysErr, PathExt};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod display {
    use super::*;

    #[test]
    fn absolute_path() {
        let file = filesys::File::new(PathBuf::from("/tmp").join("test-file.txt"));
        assert_eq!(file.path(), &PathBuf::from("/tmp").join("test-file.txt"));
    }

    #[test]
    fn relative_path() {
        let file = filesys::File::new(PathBuf::from("relative").join("path.txt"));
        assert_eq!(file.path(), &PathBuf::from("relative").join("path.txt"));
    }
}

pub mod new_normalization {
    use super::*;

    #[test]
    fn strips_dot_component() {
        assert_eq!(
            filesys::File::new(PathBuf::from("/a/./b")),
            filesys::File::new(PathBuf::from("/a/b")),
        );
    }

    #[test]
    fn strips_trailing_separator() {
        assert_eq!(
            filesys::File::new(PathBuf::from("/a/b/")),
            filesys::File::new(PathBuf::from("/a/b")),
        );
    }

    #[test]
    fn strips_dot_in_relative_path() {
        assert_eq!(
            filesys::File::new(PathBuf::from("relative/./path")),
            filesys::File::new(PathBuf::from("relative/path")),
        );
    }

    #[test]
    fn preserves_parent_dir_component() {
        // .. is NOT resolved — it is preserved as a component
        assert_ne!(
            filesys::File::new(PathBuf::from("/a/../b")),
            filesys::File::new(PathBuf::from("/b")),
        );
    }
}

pub mod is_absolute {
    use super::*;

    #[test]
    fn returns_true_for_absolute_path() {
        let f = filesys::File::new(PathBuf::from("/etc/foo.json"));
        assert!(f.is_absolute());
    }

    #[test]
    fn returns_false_for_relative_path() {
        let f = filesys::File::new(PathBuf::from("foo/bar.json"));
        assert!(!f.is_absolute());
    }
}

pub mod parent {
    use super::*;

    #[test]
    fn simple() {
        let file = filesys::File::new(PathBuf::from("tmp").join("some-dir").join("test-file.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("tmp").join("some-dir"));
    }

    #[test]
    fn nested() {
        let file = filesys::File::new(PathBuf::from("a").join("b").join("c").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b").join("c"));
    }

    #[test]
    fn trailing_separator() {
        let file = filesys::File::new(PathBuf::from("a").join("b").join("").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b"));
    }

    #[test]
    fn trailing_separator_and_dot() {
        let file = filesys::File::new(PathBuf::from("a").join("b").join(".").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b"));
    }

    #[test]
    fn trailing_separator_and_dot_dot() {
        let file = filesys::File::new(PathBuf::from("a").join("b").join("..").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b").join(".."));
    }

    #[test]
    fn root_file() {
        let file = filesys::File::new(PathBuf::from("/file.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("/"));
    }

    #[test]
    fn file_only() {
        let file = filesys::File::new(PathBuf::from("file.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from(""));
    }
}
pub mod name {
    use super::*;

    #[tokio::test]
    async fn basic_names() {
        let file = filesys::File::new(PathBuf::from("lebron").join("james.txt"));
        assert_eq!(file.name().unwrap(), "james.txt");

        let file = filesys::File::new(PathBuf::from("lebron").join("james.txt").join(""));
        assert_eq!(file.name().unwrap(), "james.txt");
    }

    #[tokio::test]
    async fn with_special_characters() {
        let file = filesys::File::new(PathBuf::from("path").join("my-file_123.txt"));
        assert_eq!(file.name().unwrap(), "my-file_123.txt");

        let file = filesys::File::new(PathBuf::from("path").join("file.with.dots.txt"));
        assert_eq!(file.name().unwrap(), "file.with.dots.txt");

        let file = filesys::File::new(PathBuf::from("path").join("file with spaces.txt"));
        assert_eq!(file.name().unwrap(), "file with spaces.txt");
    }

    #[tokio::test]
    async fn with_unicode() {
        let file = filesys::File::new(PathBuf::from("path").join("文件.txt"));
        assert_eq!(file.name().unwrap(), "文件.txt");

        let file = filesys::File::new(PathBuf::from("path").join("файл.txt"));
        assert_eq!(file.name().unwrap(), "файл.txt");

        let file = filesys::File::new(PathBuf::from("path").join("🦀.txt"));
        assert_eq!(file.name().unwrap(), "🦀.txt");
    }

    #[tokio::test]
    async fn root_path() {
        let file = filesys::File::new(PathBuf::from("/"));
        assert!(matches!(
            file.name().unwrap_err(),
            FileSysErr::UnknownFileNameErr { .. }
        ));
    }

    #[tokio::test]
    async fn empty_path() {
        let file = filesys::File::new(PathBuf::from(""));
        assert!(matches!(
            file.name().unwrap_err(),
            FileSysErr::UnknownFileNameErr { .. }
        ));
    }
}
mod sanitize_filename {
    use super::*;

    #[test]
    fn allowed_characters() {
        // alphabets
        assert_eq!(file::sanitize_filename("abcxyzABCXYZ"), "abcxyzABCXYZ");

        // numbers
        assert_eq!(file::sanitize_filename("0123456789"), "0123456789");

        // allowed special characters
        assert_eq!(
            file::sanitize_filename("test-file_name.txt"),
            "test-file_name.txt"
        );

        // mixed allowed characters
        assert_eq!(
            file::sanitize_filename("File-123_TEST.txt"),
            "File-123_TEST.txt"
        );
    }

    #[test]
    fn disallowed_characters() {
        // spaces
        assert_eq!(file::sanitize_filename("file name"), "file_name");

        // special characters
        assert_eq!(file::sanitize_filename("file@#$%^&*"), "file_______");

        // slashes
        assert_eq!(file::sanitize_filename("path/to/file"), "path_to_file");
        assert_eq!(file::sanitize_filename("path\\to\\file"), "path_to_file");

        // mixed special characters
        assert_eq!(
            file::sanitize_filename("my<>file:*?.txt"),
            "my__file___.txt"
        );
    }

    #[test]
    fn unicode_characters() {
        // emoji
        assert_eq!(file::sanitize_filename("hello😊world"), "hello_world");

        // accented characters
        assert_eq!(file::sanitize_filename("résumé.pdf"), "r_sum_.pdf");

        // non-Latin scripts
        assert_eq!(file::sanitize_filename("文件.txt"), "__.txt");
        assert_eq!(file::sanitize_filename("файл.txt"), "____.txt");
    }

    #[test]
    fn edge_cases() {
        // empty string
        assert_eq!(file::sanitize_filename(""), "");

        // string with only special characters
        assert_eq!(file::sanitize_filename("@#$%^&*"), "_______");

        // string with only allowed special characters
        assert_eq!(file::sanitize_filename(".-_"), ".-_");

        // repeated special characters
        assert_eq!(file::sanitize_filename("file!!!name"), "file___name");

        // leading/trailing special characters
        assert_eq!(file::sanitize_filename("...file..."), "...file...");
        assert_eq!(file::sanitize_filename("###file###"), "___file___");
    }

    #[test]
    fn common_filename_patterns() {
        // common file extensions
        assert_eq!(file::sanitize_filename("document.pdf"), "document.pdf");
        assert_eq!(file::sanitize_filename("image.jpg"), "image.jpg");
        assert_eq!(file::sanitize_filename("script.sh"), "script.sh");

        // hidden files (Unix-style)
        assert_eq!(file::sanitize_filename(".gitignore"), ".gitignore");

        // version numbers
        assert_eq!(
            file::sanitize_filename("file-v1.2.3.txt"),
            "file-v1.2.3.txt"
        );

        // common naming patterns
        assert_eq!(
            file::sanitize_filename("2023-01-01_backup.tar.gz"),
            "2023-01-01_backup.tar.gz"
        );
        assert_eq!(file::sanitize_filename("file (1)"), "file__1_");
        assert_eq!(file::sanitize_filename("my_file [v2]"), "my_file__v2_");
    }
}
