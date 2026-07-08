// standard crates
use std::os::unix::fs::PermissionsExt;
use std::{env, path::PathBuf};

// internal crates
use miru_agent::filesys::{self, dirs, files, FileSysErr, Overwrite, PathExt, WriteOptions};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod assert_exists {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        dir.assert_exists().unwrap();
    }

    #[test]
    fn failure() {
        let dir = filesys::Dir::new(PathBuf::from("nonexistent").join("path"));
        assert!(matches!(
            dir.assert_exists().unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }
}

pub mod assert_doesnt_exist {
    use super::*;

    #[test]
    fn success() {
        let dir = filesys::Dir::new(PathBuf::from("nonexistent").join("path"));
        dir.assert_doesnt_exist().unwrap();
    }

    #[tokio::test]
    async fn failure() {
        let dir = dirs::temp("testing").unwrap();
        assert!(matches!(
            dir.assert_doesnt_exist().unwrap_err(),
            FileSysErr::PathExistsErr { .. }
        ));
    }
}

pub mod delete {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        assert!(dir.exists());
        dirs::delete(&dir).await.unwrap();
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = filesys::Dir::new(PathBuf::from("doesnt_exist"));
        assert!(!dir.exists());
        dirs::delete(&dir).await.unwrap();
        assert!(!dir.exists());
    }
}

pub mod new_home_dir {
    use super::*;

    #[test]
    fn success() {
        let dir = dirs::home().unwrap();
        assert!(dir.exists());
        assert!(dir.path().to_str().unwrap().contains("home"));
    }
}

pub mod new_current_dir {
    use super::*;

    #[test]
    fn success() {
        let dir = dirs::current().unwrap();
        assert!(dir.exists());
        assert_eq!(dir.path(), &env::current_dir().unwrap());
    }
}

pub mod create_temp_dir {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        assert!(dir.exists());
        assert!(dir.path().to_str().unwrap().contains("testing"));
    }
}

mod create {
    use super::*;

    mod success {
        use super::*;

        #[tokio::test]
        async fn doesnt_exist() {
            let temp_dir = dirs::temp("testing").unwrap();

            let subdir = temp_dir.subdir(PathBuf::from("subdir"));
            dirs::create(&subdir).await.unwrap();
            assert!(subdir.exists());
        }

        #[tokio::test]
        async fn parent_doesnt_exist() {
            let temp_dir = dirs::temp("testing").unwrap();

            let subdir = temp_dir.subdir(PathBuf::from("does/not/exist"));
            dirs::create(&subdir).await.unwrap();
            assert!(subdir.exists());
        }

        #[tokio::test]
        async fn already_exists() {
            let dir = dirs::temp("testing").unwrap();
            dirs::create(&dir).await.unwrap();
            assert!(dir.exists());
        }
    }
}

mod create_if_absent {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let temp_dir = dirs::temp("testing").unwrap();

        let subdir = temp_dir.subdir(PathBuf::from("subdir"));
        dirs::create_if_absent(&subdir).await.unwrap();
        assert!(subdir.exists());
    }

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();

        // create some files in the directory to check if they exist afterward
        let file = dir.file("test-file");
        files::write_string(&file, "arglebargle", WriteOptions::default())
            .await
            .unwrap();

        // create the directory
        dirs::create_if_absent(&dir).await.unwrap();
        assert!(dir.exists());
        assert!(file.exists());
    }
}

mod subdirs {
    use super::*;

    #[tokio::test]
    async fn empty() {
        let dir = dirs::temp("testing").unwrap();
        assert_eq!(dirs::subdirs(&dir).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();

        // create some subdirs
        let subdir1 = dir.subdir(PathBuf::from("subdir1"));
        dirs::create(&subdir1).await.unwrap();
        let subdir2 = dir.subdir(PathBuf::from("subdir2"));
        dirs::create(&subdir2).await.unwrap();
        assert!(subdir1.exists());
        assert!(subdir2.exists());

        // get the subdirs
        let subdirs = dirs::subdirs(&dir).await.unwrap();
        assert_eq!(subdirs.len(), 2);
        assert!(subdirs.iter().any(|d| d.path() == subdir1.path()));
        assert!(subdirs.iter().any(|d| d.path() == subdir2.path()));
    }
}

mod list_files {
    use super::*;

    #[tokio::test]
    async fn empty() {
        let dir = dirs::temp("testing").unwrap();
        assert_eq!(dirs::files(&dir).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();

        // create some files
        let file1 = dir.file("file1.txt");
        files::write_string(&file1, "arglebargle", WriteOptions::default())
            .await
            .unwrap();
        let file2 = dir.file("file2.txt");
        files::write_string(&file2, "arglebargle", WriteOptions::default())
            .await
            .unwrap();

        // get the files
        let files = dirs::files(&dir).await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path() == file1.path()));
        assert!(files.iter().any(|f| f.path() == file2.path()));
    }
}

