// internal crates
use miru_agent::filesys::{dir::Dir, errors::FileSysErr, path::PathExt};

// external crates
use std::{env, path::PathBuf};
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod delete {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert!(dir.exists());
        dir.delete().await.unwrap();
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = Dir::new(PathBuf::from("doesnt_exist"));
        assert!(!dir.exists());
        dir.delete().await.unwrap();
        assert!(!dir.exists());
    }
}

pub mod new_home_dir {
    use super::*;

    #[test]
    fn success() {
        let dir = Dir::new_home_dir().unwrap();
        assert!(dir.exists());
        assert!(dir.path().to_str().unwrap().contains("home"));
    }
}

pub mod create_temp_dir {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert!(dir.exists());
        assert!(dir.path().to_str().unwrap().contains("testing"));
    }
}

pub mod name {
    use super::*;

    pub mod success {
        use super::*;

        #[test]
        fn basic_names() {
            let dir = Dir::new(PathBuf::from("lebron").join("james"));
            assert_eq!(dir.name().unwrap(), "james");

            let dir = Dir::new(PathBuf::from("lebron").join("james").join(""));
            assert_eq!(dir.name().unwrap(), "james");
        }

        #[test]
        fn with_special_characters() {
            let dir = Dir::new(PathBuf::from("path").join("my-dir_123"));
            assert_eq!(dir.name().unwrap(), "my-dir_123");

            let dir = Dir::new(PathBuf::from("path").join("dir.with.dots"));
            assert_eq!(dir.name().unwrap(), "dir.with.dots");

            let dir = Dir::new(PathBuf::from("path").join("dir with spaces"));
            assert_eq!(dir.name().unwrap(), "dir with spaces");
        }

        #[test]
        fn with_unicode() {
            let dir = Dir::new(PathBuf::from("path").join("目录"));
            assert_eq!(dir.name().unwrap(), "目录");

            let dir = Dir::new(PathBuf::from("path").join("привет"));
            assert_eq!(dir.name().unwrap(), "привет");

            let dir = Dir::new(PathBuf::from("path").join("🦀"));
            assert_eq!(dir.name().unwrap(), "🦀");
        }
    }

    pub mod failure {
        use super::*;

        #[test]
        fn root_directory() {
            let dir = Dir::new(PathBuf::from("/"));
            assert!(matches!(
                dir.name().unwrap_err(),
                FileSysErr::UnknownDirNameErr { .. }
            ));
        }

        #[test]
        fn empty_path() {
            let dir = Dir::new("");
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
            let dir = Dir::new(PathBuf::from("path").join("dir"));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "path");
        }

