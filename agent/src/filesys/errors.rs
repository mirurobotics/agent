// standard library
use std::path::PathBuf;

// internal crates
use crate::errors::Trace;
use crate::filesys::Overwrite;
use crate::filesys::{dir::Dir, file::File};

#[derive(Debug, thiserror::Error)]
#[error("unable to find home directory: {source}")]
pub struct UnknownHomeDirErr {
    pub source: Box<std::env::VarError>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnknownHomeDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("invalid directory name: {name}")]
pub struct InvalidDirNameErr {
    pub name: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidDirNameErr {}

#[derive(Debug, thiserror::Error)]
#[error("unable to determine directory name for directory path: {dir}")]
pub struct UnknownDirNameErr {
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnknownDirNameErr {}

#[derive(Debug, thiserror::Error)]
#[error("unable to determine file name for file path: {file}")]
pub struct UnknownFileNameErr {
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnknownFileNameErr {}

#[derive(Debug, thiserror::Error)]
#[error("path does not exist: {path}")]
pub struct PathDoesNotExistErr {
    pub path: PathBuf,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for PathDoesNotExistErr {}

#[derive(Debug, thiserror::Error)]
#[error("path exists: {path}")]
pub struct PathExistsErr {
    pub path: PathBuf,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for PathExistsErr {}

#[derive(Debug, thiserror::Error)]
#[error("cannot overwrite existing file (overwrite is {overwrite:?}): {file}")]
pub struct InvalidFileOverwriteErr {
    pub file: File,
    pub overwrite: Overwrite,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidFileOverwriteErr {}

#[derive(Debug, thiserror::Error)]
#[error("unable to determine parent directory for directory: {dir}")]
pub struct UnknownParentDirForDirErr {
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnknownParentDirForDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("unable to determine parent directory for file: {file}")]
pub struct UnknownParentDirForFileErr {
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnknownParentDirForFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to read directory: {dir}")]
pub struct ReadDirErr {
    pub dir: Dir,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReadDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to write file atomically: {file}")]
pub struct AtomicWriteFileErr {
    pub file: File,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for AtomicWriteFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("UTF-8 conversion failed: {source}")]
pub struct ConvertUTF8Err {
    pub source: Box<std::str::Utf8Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConvertUTF8Err {}

#[derive(Debug, thiserror::Error)]
#[error("failed to create directory '{dir}': {source}")]
pub struct CreateDirErr {
    pub source: Box<std::io::Error>,
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CreateDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to create symlink '{link}' aliasing to file '{file}': {source}")]
pub struct CreateSymlinkErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub link: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CreateSymlinkErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to create temporary directory: {source}")]
pub struct CreateTmpDirErr {
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CreateTmpDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to delete directory '{dir}': {source}")]
pub struct DeleteDirErr {
    pub source: Box<std::io::Error>,
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DeleteDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to delete file '{file}': {source}")]
pub struct DeleteFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DeleteFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to extract file metadata for file '{file}': {source}")]
pub struct FileMetadataErr {
    pub file: File,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for FileMetadataErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to move file '{src_file}' to '{dest_file}': {source}")]
pub struct MoveFileErr {
    pub source: Box<std::io::Error>,
    pub src_file: File,
    pub dest_file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MoveFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to move directory '{src_dir}' to '{dest_dir}': {source}")]
pub struct MoveDirErr {
    pub source: Box<std::io::Error>,
    pub src_dir: Dir,
    pub dest_dir: Dir,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MoveDirErr {}

#[derive(Debug, thiserror::Error)]
#[error(
    "failed to move directory '{src_dir}' to '{dest_dir}' and rollback failed (trash: '{trash_dir}'): primary={primary_source}; rollback={rollback_source}; cleanup={cleanup_source:?}"
)]
pub struct MoveDirRollbackErr {
    pub primary_source: Box<std::io::Error>,
    pub rollback_source: Box<std::io::Error>,
    pub cleanup_source: Option<Box<std::io::Error>>,
    pub src_dir: Dir,
    pub dest_dir: Dir,
    pub trash_dir: Dir,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MoveDirRollbackErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to open file '{file}': {source}")]
pub struct OpenFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for OpenFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to parse JSON for file '{file}': {source}")]
pub struct ParseJSONErr {
    pub source: Box<serde_json::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ParseJSONErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to read file '{file}': {source}")]
pub struct ReadFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReadFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to determine current directory: {source}")]
pub struct UnknownCurrentDirErr {
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnknownCurrentDirErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to write to file '{file}': {source}")]
pub struct WriteFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for WriteFileErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to send actor message: {source:?}")]
pub struct SendActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SendActorMessageErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to receive actor message: {source}")]
pub struct ReceiveActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReceiveActorMessageErr {}

#[derive(Debug, thiserror::Error)]
pub enum FileSysErr {
    #[error(transparent)]
    InvalidDirNameErr(InvalidDirNameErr),
    #[error(transparent)]
    UnknownDirNameErr(UnknownDirNameErr),
    #[error(transparent)]
    InvalidFileOverwriteErr(InvalidFileOverwriteErr),
    #[error(transparent)]
    UnknownFileNameErr(UnknownFileNameErr),
    #[error(transparent)]
    PathDoesNotExistErr(PathDoesNotExistErr),
    #[error(transparent)]
    PathExistsErr(PathExistsErr),
    #[error(transparent)]
    UnknownParentDirForDirErr(UnknownParentDirForDirErr),
    #[error(transparent)]
    UnknownParentDirForFileErr(UnknownParentDirForFileErr),
    #[error(transparent)]
    AtomicWriteFileErr(AtomicWriteFileErr),
    #[error(transparent)]
    ConvertUTF8Err(ConvertUTF8Err),
    #[error(transparent)]
    CreateDirErr(CreateDirErr),
    #[error(transparent)]
    CreateSymlinkErr(CreateSymlinkErr),
    #[error(transparent)]
    CreateTmpDirErr(CreateTmpDirErr),
    #[error(transparent)]
    DeleteDirErr(DeleteDirErr),
    #[error(transparent)]
    DeleteFileErr(DeleteFileErr),
    #[error(transparent)]
    FileMetadataErr(FileMetadataErr),
    #[error(transparent)]
    MoveFileErr(MoveFileErr),
    #[error(transparent)]
    MoveDirErr(MoveDirErr),
    #[error(transparent)]
    MoveDirRollbackErr(MoveDirRollbackErr),
    #[error(transparent)]
    OpenFileErr(OpenFileErr),
    #[error(transparent)]
    ParseJSONErr(ParseJSONErr),
    #[error(transparent)]
    ReadDirErr(ReadDirErr),
    #[error(transparent)]
    ReadFileErr(ReadFileErr),
    #[error(transparent)]
    UnknownCurrentDirErr(UnknownCurrentDirErr),
    #[error(transparent)]
    UnknownHomeDirErr(UnknownHomeDirErr),
    #[error(transparent)]
    WriteFileErr(WriteFileErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(FileSysErr {
    InvalidDirNameErr,
    UnknownDirNameErr,
    InvalidFileOverwriteErr,
    UnknownFileNameErr,
    PathDoesNotExistErr,
    PathExistsErr,
    UnknownParentDirForDirErr,
    UnknownParentDirForFileErr,
    AtomicWriteFileErr,
    ConvertUTF8Err,
    CreateDirErr,
    CreateSymlinkErr,
    CreateTmpDirErr,
    DeleteDirErr,
    DeleteFileErr,
    FileMetadataErr,
    MoveFileErr,
    MoveDirErr,
    MoveDirRollbackErr,
    OpenFileErr,
    ParseJSONErr,
    ReadDirErr,
    ReadFileErr,
    UnknownCurrentDirErr,
    UnknownHomeDirErr,
    WriteFileErr,
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
