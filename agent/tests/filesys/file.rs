// standard library
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::{
    dir::Dir, errors::FileSysErr, file, file::File, path::PathExt, Atomic, Overwrite, WriteOptions,
};

// external crates
use secrecy::ExposeSecret;
use serde_json::json;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod display {
    use super::*;

    #[test]
    fn absolute_path() {
        let file = File::new(PathBuf::from("/tmp").join("test-file.txt"));
        assert_eq!(file.path(), &PathBuf::from("/tmp").join("test-file.txt"));
    }

    #[test]
    fn relative_path() {
        let file = File::new(PathBuf::from("relative").join("path.txt"));
        assert_eq!(file.path(), &PathBuf::from("relative").join("path.txt"));
    }
}

pub mod parent {
    use super::*;

    #[test]
    fn simple() {
        let file = File::new(PathBuf::from("tmp").join("some-dir").join("test-file.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("tmp").join("some-dir"));
    }

    #[test]
    fn nested() {
        let file = File::new(PathBuf::from("a").join("b").join("c").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b").join("c"));
    }

    #[test]
    fn trailing_separator() {
        let file = File::new(PathBuf::from("a").join("b").join("").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b"));
    }

    #[test]
    fn trailing_separator_and_dot() {
        let file = File::new(PathBuf::from("a").join("b").join(".").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b"));
    }

    #[test]
    fn trailing_separator_and_dot_dot() {
        let file = File::new(PathBuf::from("a").join("b").join("..").join("d.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("a").join("b").join(".."));
    }

    #[test]
    fn root_file() {
        let file = File::new(PathBuf::from("/file.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from("/"));
    }

    #[test]
    fn file_only() {
        let file = File::new(PathBuf::from("file.txt"));
        let parent = file.parent().unwrap();
        assert_eq!(parent.path(), &PathBuf::from(""));
    }
}

pub mod assert_exists {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        file.assert_exists().unwrap();
    }

    #[test]
    fn failure() {
        let file = File::new(PathBuf::from("nonexistent").join("path").join("file.txt"));
        assert!(matches!(
            file.assert_exists().unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }
}

pub mod assert_doesnt_exist {
    use super::*;

    #[test]
    fn success() {
        let file = File::new(PathBuf::from("nonexistent").join("path").join("file.txt"));
        file.assert_doesnt_exist().unwrap();
    }

    #[tokio::test]
    async fn failure() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        assert!(matches!(
            file.assert_doesnt_exist().unwrap_err(),
            FileSysErr::PathExistsErr { .. }
        ));
    }
}

pub mod delete {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        assert!(file.exists());
        file.delete().await.unwrap();
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let file = File::new(PathBuf::from("doesnt_exist"));
        assert!(!file.exists());
        file.delete().await.unwrap();
        assert!(!file.exists());
    }
}

pub mod name {
    use super::*;

    #[tokio::test]
    async fn basic_names() {
        let file = File::new(PathBuf::from("lebron").join("james.txt"));
        assert_eq!(file.name().unwrap(), "james.txt");

        let file = File::new(PathBuf::from("lebron").join("james.txt").join(""));
        assert_eq!(file.name().unwrap(), "james.txt");
    }

    #[tokio::test]
    async fn with_special_characters() {
        let file = File::new(PathBuf::from("path").join("my-file_123.txt"));
        assert_eq!(file.name().unwrap(), "my-file_123.txt");

        let file = File::new(PathBuf::from("path").join("file.with.dots.txt"));
        assert_eq!(file.name().unwrap(), "file.with.dots.txt");

        let file = File::new(PathBuf::from("path").join("file with spaces.txt"));
        assert_eq!(file.name().unwrap(), "file with spaces.txt");
    }

    #[tokio::test]
    async fn with_unicode() {
        let file = File::new(PathBuf::from("path").join("文件.txt"));
        assert_eq!(file.name().unwrap(), "文件.txt");

        let file = File::new(PathBuf::from("path").join("файл.txt"));
        assert_eq!(file.name().unwrap(), "файл.txt");

        let file = File::new(PathBuf::from("path").join("🦀.txt"));
        assert_eq!(file.name().unwrap(), "🦀.txt");
    }

    #[tokio::test]
    async fn root_path() {
        let file = File::new(PathBuf::from("/"));
        assert!(matches!(
            file.name().unwrap_err(),
            FileSysErr::UnknownFileNameErr { .. }
        ));
    }

    #[tokio::test]
    async fn empty_path() {
        let file = File::new(PathBuf::from(""));
        assert!(matches!(
            file.name().unwrap_err(),
            FileSysErr::UnknownFileNameErr { .. }
        ));
    }
}

pub mod move_to {
    use super::*;

    #[tokio::test]
    async fn src_doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");

        // overwrite false
        assert!(matches!(
            file.move_to(&file, Overwrite::Deny).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));

        // overwrite true
        assert!(matches!(
            file.move_to(&file, Overwrite::Deny).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = dir.file("src-file");
        src.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");

        // overwrite false
        assert!(src.exists());
        assert!(!dest.exists());
        src.move_to(&dest, Overwrite::Deny).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());

        // overwrite true
        let tmp = src;
        let src = dest;
        let dest = tmp;
        assert!(src.exists());
        assert!(!dest.exists());
        src.move_to(&dest, Overwrite::Allow).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn dest_exists_overwrite_false() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = dir.file("src-file");
        src.write_string("src", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        dest.write_string("dest", WriteOptions::default())
            .await
            .unwrap();

        // overwrite false
        assert!(src.exists());
        assert!(dest.exists());
        assert!(matches!(
            src.move_to(&dest, Overwrite::Deny).await.unwrap_err(),
            FileSysErr::InvalidFileOverwriteErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_exists_overwrite_true() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let src = dir.file("src-file");
        src.write_string("src", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        dest.write_string("dest", WriteOptions::default())
            .await
            .unwrap();

        // overwrite false
        assert!(src.exists());
        assert!(dest.exists());
        src.move_to(&dest, Overwrite::Allow).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn src_and_dest_are_same_file() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        file.move_to(&file, Overwrite::Deny).await.unwrap();
        file.assert_exists().unwrap();
        file.move_to(&file, Overwrite::Allow).await.unwrap();
        assert!(file.exists());
        assert!(file.read_string().await.unwrap() == "test");
    }
}

pub mod read_bytes {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            file.read_bytes().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("arglebargle", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(file.read_bytes().await.unwrap(), b"arglebargle");
    }
}

pub mod read_secret_bytes {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            file.read_secret_bytes().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("arglebargle", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            file.read_secret_bytes().await.unwrap().expose_secret(),
            b"arglebargle"
        );
    }
}

pub mod read_string {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            file.read_string().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("arglebargle", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(file.read_string().await.unwrap(), "arglebargle");
    }
}

pub mod read_json {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            file.read_json::<String>().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("{\"test\": \"arglebargle\"}", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            file.read_json::<serde_json::Value>().await.unwrap(),
            serde_json::json!({"test": "arglebargle"})
        );
    }
}

pub mod write_bytes {
    use super::*;

    fn write_bytes_atomic(
        file: &File,
        buf: &[u8],
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let buf = buf.to_vec();
        Box::pin(async move {
            file.write_bytes(
                &buf,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::Yes,
                },
            )
            .await
        })
    }

    fn write_bytes_non_atomic(
        file: &File,
        buf: &[u8],
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let buf = buf.to_vec();
        Box::pin(async move {
            file.write_bytes(
                &buf,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::No,
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let write_funcs = &[write_bytes_atomic, write_bytes_non_atomic];
        for write_bytes in write_funcs {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_bytes().await.unwrap(), b"arglebargle");
        }
    }

    #[tokio::test]
    async fn parent_doesnt_exist() {
        for write_bytes in [write_bytes_atomic, write_bytes_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
            let file = subdir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_bytes().await.unwrap(), b"arglebargle");
        }
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        for write_bytes in [write_bytes_atomic, write_bytes_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_bytes().await.unwrap(), b"arglebargle");

            // should fail when writing again
            assert!(matches!(
                write_bytes(&file, b"arglebargle", Overwrite::Deny)
                    .await
                    .unwrap_err(),
                FileSysErr::InvalidFileOverwriteErr { .. }
            ));
        }
    }

    #[tokio::test]
    async fn exists_overwrite_true() {
        for write_bytes in [write_bytes_atomic, write_bytes_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_bytes().await.unwrap(), b"arglebargle");

            // should succeed when writing again
            write_bytes(&file, b"arglebargle", Overwrite::Allow)
                .await
                .unwrap();
            assert_eq!(file.read_bytes().await.unwrap(), b"arglebargle");
        }
    }
}

pub mod write_string {
    use super::*;

    fn write_string_atomic(
        file: &File,
        s: &str,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let s = s.to_string();
        Box::pin(async move {
            file.write_string(
                &s,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::Yes,
                },
            )
            .await
        })
    }

    fn write_string_non_atomic(
        file: &File,
        s: &str,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let s = s.to_string();
        Box::pin(async move {
            file.write_string(
                &s,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::No,
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn doesnt_exist() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_string().await.unwrap(), "hello world");
        }
    }

    #[tokio::test]
    async fn parent_doesnt_exist() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
            let file = subdir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_string().await.unwrap(), "hello world");
        }
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_string().await.unwrap(), "hello world");

            // should fail when writing again
            assert!(matches!(
                write_string(&file, "new content", Overwrite::Deny)
                    .await
                    .unwrap_err(),
                FileSysErr::InvalidFileOverwriteErr { .. }
            ));
        }
    }

    #[tokio::test]
    async fn exists_overwrite_true() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(file.read_string().await.unwrap(), "hello world");

            // should succeed when writing again
            write_string(&file, "new content", Overwrite::Allow)
                .await
                .unwrap();
            assert_eq!(file.read_string().await.unwrap(), "new content");
        }
    }
}

mod write_json {
    use super::*;