        #[test]
        fn with_trailing_separator() {
            let dir = Dir::new(PathBuf::from("path").join("dir").join(""));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "path");
        }

        #[test]
        fn with_trailing_separator_and_dot() {
            let dir = Dir::new(PathBuf::from("path").join("dir").join("."));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "path");
        }

        #[test]
        fn with_trailing_separator_and_dot_dot() {
            let dir = Dir::new(PathBuf::from("bronny").join("james").join("jr").join(".."));
            assert_eq!(dir.parent().unwrap().name().unwrap(), "bronny");
        }

        #[test]
        fn empty_path() {
            let dir = Dir::new("");

            let current_dir_path = env::current_dir().unwrap();
            let expected = current_dir_path.parent().unwrap();
            assert_eq!(dir.parent().unwrap().path(), expected);
        }
    }

    pub mod failure {
        use super::*;

        #[test]
        fn root_directory() {
            let dir = Dir::new(PathBuf::from("/"));
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
            assert!(Dir::is_valid_dir_name(dir_name));
            Dir::assert_valid_dir_name(dir_name).unwrap();
        }

        #[test]
        fn exact_max_length() {
            let dir_name = "a".repeat(255); // A string with 255 characters
            assert!(Dir::is_valid_dir_name(&dir_name));
            Dir::assert_valid_dir_name(&dir_name).unwrap();
        }

        #[test]
        fn contains_special_characters() {
            let special_chars = "!@#$%^&*()";
            for special_char in special_chars.chars() {
                let dir_name = format!("is_valid_dir_name{special_char}");
                assert!(Dir::is_valid_dir_name(&dir_name));
                Dir::assert_valid_dir_name(&dir_name).unwrap();
            }
        }

        #[test]
        fn contains_leading_trailing_spaces() {
            let dir_name = "  is_valid_dir_name  ";
            assert!(Dir::is_valid_dir_name(dir_name));
            Dir::assert_valid_dir_name(dir_name).unwrap();
        }
    }

    pub mod failure {
        use super::*;

        #[test]
        fn empty_string() {
            let dir_name = "";
            assert!(!Dir::is_valid_dir_name(dir_name));
            assert!(matches!(
                Dir::assert_valid_dir_name(dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }

        #[test]
        fn contains_slash() {
            let dir_name = "invalid/dir_name";
            assert!(!Dir::is_valid_dir_name(dir_name));
            assert!(matches!(
                Dir::assert_valid_dir_name(dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }

        #[test]
        fn contains_null_byte() {
            let dir_name = "invalid\0dir_name";
            assert!(!Dir::is_valid_dir_name(dir_name));
            assert!(matches!(
                Dir::assert_valid_dir_name(dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }

        #[test]
        fn exceeds_max_length() {
            let dir_name = "a".repeat(256); // A string with 256 characters
            assert!(!Dir::is_valid_dir_name(&dir_name));
            assert!(matches!(
                Dir::assert_valid_dir_name(&dir_name).unwrap_err(),
                FileSysErr::InvalidDirNameErr { .. }
            ));
        }
    }
}

mod subdir {
    use super::*;

    #[test]
    fn basic() {
        let dir = Dir::new(PathBuf::from("path").join("dir"));
        let subdir = dir.subdir(PathBuf::from("subdir"));
        assert_eq!(subdir.path(), &dir.path().join("subdir"));
        assert_eq!(subdir.name().unwrap(), "subdir");
    }

    #[test]
    fn nested_subdir_leading_slash() {
        let base_path = PathBuf::from("base").join("path");
        let base_dir = Dir::new(base_path);
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
        let base_dir = Dir::new(base_path);
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
        let dir = Dir::new(PathBuf::from("test"));
        let subdir = dir.subdir("space folder");
        assert_eq!(subdir.path(), &PathBuf::from("test").join("space folder"));
        assert_eq!(subdir.name().unwrap(), "space folder");

        let subdir = dir.subdir("hyphen-folder");
        assert_eq!(subdir.path(), &PathBuf::from("test").join("hyphen-folder"));
        assert_eq!(subdir.name().unwrap(), "hyphen-folder");
    }

    #[test]
    fn with_empty_path() {
        let dir = Dir::new(PathBuf::from("test"));
        let subdir = dir.subdir("");
        assert_eq!(subdir.path(), &PathBuf::from("test").join(""));
    }

    #[test]
    fn with_absolute_path_component() {
        let dir = Dir::new(PathBuf::from("test"));
        let path_component = PathBuf::from("absolute").join("path");
        let subdir = dir.subdir(path_component);
        assert_eq!(
            subdir.path(),
            &PathBuf::from("test").join("absolute").join("path")
        );
    }

    #[test]
    fn with_dot_paths() {
        let dir = Dir::new(PathBuf::from("test"));
        let subdir = dir.subdir(".");
        assert_eq!(subdir.path(), &PathBuf::from("test").join("."));

        let subdir = dir.subdir("..");
        assert_eq!(subdir.path(), &PathBuf::from("test").join(".."));
    }
}

mod create {
    use super::*;

    mod success {
        use super::*;

        #[tokio::test]
        async fn doesnt_exist_with_overwrite() {
            let temp_dir = Dir::create_temp_dir("testing").await.unwrap();

            let subdir = temp_dir.subdir(PathBuf::from("subdir"));
            subdir.create(true).await.unwrap();
            assert!(subdir.exists());
        }

        #[tokio::test]
        async fn doesnt_exist_no_overwrite() {
            let temp_dir = Dir::create_temp_dir("testing").await.unwrap();

            let subdir = temp_dir.subdir(PathBuf::from("subdir"));
            subdir.create(false).await.unwrap();
            assert!(subdir.exists());
        }

        #[tokio::test]
        async fn exists_with_overwrite() {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            dir.create(true).await.unwrap();
            assert!(dir.exists());
        }

        #[tokio::test]
        async fn exists_no_overwrite() {
            let dir = Dir::create_temp_dir("testing").await.unwrap();

            assert!(matches!(
                dir.create(false).await.unwrap_err(),
                FileSysErr::PathExistsErr { .. }
            ));
        }
    }
}

mod create_if_absent {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let temp_dir = Dir::create_temp_dir("testing").await.unwrap();

        let subdir = temp_dir.subdir(PathBuf::from("subdir"));
        subdir.create_if_absent().await.unwrap();
        assert!(subdir.exists());
    }

    #[tokio::test]
    async fn exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // create some files in the directory to check if they exist afterward
        let file = dir.file("test-file");
        file.write_string("arglebargle", false, false)
            .await
            .unwrap();

        // create the directory
        dir.create_if_absent().await.unwrap();
        assert!(dir.exists());
        assert!(file.exists());
    }
}

mod file {
    use super::*;

    #[test]
    fn filename_with_leading_slash() {
        let dir_path = PathBuf::from("tmp").join("test_dir");
        let dir = Dir::new(dir_path.clone());
        let file = dir.file("/root/test.txt");
        assert_eq!(file.path(), &dir_path.join("root").join("test.txt"));
    }

    #[test]
    fn abs_dir() {
        let dir_path = PathBuf::from("tmp").join("test_dir");
        let dir = Dir::new(dir_path.clone());
        let file = dir.file("test.txt");
        assert_eq!(file.path(), &dir_path.join("test.txt"));
    }

    #[test]
    fn nested_file_path() {
        let dir_path = PathBuf::from("base").join("test_dir");
        let dir = Dir::new(dir_path.clone());
        let file = dir.file("nested/folder/test.txt");
        assert_eq!(
            file.path(),
            &dir_path.join("nested").join("folder").join("test.txt")
        );
    }

    #[test]
    fn special_characters() {
        let dir_path = PathBuf::from("test_dir");
        let dir = Dir::new(dir_path.clone());

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
        let dir = Dir::new(dir_path.clone());
        let file = dir.file("");
        assert_eq!(file.path(), &dir_path.join(""));
    }

    #[test]
    fn with_different_extensions() {
        let dir_path = PathBuf::from("test_dir");
        let dir = Dir::new(dir_path.clone());

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
        let dir = Dir::new(dir_path.clone());

        let file = dir.file("文件.txt");
        assert_eq!(file.path(), &dir_path.join("文件.txt"));

        let file = dir.file("🦀rust.rs");
        assert_eq!(file.path(), &dir_path.join("🦀rust.rs"));
    }
}

mod subdirs {
    use super::*;

    #[tokio::test]
    async fn empty() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert_eq!(dir.subdirs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // create some subdirs
        let subdir1 = dir.subdir(PathBuf::from("subdir1"));
        subdir1.create(true).await.unwrap();
        let subdir2 = dir.subdir(PathBuf::from("subdir2"));
        subdir2.create(true).await.unwrap();
        assert!(subdir1.exists());
        assert!(subdir2.exists());

        // get the subdirs
        let subdirs = dir.subdirs().await.unwrap();
        assert_eq!(subdirs.len(), 2);
        assert!(subdirs.iter().any(|d| d.path() == subdir1.path()));
        assert!(subdirs.iter().any(|d| d.path() == subdir2.path()));
    }
}

mod files {
    use super::*;

    #[tokio::test]
    async fn empty() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert_eq!(dir.files().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // create some files
        let file1 = dir.file("file1.txt");
        file1
            .write_string("arglebargle", false, false)
            .await
            .unwrap();
        let file2 = dir.file("file2.txt");
        file2
            .write_string("arglebargle", false, false)
            .await
            .unwrap();

        // get the files
        let files = dir.files().await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path() == file1.path()));
        assert!(files.iter().any(|f| f.path() == file2.path()));
    }
}

mod is_empty {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert!(dir.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn has_files() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test");
        file.write_string("arglechargle", false, false)
            .await
            .unwrap();
        assert!(!dir.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn has_subdirs() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let subdir = dir.subdir(PathBuf::from("test"));
        subdir.create(true).await.unwrap();
        assert!(!dir.is_empty().await.unwrap());
    }
}

mod delete_if_empty_recursive {
    use super::*;

    #[tokio::test]
    async fn success_empty() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn has_files() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test");
        file.write_string("arglechargle", false, false)
            .await
            .unwrap();
        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn has_a_non_empty_subdir() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let subdir = dir.subdir(PathBuf::from("test"));
        subdir.create(true).await.unwrap();
        let file = subdir.file("test");
        file.write_string("arglechargle", false, false)
            .await
            .unwrap();
        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn has_empty_subdir() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let subdir = dir.subdir(PathBuf::from("test"));
        subdir.create(true).await.unwrap();
        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn complex_nested_structure_all_empty() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create nested structure: dir/subdir1/subdir2/subdir3
        let subdir1 = dir.subdir(PathBuf::from("subdir1"));
        subdir1.create(true).await.unwrap();

        let subdir2 = subdir1.subdir(PathBuf::from("subdir2"));
        subdir2.create(true).await.unwrap();

        let subdir3 = subdir2.subdir(PathBuf::from("subdir3"));
        subdir3.create(true).await.unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn complex_nested_structure_mixed_content() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create nested structure with some files
        let subdir1 = dir.subdir(PathBuf::from("subdir1"));
        subdir1.create(true).await.unwrap();

        let subdir2 = subdir1.subdir(PathBuf::from("subdir2"));
        subdir2.create(true).await.unwrap();

        // Add a file to subdir2 (making it non-empty)
        let file = subdir2.file("keep.txt");
        file.write_string("don't delete me", false, false)
            .await
            .unwrap();

        let subdir3 = subdir2.subdir(PathBuf::from("subdir3"));
        subdir3.create(true).await.unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists()); // Main dir should still exist
        assert!(subdir1.exists()); // subdir1 should still exist
        assert!(subdir2.exists()); // subdir2 should still exist (has file)
        assert!(!subdir3.exists()); // subdir3 should be deleted (empty)
    }

    #[tokio::test]
    async fn multiple_empty_subdirs_at_same_level() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create multiple empty subdirs at the same level
        let subdir1 = dir.subdir(PathBuf::from("empty1"));
        subdir1.create(true).await.unwrap();

        let subdir2 = dir.subdir(PathBuf::from("empty2"));
        subdir2.create(true).await.unwrap();

        let subdir3 = dir.subdir(PathBuf::from("empty3"));
        subdir3.create(true).await.unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(!dir.exists()); // All should be deleted
    }

    #[tokio::test]
    async fn mixed_empty_and_non_empty_subdirs() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create empty subdir
        let empty_subdir = dir.subdir(PathBuf::from("empty"));
        empty_subdir.create(true).await.unwrap();

        // Create non-empty subdir
        let non_empty_subdir = dir.subdir(PathBuf::from("non_empty"));
        non_empty_subdir.create(true).await.unwrap();
        let file = non_empty_subdir.file("test.txt");
        file.write_string("content", false, false).await.unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists()); // Main dir should still exist
        assert!(!empty_subdir.exists()); // Empty subdir should be deleted
        assert!(non_empty_subdir.exists()); // Non-empty subdir should remain
    }

    #[tokio::test]
    async fn deeply_nested_with_files_at_different_levels() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create structure: dir/level1/level2/level3/level4
        let level1 = dir.subdir(PathBuf::from("level1"));
        level1.create(true).await.unwrap();

        let level2 = level1.subdir(PathBuf::from("level2"));
        level2.create(true).await.unwrap();

        let level3 = level2.subdir(PathBuf::from("level3"));
        level3.create(true).await.unwrap();

        let level4 = level3.subdir(PathBuf::from("level4"));
        level4.create(true).await.unwrap();

        // Add file at level2
        let file2 = level2.file("level2_file.txt");
        file2
            .write_string("level2 content", false, false)
            .await
            .unwrap();

        // Add file at level4
        let file4 = level4.file("level4_file.txt");
        file4
            .write_string("level4 content", false, false)
            .await
            .unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists()); // Main dir should exist
        assert!(level1.exists()); // level1 should exist
        assert!(level2.exists()); // level2 should exist (has file)
        assert!(level3.exists()); // level3 should exist (has non-empty child)
        assert!(level4.exists()); // level4 should exist (has file)
    }

    #[tokio::test]
    async fn empty_subdirs_with_hidden_files() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        let subdir = dir.subdir(PathBuf::from("subdir"));
        subdir.create(true).await.unwrap();

        // Add hidden file
        let hidden_file = subdir.file(".hidden");
        hidden_file
            .write_string("hidden content", false, false)
            .await
            .unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists()); // Main dir should exist
        assert!(subdir.exists()); // Subdir should exist (has hidden file)
    }

    #[tokio::test]
    async fn stress_test_many_nested_directories() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create many nested directories
        let mut current_dir = dir.clone();
        for i in 0..10 {
            let subdir = current_dir.subdir(format!("level{i}"));
            subdir.create(true).await.unwrap();
            current_dir = subdir;
        }

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(!dir.exists()); // All should be deleted
    }

    #[tokio::test]
    async fn partial_cleanup_with_remaining_structure() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();

        // Create structure: dir/branch1/empty1, dir/branch1/empty2, dir/branch2/file
        let branch1 = dir.subdir(PathBuf::from("branch1"));
        branch1.create(true).await.unwrap();

        let empty1 = branch1.subdir(PathBuf::from("empty1"));
        empty1.create(true).await.unwrap();

        let empty2 = branch1.subdir(PathBuf::from("empty2"));
        empty2.create(true).await.unwrap();

        let branch2 = dir.subdir(PathBuf::from("branch2"));
        branch2.create(true).await.unwrap();

        let file = branch2.file("keep.txt");
        file.write_string("keep this", false, false).await.unwrap();

        assert!(dir.delete_if_empty_recursive().await.is_ok());
        assert!(dir.exists()); // Main dir should exist
        assert!(!branch1.exists()); // branch1 should be deleted (all children empty)
        assert!(branch2.exists()); // branch2 should exist (has file)
    }
}

