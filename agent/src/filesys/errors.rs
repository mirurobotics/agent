// standard library
use std::fmt;
use std::path::PathBuf;

// internal crates
use crate::errors::{Code, HTTPCode, Error, Trace};
use crate::filesys::{dir::Dir, file::File};

#[derive(Debug)]
pub struct UnknownHomeDirErr {
    pub source: Box<std::env::VarError>,
    pub trace: Box<Trace>,
}

impl Error for UnknownHomeDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnknownHomeDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unable to find home directory: {}", self.source)
    }
}

#[derive(Debug)]
pub struct InvalidDirNameErr {
    pub name: String,
    pub trace: Box<Trace>,
}

impl Error for InvalidDirNameErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for InvalidDirNameErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid directory name: {}", self.name)
    }
}

#[derive(Debug)]
pub struct UnknownDirNameErr {
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl Error for UnknownDirNameErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnknownDirNameErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unable to determine directory name for directory path: {}",
            self.dir
        )
    }
}

#[derive(Debug)]
pub struct UnknownFileNameErr {
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for UnknownFileNameErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnknownFileNameErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unable to determine file name for file path: {}",
            self.file
        )
    }
}

#[derive(Debug)]
pub struct PathDoesNotExistErr {
    pub path: PathBuf,
    pub trace: Box<Trace>,
}

impl Error for PathDoesNotExistErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for PathDoesNotExistErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "path does not exist: {}",
            self.path.to_str().unwrap_or("unknown")
        )
    }
}

#[derive(Debug)]
pub struct PathExistsErr {
    pub path: PathBuf,
    pub trace: Box<Trace>,
}

impl Error for PathExistsErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for PathExistsErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "path exists: {}",
            self.path.to_str().unwrap_or("unknown")
        )
    }
}

#[derive(Debug)]
pub struct InvalidFileOverwriteErr {
    pub file: File,
    pub overwrite: bool,
    pub trace: Box<Trace>,
}

impl Error for InvalidFileOverwriteErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for InvalidFileOverwriteErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot overwrite existing file (allow overwrite is {}): {}",
            self.overwrite, self.file
        )
    }
}

#[derive(Debug)]
pub struct UnknownParentDirForDirErr {
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl Error for UnknownParentDirForDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnknownParentDirForDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unable to determine parent directory for directory: {}",
            self.dir
        )
    }
}

#[derive(Debug)]
pub struct UnknownParentDirForFileErr {
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for UnknownParentDirForFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnknownParentDirForFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unable to determine parent directory for file: {}",
            self.file
        )
    }
}

#[derive(Debug)]
pub struct ReadDirErr {
    pub dir: Dir,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl Error for ReadDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ReadDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to read directory: {}", self.dir)
    }
}

#[derive(Debug)]
pub struct AtomicWriteFileErr {
    pub file: File,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl Error for AtomicWriteFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for AtomicWriteFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to write file atomically: {}", self.file)
    }
}

#[derive(Debug)]
pub struct ConvertUTF8Err {
    pub source: Box<std::str::Utf8Error>,
    pub trace: Box<Trace>,
}

impl Error for ConvertUTF8Err {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ConvertUTF8Err {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UTF-8 conversion failed: {}", self.source)
    }
}

#[derive(Debug)]
pub struct CopyFileErr {
    pub source: Box<std::io::Error>,
    pub src_file: File,
    pub dest_file: File,
    pub trace: Box<Trace>,
}

impl Error for CopyFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for CopyFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to copy file '{}' to '{}': {}",
            self.src_file, self.dest_file, self.source
        )
    }
}

#[derive(Debug)]
pub struct CreateDirErr {
    pub source: Box<std::io::Error>,
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl Error for CreateDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for CreateDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to create directory '{}': {}",
            self.dir, self.source
        )
    }
}

#[derive(Debug)]
pub struct CreateSymlinkErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub link: File,
    pub trace: Box<Trace>,
}

impl Error for CreateSymlinkErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for CreateSymlinkErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to create symlink '{}' aliasing to file '{}': {}",
            self.link, self.file, self.source
        )
    }
}

#[derive(Debug)]
pub struct CreateTmpDirErr {
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl Error for CreateTmpDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for CreateTmpDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to create temporary directory: {}", self.source)
    }
}

#[derive(Debug)]
pub struct DeleteDirErr {
    pub source: Box<std::io::Error>,
    pub dir: Dir,
    pub trace: Box<Trace>,
}

impl Error for DeleteDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DeleteDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to delete directory '{}': {}",
            self.dir, self.source
        )
    }
}

#[derive(Debug)]
pub struct DeleteFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for DeleteFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DeleteFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to delete file '{}': {}", self.file, self.source)
    }
}

#[derive(Debug)]
pub struct FileMetadataErr {
    pub file: File,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl Error for FileMetadataErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for FileMetadataErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to extract file metadata for file '{}': {}",
            self.file, self.source
        )
    }
}

#[derive(Debug)]
pub struct MoveFileErr {
    pub source: Box<std::io::Error>,
    pub src_file: File,
    pub dest_file: File,
    pub trace: Box<Trace>,
}

impl Error for MoveFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for MoveFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to move file '{}' to '{}': {}",
            self.src_file, self.dest_file, self.source
        )
    }
}

#[derive(Debug)]
pub struct MoveDirErr {
    pub source: Box<std::io::Error>,
    pub src_dir: Dir,
    pub dest_dir: Dir,
    pub trace: Box<Trace>,
}

impl Error for MoveDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for MoveDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to move directory '{}' to '{}': {}",
            self.src_dir, self.dest_dir, self.source
        )
    }
}

#[derive(Debug)]
pub struct OpenFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for OpenFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for OpenFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to open file '{}': {}", self.file, self.source)
    }
}

