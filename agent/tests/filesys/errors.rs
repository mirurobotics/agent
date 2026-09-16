// internal crates
use crate::test_utils::filesys::abs_path;
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
            dir: filesys::Dir::new("/"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("directory name"), "message: {msg}");
    }

    #[test]
    fn path_does_not_exist_err() {
        let path = abs_path("missing/path");
        let expected_path = path.display().to_string();
        let err = FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
            path,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_path.as_str()), "message: {msg}");
    }

    #[test]
    fn path_existence_err() {
        let path = abs_path("unreadable/path");
        let expected_path = path.display().to_string();
        let err = FileSysErr::PathExistenceErr(PathExistenceErr {
            path,
            source: Box::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_path.as_str()), "message: {msg}");
        assert!(
            msg.contains(&std::io::Error::from(std::io::ErrorKind::PermissionDenied).to_string()),
            "message must name the underlying cause: {msg}"
        );
    }

    #[test]
    fn path_exists_err() {
        let path = abs_path("exists/path");
        let expected_path = path.display().to_string();
        let err = FileSysErr::PathExistsErr(PathExistsErr {
            path,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_path.as_str()), "message: {msg}");
    }

    #[test]
    fn invalid_file_overwrite_err() {
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
            file,
            overwrite: filesys::Overwrite::Deny,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
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
            dir: filesys::Dir::new("/"),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains("parent directory"), "message: {msg}");
    }

    #[test]
    fn parse_json_err() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let file = filesys::File::new("/some/file.json");
        let expected_file = file.to_string();
        let err = FileSysErr::ParseJSONErr(ParseJSONErr {
            source: Box::new(serde_err),
            file,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
        assert!(msg.contains("JSON"), "message: {msg}");
    }

    #[test]
    fn read_file_err() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::ReadFileErr(ReadFileErr {
            source: Box::new(io_err),
            file,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
    }

    #[test]
    fn write_file_err() {
        let io_err = std::io::Error::other("disk full");
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::WriteFileErr(WriteFileErr {
            source: Box::new(io_err),
            file,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
    }

    #[test]
    fn open_file_err() {
        let io_err = std::io::Error::other("i/o error");
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::OpenFileErr(OpenFileErr {
            source: Box::new(io_err),
            file,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
    }

    #[test]
    fn create_dir_err() {
        let io_err = std::io::Error::other("mkdir failed");
        let dir = filesys::Dir::new("/some/dir");
        let expected_dir = dir.to_string();
        let err = FileSysErr::CreateDirErr(CreateDirErr {
            source: Box::new(io_err),
            dir,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_dir.as_str()), "message: {msg}");
    }

    #[test]
    fn delete_dir_err() {
        let io_err = std::io::Error::other("rmdir failed");
        let dir = filesys::Dir::new("/some/dir");
        let expected_dir = dir.to_string();
        let err = FileSysErr::DeleteDirErr(DeleteDirErr {
            source: Box::new(io_err),
            dir,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_dir.as_str()), "message: {msg}");
    }

    #[test]
    fn delete_file_err() {
        let io_err = std::io::Error::other("rm failed");
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::DeleteFileErr(DeleteFileErr {
            source: Box::new(io_err),
            file,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
    }

    #[test]
    fn move_file_err() {
        let io_err = std::io::Error::other("rename failed");
        let src_file = filesys::File::new("/src");
        let dest_file = filesys::File::new("/dest");
        let expected_src = src_file.to_string();
        let expected_dest = dest_file.to_string();
        let err = FileSysErr::MoveFileErr(MoveFileErr {
            source: Box::new(io_err),
            src_file,
            dest_file,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_src.as_str()), "message: {msg}");
        assert!(msg.contains(expected_dest.as_str()), "message: {msg}");
    }

    #[test]
    fn move_dir_err() {
        let io_err = std::io::Error::other("rename failed");
        let src_dir = filesys::Dir::new("/src");
        let dest_dir = filesys::Dir::new("/dest");
        let expected_src = src_dir.to_string();
        let expected_dest = dest_dir.to_string();
        let err = FileSysErr::MoveDirErr(MoveDirErr {
            source: Box::new(io_err),
            src_dir,
            dest_dir,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_src.as_str()), "message: {msg}");
        assert!(msg.contains(expected_dest.as_str()), "message: {msg}");
    }

    #[test]
    fn file_metadata_err() {
        let io_err = std::io::Error::other("stat failed");
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::FileMetadataErr(FileMetadataErr {
            file,
            source: Box::new(io_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
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
        let file = filesys::File::new("/some/file");
        let expected_file = file.to_string();
        let err = FileSysErr::AtomicWriteFileErr(AtomicWriteFileErr {
            file,
            source: Box::new(io_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_file.as_str()), "message: {msg}");
        assert!(msg.contains("atomically"), "message: {msg}");
    }

    #[test]
    fn read_dir_err() {
        let io_err = std::io::Error::other("readdir failed");
        let dir = filesys::Dir::new("/some/dir");
        let expected_dir = dir.to_string();
        let err = FileSysErr::ReadDirErr(ReadDirErr {
            dir,
            source: Box::new(io_err),
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_dir.as_str()), "message: {msg}");
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
        let src_dir = filesys::Dir::new("/src");
        let dest_dir = filesys::Dir::new("/dest");
        let trash_dir = filesys::Dir::new("/trash");
        let expected_src = src_dir.to_string();
        let expected_dest = dest_dir.to_string();
        let expected_trash = trash_dir.to_string();
        let err = FileSysErr::MoveDirRollbackErr(MoveDirRollbackErr {
            primary_source: Box::new(primary),
            rollback_source: Box::new(rollback),
            cleanup_source: None,
            src_dir,
            dest_dir,
            trash_dir,
            trace: miru_agent::trace!(),
        });
        let msg = err.to_string();
        assert!(msg.contains(expected_src.as_str()), "message: {msg}");
        assert!(msg.contains(expected_dest.as_str()), "message: {msg}");
        assert!(msg.contains(expected_trash.as_str()), "message: {msg}");
        assert!(msg.contains("rollback"), "message: {msg}");
    }
}