mod is_empty {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        assert!(dirs::is_empty(&dir).await.unwrap());
    }

    #[tokio::test]
    async fn has_files() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test");
        files::write_string(&file, "arglechargle", WriteOptions::default())
            .await
            .unwrap();
        assert!(!dirs::is_empty(&dir).await.unwrap());
    }

    #[tokio::test]
    async fn has_subdirs() {
        let dir = dirs::temp("testing").unwrap();
        let subdir = dir.subdir(PathBuf::from("test"));
        dirs::create(&subdir).await.unwrap();
        assert!(!dirs::is_empty(&dir).await.unwrap());
    }
}

mod delete_if_empty_recursive {
    use super::*;

    #[tokio::test]
    async fn success_empty() {
        let dir = dirs::temp("testing").unwrap();
        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn has_files() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test");
        files::write_string(&file, "arglechargle", WriteOptions::default())
            .await
            .unwrap();
        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn has_a_non_empty_subdir() {
        let dir = dirs::temp("testing").unwrap();
        let subdir = dir.subdir(PathBuf::from("test"));
        dirs::create(&subdir).await.unwrap();
        let file = subdir.file("test");
        files::write_string(&file, "arglechargle", WriteOptions::default())
            .await
            .unwrap();
        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn has_empty_subdir() {
        let dir = dirs::temp("testing").unwrap();
        let subdir = dir.subdir(PathBuf::from("test"));
        dirs::create(&subdir).await.unwrap();
        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn complex_nested_structure_all_empty() {
        let dir = dirs::temp("testing").unwrap();

        // Create nested structure: dir/subdir1/subdir2/subdir3
        let subdir1 = dir.subdir(PathBuf::from("subdir1"));
        dirs::create(&subdir1).await.unwrap();

        let subdir2 = subdir1.subdir(PathBuf::from("subdir2"));
        dirs::create(&subdir2).await.unwrap();

        let subdir3 = subdir2.subdir(PathBuf::from("subdir3"));
        dirs::create(&subdir3).await.unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn complex_nested_structure_mixed_content() {
        let dir = dirs::temp("testing").unwrap();

        // Create nested structure with some files
        let subdir1 = dir.subdir(PathBuf::from("subdir1"));
        dirs::create(&subdir1).await.unwrap();

        let subdir2 = subdir1.subdir(PathBuf::from("subdir2"));
        dirs::create(&subdir2).await.unwrap();

        // Add a file to subdir2 (making it non-empty)
        let file = subdir2.file("keep.txt");
        files::write_string(&file, "don't delete me", WriteOptions::default())
            .await
            .unwrap();

        let subdir3 = subdir2.subdir(PathBuf::from("subdir3"));
        dirs::create(&subdir3).await.unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists()); // Main dir should still exist
        assert!(subdir1.exists()); // subdir1 should still exist
        assert!(subdir2.exists()); // subdir2 should still exist (has file)
        assert!(!subdir3.exists()); // subdir3 should be deleted (empty)
    }

    #[tokio::test]
    async fn multiple_empty_subdirs_at_same_level() {
        let dir = dirs::temp("testing").unwrap();

        // Create multiple empty subdirs at the same level
        let subdir1 = dir.subdir(PathBuf::from("empty1"));
        dirs::create(&subdir1).await.unwrap();

        let subdir2 = dir.subdir(PathBuf::from("empty2"));
        dirs::create(&subdir2).await.unwrap();

        let subdir3 = dir.subdir(PathBuf::from("empty3"));
        dirs::create(&subdir3).await.unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(!dir.exists()); // All should be deleted
    }

    #[tokio::test]
    async fn mixed_empty_and_non_empty_subdirs() {
        let dir = dirs::temp("testing").unwrap();

        // Create empty subdir
        let empty_subdir = dir.subdir(PathBuf::from("empty"));
        dirs::create(&empty_subdir).await.unwrap();

        // Create non-empty subdir
        let non_empty_subdir = dir.subdir(PathBuf::from("non_empty"));
        dirs::create(&non_empty_subdir).await.unwrap();
        let file = non_empty_subdir.file("test.txt");
        files::write_string(&file, "content", WriteOptions::default())
            .await
            .unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists()); // Main dir should still exist
        assert!(!empty_subdir.exists()); // Empty subdir should be deleted
        assert!(non_empty_subdir.exists()); // Non-empty subdir should remain
    }

    #[tokio::test]
    async fn deeply_nested_with_files_at_different_levels() {
        let dir = dirs::temp("testing").unwrap();

        // Create structure: dir/level1/level2/level3/level4
        let level1 = dir.subdir(PathBuf::from("level1"));
        dirs::create(&level1).await.unwrap();

        let level2 = level1.subdir(PathBuf::from("level2"));
        dirs::create(&level2).await.unwrap();

        let level3 = level2.subdir(PathBuf::from("level3"));
        dirs::create(&level3).await.unwrap();

        let level4 = level3.subdir(PathBuf::from("level4"));
        dirs::create(&level4).await.unwrap();

        // Add file at level2
        let file2 = level2.file("level2_file.txt");
        files::write_string(&file2, "level2 content", WriteOptions::default())
            .await
            .unwrap();

        // Add file at level4
        let file4 = level4.file("level4_file.txt");
        files::write_string(&file4, "level4 content", WriteOptions::default())
            .await
            .unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists()); // Main dir should exist
        assert!(level1.exists()); // level1 should exist
        assert!(level2.exists()); // level2 should exist (has file)
        assert!(level3.exists()); // level3 should exist (has non-empty child)
        assert!(level4.exists()); // level4 should exist (has file)
    }

    #[tokio::test]
    async fn empty_subdirs_with_hidden_files() {
        let dir = dirs::temp("testing").unwrap();

        let subdir = dir.subdir(PathBuf::from("subdir"));
        dirs::create(&subdir).await.unwrap();

        // Add hidden file
        let hidden_file = subdir.file(".hidden");
        files::write_string(&hidden_file, "hidden content", WriteOptions::default())
            .await
            .unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists()); // Main dir should exist
        assert!(subdir.exists()); // Subdir should exist (has hidden file)
    }