    fn write_json_atomic(
        file: &File,
        data: &serde_json::Value,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let data = data.clone();
        Box::pin(async move {
            file.write_json(
                &data,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::Yes,
                },
            )
            .await
        })
    }

    fn write_json_non_atomic(
        file: &File,
        data: &serde_json::Value,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let data = data.clone();
        Box::pin(async move {
            file.write_json(
                &data,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::No,
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn doesnt_exist() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            let data = json!({
                "name": "test",
                "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = file.read_json().await.unwrap();
            assert_eq!(read_data, data);
        }
    }

    #[tokio::test]
    async fn parent_doesnt_exist() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
            let file = subdir.file("test-file");
            let data = json!({
            "name": "test",
            "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = file.read_json().await.unwrap();
            assert_eq!(read_data, data);
        }
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            let data = json!({
            "name": "test",
            "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = file.read_json().await.unwrap();
            assert_eq!(read_data, data);

            // should fail when writing again
            let new_data = json!({
                "name": "updated",
                "value": 100
            });
            assert!(matches!(
                write_json(&file, &new_data, Overwrite::Deny)
                    .await
                    .unwrap_err(),
                FileSysErr::InvalidFileOverwriteErr { .. }
            ));
        }
    }

    #[tokio::test]
    async fn exists_overwrite_true() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = Dir::create_temp_dir("testing").await.unwrap();
            let file = dir.file("test-file");
            let data = json!({
            "name": "test",
            "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = file.read_json().await.unwrap();
            assert_eq!(read_data, data);

            // should succeed when writing again
            let new_data = json!({
                "name": "updated",
                "value": 100
            });
            write_json(&file, &new_data, Overwrite::Allow)
                .await
                .unwrap();
            let read_data: serde_json::Value = file.read_json().await.unwrap();
            assert_eq!(read_data, new_data);
        }
    }
}

pub mod set_permissions {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("nonexistent-file");
        let permissions = std::fs::Permissions::from_mode(0o644);

        // Should fail because file doesn't exist
        assert!(matches!(
            file.set_permissions(permissions).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn basic_permissions() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");

        // Create the file first
        file.write_string("test content", WriteOptions::default())
            .await
            .unwrap();
        let readonly = std::fs::Permissions::from_mode(0o444);
        let readwrite = std::fs::Permissions::from_mode(0o644);
        let executable = std::fs::Permissions::from_mode(0o755);

        // Test read-only (444 in octal)
        file.set_permissions(readonly).await.unwrap();
        let perms = file.permissions().await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o444);

        // Test read-write (644 in octal)
        file.set_permissions(readwrite).await.unwrap();
        let perms = file.permissions().await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o644);

        // Test executable (755 in octal)
        file.set_permissions(executable).await.unwrap();
        let perms = file.permissions().await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_permission_combinations() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test content", WriteOptions::default())
            .await
            .unwrap();

        // Test various permission combinations
        let permissions = [
            0o400, // read only for owner
            0o200, // write only for owner
            0o100, // execute only for owner
            0o440, // read for owner and group
            0o444, // read for owner, group, and others
            0o666, // read-write for all
            0o777, // read-write-execute for all
        ];

        for mode in permissions {
            let expected = std::fs::Permissions::from_mode(mode);
            file.set_permissions(expected.clone()).await.unwrap();
            let actual = file.permissions().await.unwrap();
            assert_eq!(actual.mode() & 0o777, expected.mode() & 0o777);
        }
    }
}

pub mod create_symlink {
    use super::*;

    #[tokio::test]
    async fn src_doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("nonexistent-file");
        let link = dir.file("link");
        assert!(matches!(
            file.create_symlink(&link, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_doesnt_exist_overwrite_false() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");

        // overwrite false
        file.create_symlink(&link, Overwrite::Deny).await.unwrap();
        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
    }

    #[tokio::test]
    async fn dest_doesnt_exist_overwrite_true() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");

        // overwrite true
        file.create_symlink(&link, Overwrite::Allow).await.unwrap();
        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
    }

    #[tokio::test]
    async fn dest_exists_overwrite_false() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");
        file.create_symlink(&link, Overwrite::Allow).await.unwrap();

        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
        assert!(matches!(
            file.create_symlink(&link, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::InvalidFileOverwriteErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_exists_overwrite_true() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");
        file.create_symlink(&link, Overwrite::Allow).await.unwrap();

        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
        file.create_symlink(&link, Overwrite::Allow).await.unwrap();
        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
    }
}

// permissions test above
pub mod permissions {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("nonexistent-file");

        // Should fail because file doesn't exist
        assert!(matches!(
            file.permissions().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn basic_permissions() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");

        // Create the file first
        file.write_string("test content", WriteOptions::default())
            .await
            .unwrap();
        let readonly = std::fs::Permissions::from_mode(0o444);
        let readwrite = std::fs::Permissions::from_mode(0o644);
        let executable = std::fs::Permissions::from_mode(0o755);

        // Test read-only (444 in octal)
        file.set_permissions(readonly).await.unwrap();
        let perms = file.permissions().await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o444);

        // Test read-write (644 in octal)
        file.set_permissions(readwrite).await.unwrap();
        let perms = file.permissions().await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o644);

        // Test executable (755 in octal)
        file.set_permissions(executable).await.unwrap();
        let perms = file.permissions().await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o755);
    }
}

pub mod last_modified {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("nonexistent-file");

        // Should fail because file doesn't exist
        assert!(matches!(
            file.last_modified().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::default())
            .await
            .unwrap();
        let modified = file.last_modified().await.unwrap();
        assert!(modified.elapsed().unwrap() < std::time::Duration::from_secs(1));
    }
}

pub mod size {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("nonexistent-file");

        // Should fail because file doesn't exist
        assert!(matches!(
            file.size().await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let file = dir.file("test-file");
        file.write_string("test", WriteOptions::ATOMIC)
            .await
            .unwrap();
        assert_eq!(file.size().await.unwrap(), 4);
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