mod move_to {
    use super::*;

    #[tokio::test]
    async fn src_doesnt_exist() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        let dest = base_dir.subdir("dest-dir");

        // overwrite false
        assert!(matches!(
            src.move_to(&dest, false).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));

        // overwrite true
        assert!(matches!(
            src.move_to(&dest, true).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_doesnt_exist() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        src.create(false).await.unwrap();

        // Add a file to verify it moves with the directory
        let test_file = src.file("test.txt");
        test_file
            .write_string("test content", false, false)
            .await
            .unwrap();

        let dest = base_dir.subdir("dest-dir");

        // overwrite false
        assert!(src.exists());
        assert!(!dest.exists());
        src.move_to(&dest, false).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());
        // Verify file moved with directory
        assert!(dest.file("test.txt").exists());
        assert_eq!(
            dest.file("test.txt").read_string().await.unwrap(),
            "test content"
        );
    }

    #[tokio::test]
    async fn dest_exists_overwrite_false() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        src.create(false).await.unwrap();
        let dest = base_dir.subdir("dest-dir");
        dest.create(false).await.unwrap();

        // overwrite false should fail
        assert!(src.exists());
        assert!(dest.exists());
        assert!(matches!(
            src.move_to(&dest, false).await.unwrap_err(),
            FileSysErr::PathExistsErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_exists_overwrite_true() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        src.create(false).await.unwrap();

        // Add files to src
        let src_file = src.file("src-file.txt");
        src_file
            .write_string("src content", false, false)
            .await
            .unwrap();

        let dest = base_dir.subdir("dest-dir");
        dest.create(false).await.unwrap();

        // Add different files to dest
        let dest_file = dest.file("dest-file.txt");
        dest_file
            .write_string("dest content", false, false)
            .await
            .unwrap();

        // overwrite true should succeed
        assert!(src.exists());
        assert!(dest.exists());
        src.move_to(&dest, true).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());
        // Verify src file moved, dest file replaced
        assert!(dest.file("src-file.txt").exists());
        assert_eq!(
            dest.file("src-file.txt").read_string().await.unwrap(),
            "src content"
        );
        assert!(!dest.file("dest-file.txt").exists());
    }