    #[tokio::test]
    async fn stress_test_many_nested_directories() {
        let dir = dirs::temp("testing").unwrap();

        // Create many nested directories
        let mut current_dir = dir.clone();
        for i in 0..10 {
            let subdir = current_dir.subdir(format!("level{i}"));
            dirs::create(&subdir).await.unwrap();
            current_dir = subdir;
        }

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(!dir.exists()); // All should be deleted
    }

    #[tokio::test]
    async fn partial_cleanup_with_remaining_structure() {
        let dir = dirs::temp("testing").unwrap();

        // Create structure: dir/branch1/empty1, dir/branch1/empty2, dir/branch2/file
        let branch1 = dir.subdir(PathBuf::from("branch1"));
        dirs::create(&branch1).await.unwrap();

        let empty1 = branch1.subdir(PathBuf::from("empty1"));
        dirs::create(&empty1).await.unwrap();

        let empty2 = branch1.subdir(PathBuf::from("empty2"));
        dirs::create(&empty2).await.unwrap();

        let branch2 = dir.subdir(PathBuf::from("branch2"));
        dirs::create(&branch2).await.unwrap();

        let file = branch2.file("keep.txt");
        files::write_string(&file, "keep this", WriteOptions::default())
            .await
            .unwrap();

        assert!(dirs::delete_if_empty_recursive(&dir).await.is_ok());
        assert!(dir.exists()); // Main dir should exist
        assert!(!branch1.exists()); // branch1 should be deleted (all children empty)
        assert!(branch2.exists()); // branch2 should exist (has file)
    }
}

mod move_to {
    use super::*;

    /// Asserts that no leftover `.rename_trash_*` directories exist under `dir`.
    async fn assert_no_trash_dirs(dir: &filesys::Dir) {
        let siblings = dirs::subdirs(dir).await.unwrap();
        for sibling in &siblings {
            let name = sibling.name().unwrap();
            assert!(
                !name.starts_with(".rename_trash_"),
                "leftover trash directory found: {name}"
            );
        }
    }

