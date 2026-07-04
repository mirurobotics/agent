// standard crates
use std::fs;

// internal crates
use miru_agent::filesys::{self, File};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod stat_matches {
    use super::*;

    #[test]
    fn matches_regular_file_with_size() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data.txt");
        let contents = b"hello world";
        fs::write(&path, contents).unwrap();

        let pattern = tmp.path().join("*.txt");
        let matches = filesys::stat_matches(pattern.to_str().unwrap());

        assert_eq!(matches.len(), 1);
        let (file, size, _mtime) = &matches[0];
        assert_eq!(*file, File::new(path));
        assert_eq!(*size, contents.len() as u64);
    }

    #[test]
    fn non_matching_file_absent() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("data.log"), b"nope").unwrap();

        let pattern = tmp.path().join("*.txt");
        let matches = filesys::stat_matches(pattern.to_str().unwrap());

        assert!(matches.is_empty());
    }

    #[test]
    fn directory_matched_by_pattern_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        // A directory whose name matches the glob must be skipped (not a
        // regular file), while a sibling regular file is returned.
        fs::create_dir(tmp.path().join("subdir")).unwrap();
        let file_path = tmp.path().join("file");
        fs::write(&file_path, b"x").unwrap();

        let pattern = tmp.path().join("*");
        let matches = filesys::stat_matches(pattern.to_str().unwrap());

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, File::new(file_path));
    }

    #[test]
    fn invalid_glob_yields_empty() {
        let matches = filesys::stat_matches("[unclosed");
        assert!(matches.is_empty());
    }
}