    #[tokio::test]
    async fn src_and_dest_are_same_dir() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let dir = base_dir.subdir("test-dir");
        dir.create(false).await.unwrap();

        let test_file = dir.file("test.txt");
        test_file.write_string("test", false, false).await.unwrap();

        // Moving to itself should be a no-op
        dir.move_to(&dir, false).await.unwrap();
        assert!(dir.exists());
        assert!(test_file.exists());
        assert_eq!(test_file.read_string().await.unwrap(), "test");

        dir.move_to(&dir, true).await.unwrap();
        assert!(dir.exists());
        assert!(test_file.exists());
        assert_eq!(test_file.read_string().await.unwrap(), "test");
    }

    #[tokio::test]
    async fn moves_nested_structure() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        src.create(false).await.unwrap();

        // Create nested structure
        let subdir1 = src.subdir("subdir1");
        subdir1.create(false).await.unwrap();
        let subdir2 = subdir1.subdir("subdir2");
        subdir2.create(false).await.unwrap();

        // Add files at different levels
        let file1 = src.file("file1.txt");
        file1.write_string("file1", false, false).await.unwrap();
        let file2 = subdir1.file("file2.txt");
        file2.write_string("file2", false, false).await.unwrap();
        let file3 = subdir2.file("file3.txt");
        file3.write_string("file3", false, false).await.unwrap();

        let dest = base_dir.subdir("dest-dir");

        src.move_to(&dest, false).await.unwrap();

        // Verify entire structure moved
        assert!(!src.exists());
        assert!(dest.exists());
        assert!(dest.file("file1.txt").exists());
        assert_eq!(dest.file("file1.txt").read_string().await.unwrap(), "file1");

        let moved_subdir1 = dest.subdir("subdir1");
        assert!(moved_subdir1.exists());
        assert!(moved_subdir1.file("file2.txt").exists());
        assert_eq!(
            moved_subdir1.file("file2.txt").read_string().await.unwrap(),
            "file2"
        );

        let moved_subdir2 = moved_subdir1.subdir("subdir2");
        assert!(moved_subdir2.exists());
        assert!(moved_subdir2.file("file3.txt").exists());
        assert_eq!(
            moved_subdir2.file("file3.txt").read_string().await.unwrap(),
            "file3"
        );
    }

    #[tokio::test]
    async fn creates_parent_directory() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        src.create(false).await.unwrap();

        let test_file = src.file("test.txt");
        test_file.write_string("test", false, false).await.unwrap();

        // Create a nested destination path (parent doesn't exist)
        let dest = base_dir.subdir("parent").subdir("dest-dir");

        src.move_to(&dest, false).await.unwrap();

        // Verify parent was created and directory moved
        assert!(dest.parent().unwrap().exists());
        assert!(dest.exists());
        assert!(dest.file("test.txt").exists());
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn overwrite_removes_existing_content() {
        let base_dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = base_dir.subdir("src-dir");
        src.create(false).await.unwrap();
        let src_file = src.file("src-only.txt");
        src_file
            .write_string("src only", false, false)
            .await
            .unwrap();

        let dest = base_dir.subdir("dest-dir");
        dest.create(false).await.unwrap();
        let dest_file = dest.file("dest-only.txt");
        dest_file
            .write_string("dest only", false, false)
            .await
            .unwrap();
        let dest_subdir = dest.subdir("dest-subdir");
        dest_subdir.create(false).await.unwrap();

        // Move with overwrite
        src.move_to(&dest, true).await.unwrap();

        // Verify dest content was replaced
        assert!(dest.exists());
        assert!(dest.file("src-only.txt").exists());
        assert!(!dest.file("dest-only.txt").exists());
        assert!(!dest.subdir("dest-subdir").exists());
        assert!(!src.exists());
    }
}