#[derive(Debug)]
pub struct ParseJSONErr {
    pub source: Box<serde_json::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for ParseJSONErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ParseJSONErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to parse JSON for file '{}': {}",
            self.file, self.source
        )
    }
}

#[derive(Debug)]
pub struct ReadFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for ReadFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ReadFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to read file '{}': {}", self.file, self.source)
    }
}

#[derive(Debug)]
pub struct UnknownCurrentDirErr {
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl Error for UnknownCurrentDirErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for UnknownCurrentDirErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to determine current directory: {}", self.source)
    }
}

#[derive(Debug)]
pub struct WriteFileErr {
    pub source: Box<std::io::Error>,
    pub file: File,
    pub trace: Box<Trace>,
}

impl Error for WriteFileErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for WriteFileErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to write to file '{}': {}",
            self.file, self.source
        )
    }
}

#[derive(Debug)]
pub struct SendActorMessageErr {
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl Error for SendActorMessageErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for SendActorMessageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to send actor message: {:?}", self.source)
    }
}

#[derive(Debug)]
pub struct ReceiveActorMessageErr {
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl Error for ReceiveActorMessageErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ReceiveActorMessageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to receive actor message: {}", self.source)
    }
}

#[derive(Debug)]
pub enum FileSysErr {
    InvalidDirNameErr(Box<InvalidDirNameErr>),
    UnknownDirNameErr(Box<UnknownDirNameErr>),
    InvalidFileOverwriteErr(Box<InvalidFileOverwriteErr>),
    UnknownFileNameErr(Box<UnknownFileNameErr>),
    PathDoesNotExistErr(Box<PathDoesNotExistErr>),
    PathExistsErr(Box<PathExistsErr>),
    UnknownParentDirForDirErr(Box<UnknownParentDirForDirErr>),
    UnknownParentDirForFileErr(Box<UnknownParentDirForFileErr>),

    // internal crate errors

    // external crate errors
    AtomicWriteFileErr(Box<AtomicWriteFileErr>),
    ConvertUTF8Err(Box<ConvertUTF8Err>),
    CopyFileErr(Box<CopyFileErr>),
    CreateDirErr(Box<CreateDirErr>),
    CreateSymlinkErr(Box<CreateSymlinkErr>),
    CreateTmpDirErr(Box<CreateTmpDirErr>),
    DeleteDirErr(Box<DeleteDirErr>),
    DeleteFileErr(Box<DeleteFileErr>),
    FileMetadataErr(Box<FileMetadataErr>),
    MoveFileErr(Box<MoveFileErr>),
    MoveDirErr(Box<MoveDirErr>),
    OpenFileErr(Box<OpenFileErr>),
    ParseJSONErr(Box<ParseJSONErr>),
    ReadDirErr(Box<ReadDirErr>),
    ReadFileErr(Box<ReadFileErr>),
    UnknownCurrentDirErr(Box<UnknownCurrentDirErr>),
    UnknownHomeDirErr(Box<UnknownHomeDirErr>),
    WriteFileErr(Box<WriteFileErr>),
    SendActorMessageErr(Box<SendActorMessageErr>),
    ReceiveActorMessageErr(Box<ReceiveActorMessageErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::InvalidDirNameErr(e) => e.$method($($arg)?),
            Self::UnknownDirNameErr(e) => e.$method($($arg)?),
            Self::InvalidFileOverwriteErr(e) => e.$method($($arg)?),
            Self::UnknownFileNameErr(e) => e.$method($($arg)?),
            Self::PathDoesNotExistErr(e) => e.$method($($arg)?),
            Self::PathExistsErr(e) => e.$method($($arg)?),
            Self::UnknownParentDirForDirErr(e) => e.$method($($arg)?),
            Self::UnknownParentDirForFileErr(e) => e.$method($($arg)?),
            Self::AtomicWriteFileErr(e) => e.$method($($arg)?),
            Self::ConvertUTF8Err(e) => e.$method($($arg)?),
            Self::CopyFileErr(e) => e.$method($($arg)?),
            Self::CreateDirErr(e) => e.$method($($arg)?),
            Self::CreateSymlinkErr(e) => e.$method($($arg)?),
            Self::CreateTmpDirErr(e) => e.$method($($arg)?),
            Self::DeleteDirErr(e) => e.$method($($arg)?),
            Self::DeleteFileErr(e) => e.$method($($arg)?),
            Self::FileMetadataErr(e) => e.$method($($arg)?),
            Self::MoveFileErr(e) => e.$method($($arg)?),
            Self::MoveDirErr(e) => e.$method($($arg)?),
            Self::OpenFileErr(e) => e.$method($($arg)?),
            Self::ParseJSONErr(e) => e.$method($($arg)?),
            Self::ReadDirErr(e) => e.$method($($arg)?),
            Self::ReadFileErr(e) => e.$method($($arg)?),
            Self::UnknownCurrentDirErr(e) => e.$method($($arg)?),
            Self::UnknownHomeDirErr(e) => e.$method($($arg)?),
            Self::WriteFileErr(e) => e.$method($($arg)?),
            Self::SendActorMessageErr(e) => e.$method($($arg)?),
            Self::ReceiveActorMessageErr(e) => e.$method($($arg)?),
        }
    };
}

impl std::error::Error for FileSysErr {}

impl fmt::Display for FileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for FileSysErr {
    fn code(&self) -> Code {
        forward_error_method!(self, code)
    }

    fn http_status(&self) -> HTTPCode {
        forward_error_method!(self, http_status)
    }

    fn is_network_connection_error(&self) -> bool {
        forward_error_method!(self, is_network_connection_error)
    }

    fn params(&self) -> Option<serde_json::Value> {
        forward_error_method!(self, params)
    }
}
