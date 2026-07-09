// standard crates
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::{
    self, dirs, files, Atomic, CopyOptions, FileSysErr, Overwrite, PathExt, WriteOptions,
};

// external crates
use secrecy::ExposeSecret;
use serde_json::json;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod assert_exists {
    use super::*;

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        file.assert_exists().unwrap();
    }

    #[test]
    fn failure() {
        let file = filesys::File::new(PathBuf::from("nonexistent").join("path").join("file.txt"));
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
        let file = filesys::File::new(PathBuf::from("nonexistent").join("path").join("file.txt"));
        file.assert_doesnt_exist().unwrap();
    }

    #[tokio::test]
    async fn failure() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
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
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        assert!(file.exists());
        files::delete(&file).await.unwrap();
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let file = filesys::File::new(PathBuf::from("doesnt_exist"));
        assert!(!file.exists());
        files::delete(&file).await.unwrap();
        assert!(!file.exists());
    }
}
pub mod copy_to {
    use super::*;

    #[tokio::test]
    async fn copies_bytes_to_new_destination() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "hello world", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");

        files::copy_to(&src, &dest, CopyOptions::default())
            .await
            .unwrap();

        assert!(src.exists(), "source should still exist after copy");
        assert!(dest.exists(), "destination should exist after copy");
        assert_eq!(files::read_string(&dest).await.unwrap(), "hello world");
        assert_eq!(files::read_string(&src).await.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn same_path_existing_file_succeeds() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "content", WriteOptions::default())
            .await
            .unwrap();

        files::copy_to(&file, &file, CopyOptions::default())
            .await
            .unwrap();
        assert!(file.exists());
        assert_eq!(files::read_string(&file).await.unwrap(), "content");
    }

    #[tokio::test]
    async fn same_path_missing_file_returns_path_does_not_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent");

        assert!(matches!(
            files::copy_to(&file, &file, CopyOptions::default())
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn overwrite_deny_with_existing_dest_returns_invalid_overwrite() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "src", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        files::write_string(&dest, "dest", WriteOptions::default())
            .await
            .unwrap();

        assert!(matches!(
            files::copy_to(&src, &dest, CopyOptions::default())
                .await
                .unwrap_err(),
            FileSysErr::InvalidFileOverwriteErr { .. }
        ));
        // dest content should be unchanged
        assert_eq!(files::read_string(&dest).await.unwrap(), "dest");
    }

    #[tokio::test]
    async fn overwrite_allow_with_existing_dest_overwrites() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "new", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        files::write_string(&dest, "old", WriteOptions::default())
            .await
            .unwrap();

        files::copy_to(&src, &dest, CopyOptions::OVERWRITE_SYNC)
            .await
            .unwrap();
        assert_eq!(files::read_string(&dest).await.unwrap(), "new");
        assert_eq!(files::read_string(&src).await.unwrap(), "new");
    }

    #[tokio::test]
    async fn source_missing_returns_path_does_not_exist() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("nonexistent-src");
        let dest = dir.file("dest");

        assert!(matches!(
            files::copy_to(&src, &dest, CopyOptions::OVERWRITE_SYNC)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn creates_parent_dirs_for_destination() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "nested", WriteOptions::default())
            .await
            .unwrap();
        let dest_dir = dir.subdir("a").subdir("b").subdir("c");
        let dest = dest_dir.file("dest-file");

        assert!(!dest_dir.exists());
        files::copy_to(&src, &dest, CopyOptions::default())
            .await
            .unwrap();
        assert!(dest.exists());
        assert_eq!(files::read_string(&dest).await.unwrap(), "nested");
    }

    #[tokio::test]
    async fn copy_with_sync_yes() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "synced", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");

        let opts = CopyOptions {
            overwrite: Overwrite::Deny,
            sync: filesys::Sync::Yes,
        };
        files::copy_to(&src, &dest, opts).await.unwrap();
        assert_eq!(files::read_string(&dest).await.unwrap(), "synced");
    }

    #[tokio::test]
    async fn copy_with_sync_no() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "unsynced", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");

        let opts = CopyOptions {
            overwrite: Overwrite::Deny,
            sync: filesys::Sync::No,
        };
        files::copy_to(&src, &dest, opts).await.unwrap();
        assert_eq!(files::read_string(&dest).await.unwrap(), "unsynced");
    }

    #[tokio::test]
    async fn overwrite_allow_with_sync_no() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "new", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        files::write_string(&dest, "old", WriteOptions::default())
            .await
            .unwrap();

        files::copy_to(&src, &dest, CopyOptions::OVERWRITE_NO_SYNC)
            .await
            .unwrap();
        assert_eq!(files::read_string(&dest).await.unwrap(), "new");
    }

    #[tokio::test]
    async fn overwrite_deny_with_sync_yes_rejects_existing() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "src", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        files::write_string(&dest, "dest", WriteOptions::default())
            .await
            .unwrap();

        let opts = CopyOptions {
            overwrite: Overwrite::Deny,
            sync: filesys::Sync::Yes,
        };
        assert!(matches!(
            files::copy_to(&src, &dest, opts).await.unwrap_err(),
            FileSysErr::InvalidFileOverwriteErr { .. }
        ));
        assert_eq!(files::read_string(&dest).await.unwrap(), "dest");
    }
}