    #[tokio::test]
    async fn src_doesnt_exist() {
        let base_dir = dirs::temp("testing").unwrap();
        let src = base_dir.subdir("src-dir");
        let dest = base_dir.subdir("dest-dir");

        // overwrite false
        assert!(matches!(
            dirs::move_to(&src, &dest, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));

        // overwrite true
        assert!(matches!(
            dirs::move_to(&src, &dest, Overwrite::Allow)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));

        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn dest_doesnt_exist() {
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src = base_dir.subdir("src-dir");
        dirs::create(&src).await.unwrap();
        files::write_string(
            &src.file("test.txt"),
            "test content",
            WriteOptions::default(),
        )
        .await
        .unwrap();

        // destination directory
        let dest = base_dir.subdir("dest-dir");

        // move source directory to destination directory
        dirs::move_to(&src, &dest, Overwrite::Deny).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());

        // check the file was moved
        assert!(dest.file("test.txt").exists());
        assert_eq!(
            files::read_string(&dest.file("test.txt")).await.unwrap(),
            "test content"
        );
        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn dest_exists_deny_overwrite() {
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src = base_dir.subdir("src-dir");
        dirs::create(&src).await.unwrap();

        // destination directory
        let dest = base_dir.subdir("dest-dir");
        dirs::create(&dest).await.unwrap();

        // move should fail
        assert!(matches!(
            dirs::move_to(&src, &dest, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::PathExistsErr { .. }
        ));

        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn dest_exists_allow_overwrite() {
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src = base_dir.subdir("src-dir");
        dirs::create(&src).await.unwrap();
        files::write_string(
            &src.file("src-file.txt"),
            "src content",
            WriteOptions::default(),
        )
        .await
        .unwrap();

        // destination directory
        let dest = base_dir.subdir("dest-dir");
        dirs::create(&dest).await.unwrap();
        assert!(dest.exists());
        files::write_string(
            &dest.file("dest-file.txt"),
            "dest content",
            WriteOptions::default(),
        )
        .await
        .unwrap();

        // overwrite true should succeed
        dirs::move_to(&src, &dest, Overwrite::Allow).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());

        // verify src file moved, dest file replaced
        assert!(dest.file("src-file.txt").exists());
        assert_eq!(
            files::read_string(&dest.file("src-file.txt"))
                .await
                .unwrap(),
            "src content"
        );
        assert!(!dest.file("dest-file.txt").exists());
        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn src_and_dest_are_same_dir() {
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src_dir = base_dir.subdir("test-dir");
        dirs::create(&src_dir).await.unwrap();
        files::write_string(&src_dir.file("test.txt"), "test", WriteOptions::default())
            .await
            .unwrap();

        // moving to itself should be a no-op
        dirs::move_to(&src_dir, &src_dir, Overwrite::Deny)
            .await
            .unwrap();
        assert!(src_dir.exists());
        assert!(src_dir.file("test.txt").exists());
        assert_eq!(
            files::read_string(&src_dir.file("test.txt")).await.unwrap(),
            "test"
        );

        dirs::move_to(&src_dir, &src_dir, Overwrite::Allow)
            .await
            .unwrap();
        assert!(src_dir.exists());
        assert!(src_dir.file("test.txt").exists());
        assert_eq!(
            files::read_string(&src_dir.file("test.txt")).await.unwrap(),
            "test"
        );
        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn moves_nested_structure() {
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src = base_dir.subdir("src-dir");
        dirs::create(&src).await.unwrap();
        let file1 = src.file("file1.txt");
        files::write_string(&file1, "file1", WriteOptions::default())
            .await
            .unwrap();
        // subdirectory 1
        let subdir1 = src.subdir("subdir1");
        dirs::create(&subdir1).await.unwrap();
        let file2 = subdir1.file("file2.txt");
        files::write_string(&file2, "file2", WriteOptions::default())
            .await
            .unwrap();
        // subdirectory 2
        let subdir2 = subdir1.subdir("subdir2");
        dirs::create(&subdir2).await.unwrap();
        let file3 = subdir2.file("file3.txt");
        files::write_string(&file3, "file3", WriteOptions::default())
            .await
            .unwrap();

        // destination directory
        let dest = base_dir.subdir("dest-dir");

        // move source directory to destination directory
        dirs::move_to(&src, &dest, Overwrite::Deny).await.unwrap();
        assert!(!src.exists());
        assert!(dest.exists());

        // verify root directory
        assert!(dest.file("file1.txt").exists());
        assert_eq!(
            files::read_string(&dest.file("file1.txt")).await.unwrap(),
            "file1"
        );
        assert_eq!(dirs::subdirs(&dest).await.unwrap().len(), 1);
        assert_eq!(dirs::files(&dest).await.unwrap().len(), 1);

        // verify subdirectory 1
        let dest_subdir1 = dest.subdir("subdir1");
        assert!(dest_subdir1.exists());
        assert!(dest_subdir1.file("file2.txt").exists());
        assert_eq!(
            files::read_string(&dest_subdir1.file("file2.txt"))
                .await
                .unwrap(),
            "file2"
        );
        assert_eq!(dirs::subdirs(&dest_subdir1).await.unwrap().len(), 1);
        assert_eq!(dirs::files(&dest_subdir1).await.unwrap().len(), 1);

        // verify subdirectory 2
        let dest_subdir2 = dest_subdir1.subdir("subdir2");
        assert!(dest_subdir2.exists());
        assert!(dest_subdir2.file("file3.txt").exists());
        assert_eq!(
            files::read_string(&dest_subdir2.file("file3.txt"))
                .await
                .unwrap(),
            "file3"
        );
        assert_eq!(dirs::subdirs(&dest_subdir2).await.unwrap().len(), 0);
        assert_eq!(dirs::files(&dest_subdir2).await.unwrap().len(), 1);
        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn creates_missing_parent_directory() {
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src = base_dir.subdir("src-dir");
        dirs::create(&src).await.unwrap();
        files::write_string(&src.file("test.txt"), "test", WriteOptions::default())
            .await
            .unwrap();

        // destination directory
        let dest = base_dir.subdir("parent").subdir("dest-dir");

        // move source directory to destination directory
        dirs::move_to(&src, &dest, Overwrite::Deny).await.unwrap();
        assert!(!src.exists());
        assert!(dest.parent().unwrap().exists());
        assert!(dest.exists());
        assert!(dest.file("test.txt").exists());
        assert_no_trash_dirs(&base_dir).await;
    }

    #[tokio::test]
    async fn overwrite_rollback_on_missing_src() {
        // When src doesn't exist but dest does, move_to_with_overwrite will:
        //   step 1: rename dest -> trash  (succeeds)
        //   step 2: rename src  -> dest   (fails — src missing)
        //   rollback: rename trash -> dest (restores original dest)
        let base_dir = dirs::temp("testing").unwrap();

        // source directory
        let src = base_dir.subdir("src-dir");

        // destination directory
        let dest = base_dir.subdir("dest-dir");
        dirs::create(&dest).await.unwrap();
        files::write_string(
            &dest.file("keep-me.txt"),
            "precious",
            WriteOptions::default(),
        )
        .await
        .unwrap();

        // move shoud fail
        let result = dirs::move_to(&src, &dest, Overwrite::Allow).await;
        assert!(result.is_err(), "expected error for missing src");

        // dest must be restored with its original content
        assert!(dest.exists());
        assert_eq!(
            files::read_string(&dest.file("keep-me.txt")).await.unwrap(),
            "precious"
        );

        assert_no_trash_dirs(&base_dir).await;
    }
}

pub mod set_permissions {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let target = dir.subdir("nonexistent-dir");
        let permissions = std::fs::Permissions::from_mode(0o755);

        assert!(matches!(
            dirs::set_permissions(&target, permissions)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn basic_permissions() {
        let dir = dirs::temp("testing").unwrap();
        let target = dir.subdir("test-dir");
        dirs::create(&target).await.unwrap();

        let readonly = std::fs::Permissions::from_mode(0o555);
        let readwrite = std::fs::Permissions::from_mode(0o755);
        let restricted = std::fs::Permissions::from_mode(0o700);

        // Test read-execute only (555 in octal)
        dirs::set_permissions(&target, readonly).await.unwrap();
        let perms = dirs::permissions(&target).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o555);

        // Test read-write-execute (755 in octal)
        dirs::set_permissions(&target, readwrite).await.unwrap();
        let perms = dirs::permissions(&target).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o755);

        // Test owner-only (700 in octal)
        dirs::set_permissions(&target, restricted).await.unwrap();
        let perms = dirs::permissions(&target).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o700);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_permission_combinations() {
        let dir = dirs::temp("testing").unwrap();
        let target = dir.subdir("test-dir");
        dirs::create(&target).await.unwrap();

        let permissions = [
            0o500, // read-execute for owner
            0o700, // read-write-execute for owner
            0o550, // read-execute for owner and group
            0o555, // read-execute for all
            0o755, // read-write-execute owner, read-execute others
            0o777, // full permissions for all
        ];

        for mode in permissions {
            let expected = std::fs::Permissions::from_mode(mode);
            dirs::set_permissions(&target, expected.clone())
                .await
                .unwrap();
            let actual = dirs::permissions(&target).await.unwrap();
            assert_eq!(actual.mode() & 0o777, expected.mode() & 0o777);
        }
    }
}

pub mod permissions {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let target = dir.subdir("nonexistent-dir");

        assert!(matches!(
            dirs::permissions(&target).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn returns_current_permissions() {
        let dir = dirs::temp("testing").unwrap();
        let target = dir.subdir("test-dir");
        dirs::create(&target).await.unwrap();

        // set known permissions and verify read-back
        dirs::set_permissions(&target, std::fs::Permissions::from_mode(0o750))
            .await
            .unwrap();
        let perms = dirs::permissions(&target).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o750);
    }
}
