// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::filesys;
use miru_agent::filesys::errors::*;

/// Verify that error Display implementations produce meaningful messages
/// containing the relevant context (file paths, directory names, etc.).
mod display {
    use super::*;

    #[test]
    fn invalid_dir_name_err() {
        let err = FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
            name: "bad/dir".to_string(),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("bad/dir"), "message: {msg}");
    }

    #[test]
    fn unknown_dir_name_err() {
        let err = FileSysErr::UnknownDirNameErr(UnknownDirNameErr {
            dir: filesys::Dir::new(PathBuf::from("/")),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("directory name"), "message: {msg}");
    }

    #[test]
    fn path_does_not_exist_err() {
        let err = FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
            path: PathBuf::from("/missing/path"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/missing/path"), "message: {msg}");
    }

    #[test]
    fn path_exists_err() {
        let err = FileSysErr::PathExistsErr(PathExistsErr {
            path: PathBuf::from("/exists/path"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/exists/path"), "message: {msg}");
    }

    #[test]
    fn invalid_file_overwrite_err() {
        let err = FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
            file: filesys::File::new("/some/file"),
            overwrite: filesys::Overwrite::Deny,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
        assert!(msg.contains("overwrite"), "message: {msg}");
    }

    #[test]
    fn unknown_parent_dir_for_file_err() {
        let err = FileSysErr::UnknownParentDirForFileErr(UnknownParentDirForFileErr {
            file: filesys::File::new("/orphan"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("parent directory"), "message: {msg}");
    }

    #[test]
    fn unknown_parent_dir_for_dir_err() {
        let err = FileSysErr::UnknownParentDirForDirErr(UnknownParentDirForDirErr {
            dir: filesys::Dir::new(PathBuf::from("/")),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("parent directory"), "message: {msg}");
    }

    #[test]
    fn parse_json_err() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let err = FileSysErr::ParseJSONErr(ParseJSONErr {
            source: Box::new(serde_err),
            file: filesys::File::new("/some/file.json"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file.json"), "message: {msg}");
        assert!(msg.contains("JSON"), "message: {msg}");
    }

    #[test]
    fn read_file_err() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let err = FileSysErr::ReadFileErr(ReadFileErr {
            source: Box::new(io_err),
            file: filesys::File::new("/some/file"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
    }

    #[test]
    fn write_file_err() {
        let io_err = std::io::Error::other("disk full");
        let err = FileSysErr::WriteFileErr(WriteFileErr {
            source: Box::new(io_err),
            file: filesys::File::new("/some/file"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
    }

    #[test]
    fn open_file_err() {
        let io_err = std::io::Error::other("i/o error");
        let err = FileSysErr::OpenFileErr(OpenFileErr {
            source: Box::new(io_err),
            file: filesys::File::new("/some/file"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
    }

    #[test]
    fn create_dir_err() {
        let io_err = std::io::Error::other("mkdir failed");
        let err = FileSysErr::CreateDirErr(CreateDirErr {
            source: Box::new(io_err),
            dir: filesys::Dir::new(PathBuf::from("/some/dir")),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/dir"), "message: {msg}");
    }

    #[test]
    fn delete_dir_err() {
        let io_err = std::io::Error::other("rmdir failed");
        let err = FileSysErr::DeleteDirErr(DeleteDirErr {
            source: Box::new(io_err),
            dir: filesys::Dir::new(PathBuf::from("/some/dir")),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/dir"), "message: {msg}");
    }

    #[test]
    fn delete_file_err() {
        let io_err = std::io::Error::other("rm failed");
        let err = FileSysErr::DeleteFileErr(DeleteFileErr {
            source: Box::new(io_err),
            file: filesys::File::new("/some/file"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
    }

    #[test]
    fn move_file_err() {
        let io_err = std::io::Error::other("rename failed");
        let err = FileSysErr::MoveFileErr(MoveFileErr {
            source: Box::new(io_err),
            src_file: filesys::File::new("/src"),
            dest_file: filesys::File::new("/dest"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/src"), "message: {msg}");
        assert!(msg.contains("/dest"), "message: {msg}");
    }

    #[test]
    fn move_dir_err() {
        let io_err = std::io::Error::other("rename failed");
        let err = FileSysErr::MoveDirErr(MoveDirErr {
            source: Box::new(io_err),
            src_dir: filesys::Dir::new(PathBuf::from("/src")),
            dest_dir: filesys::Dir::new(PathBuf::from("/dest")),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/src"), "message: {msg}");
        assert!(msg.contains("/dest"), "message: {msg}");
    }

    #[test]
    fn create_symlink_err() {
        let io_err = std::io::Error::other("symlink failed");
        let err = FileSysErr::CreateSymlinkErr(CreateSymlinkErr {
            source: Box::new(io_err),
            file: filesys::File::new("/src"),
            link: filesys::File::new("/link"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/src"), "message: {msg}");
        assert!(msg.contains("/link"), "message: {msg}");
    }

    #[test]
    fn file_metadata_err() {
        let io_err = std::io::Error::other("stat failed");
        let err = FileSysErr::FileMetadataErr(FileMetadataErr {
            file: filesys::File::new("/some/file"),
            source: Box::new(io_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
        assert!(msg.contains("metadata"), "message: {msg}");
    }

    #[test]
    fn convert_utf8_err() {
        let bytes = vec![0xFF_u8];
        let utf8_err = std::str::from_utf8(&bytes).unwrap_err();
        let err = FileSysErr::ConvertUTF8Err(ConvertUTF8Err {
            source: Box::new(utf8_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("UTF-8"), "message: {msg}");
    }

    #[test]
    fn atomic_write_file_err() {
        let io_err = std::io::Error::other("atomic write failed");
        let err = FileSysErr::AtomicWriteFileErr(AtomicWriteFileErr {
            file: filesys::File::new("/some/file"),
            source: Box::new(io_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/file"), "message: {msg}");
        assert!(msg.contains("atomically"), "message: {msg}");
    }

    #[test]
    fn read_dir_err() {
        let io_err = std::io::Error::other("readdir failed");
        let err = FileSysErr::ReadDirErr(ReadDirErr {
            dir: filesys::Dir::new(PathBuf::from("/some/dir")),
            source: Box::new(io_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/some/dir"), "message: {msg}");
    }

    #[test]
    fn unknown_file_name_err() {
        let err = FileSysErr::UnknownFileNameErr(UnknownFileNameErr {
            file: filesys::File::new("/"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("file name"), "message: {msg}");
    }

    #[test]
    fn move_dir_rollback_err() {
        let primary = std::io::Error::other("rename failed");
        let rollback = std::io::Error::other("rollback failed");
        let err = FileSysErr::MoveDirRollbackErr(MoveDirRollbackErr {
            primary_source: Box::new(primary),
            rollback_source: Box::new(rollback),
            cleanup_source: None,
            src_dir: filesys::Dir::new(PathBuf::from("/src")),
            dest_dir: filesys::Dir::new(PathBuf::from("/dest")),
            trash_dir: filesys::Dir::new(PathBuf::from("/trash")),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("/src"), "message: {msg}");
        assert!(msg.contains("/dest"), "message: {msg}");
        assert!(msg.contains("/trash"), "message: {msg}");
        assert!(msg.contains("rollback"), "message: {msg}");
    }
}