pub mod move_to {
    use super::*;

    #[tokio::test]
    async fn src_doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // overwrite false
        assert!(matches!(
            files::move_to(&file, &file, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));

        // overwrite true
        assert!(matches!(
            files::move_to(&file, &file, Overwrite::Allow)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "test", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");

        // overwrite false
        assert!(src.exists());
        assert!(!dest.exists());
        files::move_to(&src, &dest, Overwrite::Deny).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());

        // overwrite true
        let tmp = src;
        let src = dest;
        let dest = tmp;
        assert!(src.exists());
        assert!(!dest.exists());
        files::move_to(&src, &dest, Overwrite::Allow).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn dest_exists_overwrite_false() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "src", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        files::write_string(&dest, "dest", WriteOptions::default())
            .await
            .unwrap();

        // overwrite false
        assert!(src.exists());
        assert!(dest.exists());
        assert!(matches!(
            files::move_to(&src, &dest, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::InvalidFileOverwriteErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_exists_overwrite_true() {
        let dir = dirs::temp("testing").unwrap();
        let src = dir.file("src-file");
        files::write_string(&src, "src", WriteOptions::default())
            .await
            .unwrap();
        let dest = dir.file("dest-file");
        files::write_string(&dest, "dest", WriteOptions::default())
            .await
            .unwrap();

        // overwrite false
        assert!(src.exists());
        assert!(dest.exists());
        files::move_to(&src, &dest, Overwrite::Allow).await.unwrap();
        assert!(dest.exists());
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn src_and_dest_are_same_file() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        files::move_to(&file, &file, Overwrite::Deny).await.unwrap();
        file.assert_exists().unwrap();
        files::move_to(&file, &file, Overwrite::Allow)
            .await
            .unwrap();
        assert!(file.exists());
        assert!(files::read_string(&file).await.unwrap() == "test");
    }
}

pub mod read_bytes {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = filesys::File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            files::read_bytes(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "arglebargle", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(files::read_bytes(&file).await.unwrap(), b"arglebargle");
    }
}

pub mod hash {
    use super::*;

    #[tokio::test]
    async fn empty_file() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("empty");
        files::write_string(&file, "", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            files::hash(&file).await.unwrap(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[tokio::test]
    async fn known_vector_hello() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("hello");
        files::write_string(&file, "hello", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            files::hash(&file).await.unwrap(),
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn large_file_multi_read_is_deterministic() {
        // Larger than the 8 KiB streaming buffer, so hashing spans multiple reads.
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("large");
        let contents = "a".repeat(20 * 1024);
        files::write_string(&file, &contents, WriteOptions::default())
            .await
            .unwrap();

        let first = files::hash(&file).await.unwrap();
        let second = files::hash(&file).await.unwrap();
        assert!(first.starts_with("sha256:"));
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");
        assert!(matches!(
            files::hash(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }
}

pub mod read_secret_bytes {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = filesys::File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            files::read_secret_bytes(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "arglebargle", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            files::read_secret_bytes(&file)
                .await
                .unwrap()
                .expose_secret(),
            b"arglebargle"
        );
    }
}

pub mod read_string {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = filesys::File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            files::read_string(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "arglebargle", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(files::read_string(&file).await.unwrap(), "arglebargle");
    }

    #[tokio::test]
    async fn read_invalid_utf8() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        // Write invalid UTF-8 bytes directly
        files::append_bytes(
            &file,
            &[0xFF, 0xFE, 0xFD],
            filesys::AppendOptions::default(),
        )
        .await
        .unwrap();
        assert!(matches!(
            files::read_string(&file).await.unwrap_err(),
            FileSysErr::ConvertUTF8Err { .. }
        ));
    }
}

pub mod read_json {
    use super::*;

    #[tokio::test]
    async fn read_doesnt_exist() {
        let file = filesys::File::new(PathBuf::from("doesnt_exist").join("test-file.json"));
        assert!(matches!(
            files::read_json::<String>(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn read_success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(
            &file,
            "{\"test\": \"arglebargle\"}",
            WriteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            files::read_json::<serde_json::Value>(&file).await.unwrap(),
            serde_json::json!({"test": "arglebargle"})
        );
    }

    #[tokio::test]
    async fn read_invalid_json() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file.json");
        files::write_string(&file, "not valid json {{{", WriteOptions::default())
            .await
            .unwrap();
        assert!(matches!(
            files::read_json::<serde_json::Value>(&file)
                .await
                .unwrap_err(),
            FileSysErr::ParseJSONErr { .. }
        ));
    }
}

pub mod write_bytes {
    use super::*;

    fn write_bytes_atomic(
        file: &filesys::File,
        buf: &[u8],
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let buf = buf.to_vec();
        Box::pin(async move {
            files::write_bytes(
                &file,
                &buf,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::Yes,
                    mode: None,
                },
            )
            .await
        })
    }

    fn write_bytes_non_atomic(
        file: &filesys::File,
        buf: &[u8],
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let buf = buf.to_vec();
        Box::pin(async move {
            files::write_bytes(
                &file,
                &buf,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::No,
                    mode: None,
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn doesnt_exist() {
        let write_funcs = &[write_bytes_atomic, write_bytes_non_atomic];
        for write_bytes in write_funcs {
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_bytes(&file).await.unwrap(), b"arglebargle");
        }
    }

    #[tokio::test]
    async fn parent_doesnt_exist() {
        for write_bytes in [write_bytes_atomic, write_bytes_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
            let file = subdir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_bytes(&file).await.unwrap(), b"arglebargle");
        }
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        for write_bytes in [write_bytes_atomic, write_bytes_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_bytes(&file).await.unwrap(), b"arglebargle");

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
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            write_bytes(&file, b"arglebargle", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_bytes(&file).await.unwrap(), b"arglebargle");

            // should succeed when writing again
            write_bytes(&file, b"arglebargle", Overwrite::Allow)
                .await
                .unwrap();
            assert_eq!(files::read_bytes(&file).await.unwrap(), b"arglebargle");
        }
    }

    #[tokio::test]
    async fn honors_mode_atomic() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_bytes(
            &file,
            b"secret",
            WriteOptions {
                overwrite: Overwrite::Allow,
                atomic: Atomic::Yes,
                mode: Some(0o600),
            },
        )
        .await
        .unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn honors_mode_non_atomic() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_bytes(
            &file,
            b"secret",
            WriteOptions {
                overwrite: Overwrite::Allow,
                atomic: Atomic::No,
                mode: Some(0o600),
            },
        )
        .await
        .unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
}

pub mod write_string {
    use super::*;

    fn write_string_atomic(
        file: &filesys::File,
        s: &str,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let s = s.to_string();
        Box::pin(async move {
            files::write_string(
                &file,
                &s,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::Yes,
                    mode: None,
                },
            )
            .await
        })
    }

    fn write_string_non_atomic(
        file: &filesys::File,
        s: &str,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let s = s.to_string();
        Box::pin(async move {
            files::write_string(
                &file,
                &s,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::No,
                    mode: None,
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn doesnt_exist() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_string(&file).await.unwrap(), "hello world");
        }
    }

    #[tokio::test]
    async fn parent_doesnt_exist() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
            let file = subdir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_string(&file).await.unwrap(), "hello world");
        }
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        for write_string in [write_string_atomic, write_string_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_string(&file).await.unwrap(), "hello world");

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
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            write_string(&file, "hello world", Overwrite::Deny)
                .await
                .unwrap();
            assert_eq!(files::read_string(&file).await.unwrap(), "hello world");

            // should succeed when writing again
            write_string(&file, "new content", Overwrite::Allow)
                .await
                .unwrap();
            assert_eq!(files::read_string(&file).await.unwrap(), "new content");
        }
    }
}

mod write_json {
    use super::*;

    fn write_json_atomic(
        file: &filesys::File,
        data: &serde_json::Value,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let data = data.clone();
        Box::pin(async move {
            files::write_json(
                &file,
                &data,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::Yes,
                    mode: None,
                },
            )
            .await
        })
    }

    fn write_json_non_atomic(
        file: &filesys::File,
        data: &serde_json::Value,
        overwrite: Overwrite,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FileSysErr>> + Send>> {
        let file = file.clone();
        let data = data.clone();
        Box::pin(async move {
            files::write_json(
                &file,
                &data,
                WriteOptions {
                    overwrite,
                    atomic: Atomic::No,
                    mode: None,
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn doesnt_exist() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            let data = json!({
                "name": "test",
                "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = files::read_json(&file).await.unwrap();
            assert_eq!(read_data, data);
        }
    }

    #[tokio::test]
    async fn parent_doesnt_exist() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
            let file = subdir.file("test-file");
            let data = json!({
            "name": "test",
            "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = files::read_json(&file).await.unwrap();
            assert_eq!(read_data, data);
        }
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        for write_json in [write_json_atomic, write_json_non_atomic] {
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            let data = json!({
            "name": "test",
            "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = files::read_json(&file).await.unwrap();
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
            let dir = dirs::temp("testing").unwrap();
            let file = dir.file("test-file");
            let data = json!({
            "name": "test",
            "value": 42
            });
            write_json(&file, &data, Overwrite::Deny).await.unwrap();
            let read_data: serde_json::Value = files::read_json(&file).await.unwrap();
            assert_eq!(read_data, data);

            // should succeed when writing again
            let new_data = json!({
                "name": "updated",
                "value": 100
            });
            write_json(&file, &new_data, Overwrite::Allow)
                .await
                .unwrap();
            let read_data: serde_json::Value = files::read_json(&file).await.unwrap();
            assert_eq!(read_data, new_data);
        }
    }
}

pub mod append_bytes {
    use super::*;

    #[tokio::test]
    async fn creates_file_when_doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::append_bytes(&file, b"hello", filesys::AppendOptions::default())
            .await
            .unwrap();
        assert_eq!(files::read_bytes(&file).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn creates_parent_dirs() {
        let dir = dirs::temp("testing").unwrap();
        let subdir = dir.subdir(PathBuf::from("nested").join("subdir"));
        let file = subdir.file("test-file");
        files::append_bytes(&file, b"hello", filesys::AppendOptions::default())
            .await
            .unwrap();
        assert_eq!(files::read_bytes(&file).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn appends_to_existing_content() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::append_bytes(&file, b"hello ", filesys::AppendOptions::default())
            .await
            .unwrap();
        files::append_bytes(&file, b"world", filesys::AppendOptions::default())
            .await
            .unwrap();
        assert_eq!(files::read_string(&file).await.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn appends_multiple_lines() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::append_bytes(&file, b"line1\n", filesys::AppendOptions::default())
            .await
            .unwrap();
        files::append_bytes(&file, b"line2\n", filesys::AppendOptions::default())
            .await
            .unwrap();
        files::append_bytes(&file, b"line3\n", filesys::AppendOptions::default())
            .await
            .unwrap();
        let content = files::read_string(&file).await.unwrap();
        assert_eq!(content, "line1\nline2\nline3\n");
        assert_eq!(content.lines().count(), 3);
    }

    #[tokio::test]
    async fn sync_option_writes_and_persists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::append_bytes(&file, b"first\n", filesys::AppendOptions::SYNC)
            .await
            .unwrap();
        files::append_bytes(&file, b"second\n", filesys::AppendOptions::SYNC)
            .await
            .unwrap();
        assert_eq!(files::read_string(&file).await.unwrap(), "first\nsecond\n");
    }

    #[tokio::test]
    async fn empty_buffer_creates_file() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::append_bytes(&file, b"", filesys::AppendOptions::default())
            .await
            .unwrap();
        assert!(file.exists());
        assert_eq!(files::read_bytes(&file).await.unwrap(), b"");
    }

    #[tokio::test]
    async fn empty_buffer_preserves_existing_content() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::append_bytes(&file, b"existing", filesys::AppendOptions::default())
            .await
            .unwrap();
        files::append_bytes(&file, b"", filesys::AppendOptions::default())
            .await
            .unwrap();
        assert_eq!(files::read_string(&file).await.unwrap(), "existing");
    }
}

pub mod set_permissions {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");
        let permissions = std::fs::Permissions::from_mode(0o644);

        // Should fail because file doesn't exist
        assert!(matches!(
            files::set_permissions(&file, permissions)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn basic_permissions() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // Create the file first
        files::write_string(&file, "test content", WriteOptions::default())
            .await
            .unwrap();
        let readonly = std::fs::Permissions::from_mode(0o444);
        let readwrite = std::fs::Permissions::from_mode(0o644);
        let executable = std::fs::Permissions::from_mode(0o755);

        // Test read-only (444 in octal)
        files::set_permissions(&file, readonly).await.unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o444);

        // Test read-write (644 in octal)
        files::set_permissions(&file, readwrite).await.unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o644);

        // Test executable (755 in octal)
        files::set_permissions(&file, executable).await.unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_permission_combinations() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test content", WriteOptions::default())
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
            files::set_permissions(&file, expected.clone())
                .await
                .unwrap();
            let actual = files::permissions(&file).await.unwrap();
            assert_eq!(actual.mode() & 0o777, expected.mode() & 0o777);
        }
    }
}

pub mod create_symlink {
    use super::*;

    #[tokio::test]
    async fn src_doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");
        let link = dir.file("link");
        assert!(matches!(
            files::create_symlink(&file, &link, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_doesnt_exist_overwrite_false() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");

        // overwrite false
        files::create_symlink(&file, &link, Overwrite::Deny)
            .await
            .unwrap();
        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
    }

    #[tokio::test]
    async fn dest_doesnt_exist_overwrite_true() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");

        // overwrite true
        files::create_symlink(&file, &link, Overwrite::Allow)
            .await
            .unwrap();
        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
    }

    #[tokio::test]
    async fn dest_exists_overwrite_false() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");
        files::create_symlink(&file, &link, Overwrite::Allow)
            .await
            .unwrap();

        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
        assert!(matches!(
            files::create_symlink(&file, &link, Overwrite::Deny)
                .await
                .unwrap_err(),
            FileSysErr::InvalidFileOverwriteErr { .. }
        ));
    }

    #[tokio::test]
    async fn dest_exists_overwrite_true() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        let link = dir.file("link");
        files::create_symlink(&file, &link, Overwrite::Allow)
            .await
            .unwrap();

        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
        files::create_symlink(&file, &link, Overwrite::Allow)
            .await
            .unwrap();
        file.assert_exists().unwrap();
        link.assert_exists().unwrap();
    }
}

pub mod glob {
    use super::*;
    use std::fs;

    #[test]
    fn matches_regular_file() {
        let tmp = dirs::temp("testing").unwrap();
        let path = tmp.path().join("data.txt");
        fs::write(&path, b"hello world").unwrap();

        let pattern = tmp.path().join("*.txt");
        let matches = files::glob(pattern.to_str().unwrap()).unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], filesys::File::new(path));
    }

    #[test]
    fn non_matching_file_absent() {
        let tmp = dirs::temp("testing").unwrap();
        fs::write(tmp.path().join("data.log"), b"nope").unwrap();

        let pattern = tmp.path().join("*.txt");
        let matches = files::glob(pattern.to_str().unwrap()).unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn directory_matched_by_pattern_skipped() {
        let tmp = dirs::temp("testing").unwrap();
        fs::create_dir(tmp.path().join("subdir")).unwrap();
        let file_path = tmp.path().join("file");
        fs::write(&file_path, b"x").unwrap();

        let pattern = tmp.path().join("*");
        let matches = files::glob(pattern.to_str().unwrap()).unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], filesys::File::new(file_path));
    }

    #[test]
    fn invalid_glob_returns_error() {
        assert!(matches!(
            files::glob("[unclosed").unwrap_err(),
            FileSysErr::InvalidGlobErr { .. }
        ));
    }
}

pub mod metadata {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");

        assert!(matches!(
            files::metadata(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        let contents = "hello";
        files::write_string(&file, contents, WriteOptions::default())
            .await
            .unwrap();

        let metadata = files::metadata(&file).await.unwrap();
        assert!(metadata.is_file());
        assert!(!metadata.is_dir());
        assert_eq!(metadata.len(), contents.len() as u64);
        assert!(
            metadata.modified().unwrap().elapsed().unwrap() < std::time::Duration::from_secs(1)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reflects_set_permissions() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();

        let readonly = std::fs::Permissions::from_mode(0o444);
        files::set_permissions(&file, readonly).await.unwrap();

        let metadata = files::metadata(&file).await.unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o444);
    }
}

pub mod permissions {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");

        // Should fail because file doesn't exist
        assert!(matches!(
            files::permissions(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn basic_permissions() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // Create the file first
        files::write_string(&file, "test content", WriteOptions::default())
            .await
            .unwrap();
        let readonly = std::fs::Permissions::from_mode(0o444);
        let readwrite = std::fs::Permissions::from_mode(0o644);
        let executable = std::fs::Permissions::from_mode(0o755);

        // Test read-only (444 in octal)
        files::set_permissions(&file, readonly).await.unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o444);

        // Test read-write (644 in octal)
        files::set_permissions(&file, readwrite).await.unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o644);

        // Test executable (755 in octal)
        files::set_permissions(&file, executable).await.unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_eq!(perms.mode() & 0o777, 0o755);
    }
}

pub mod last_modified {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");

        // Should fail because file doesn't exist
        assert!(matches!(
            files::last_modified(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::default())
            .await
            .unwrap();
        let modified = files::last_modified(&file).await.unwrap();
        assert!(modified.elapsed().unwrap() < std::time::Duration::from_secs(1));
    }
}

pub mod size {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("nonexistent-file");

        // Should fail because file doesn't exist
        assert!(matches!(
            files::size(&file).await.unwrap_err(),
            FileSysErr::PathDoesNotExistErr { .. }
        ));
    }

    #[tokio::test]
    async fn success() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        files::write_string(&file, "test", WriteOptions::ATOMIC)
            .await
            .unwrap();
        assert_eq!(files::size(&file).await.unwrap(), 4);
    }
}
