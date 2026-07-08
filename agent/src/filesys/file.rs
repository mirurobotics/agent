// standard crates
use std::fmt::Display;
use std::path::PathBuf;

// internal crates
use crate::filesys::{
    dir::Dir,
    errors::{FileSysErr, UnknownFileNameErr, UnknownParentDirForFileErr},
    path::PathExt,
};
use crate::trace;

/// File struct for interacting with files
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct File {
    path: PathBuf,
}

pub type Metadata = std::fs::Metadata;

impl Display for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.to_str().unwrap_or_default())
    }
}

impl PathExt for File {
    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl File {
    pub fn new<T: Into<PathBuf>>(path: T) -> Self {
        let path: PathBuf = path.into().components().collect();
        File { path }
    }

    pub fn is_absolute(&self) -> bool {
        self.path().is_absolute()
    }

    pub fn name(&self) -> Result<&str, FileSysErr> {
        let file_name_os_str = match self.path.file_name() {
            Some(name) => name,
            None => {
                return Err(FileSysErr::UnknownFileNameErr(UnknownFileNameErr {
                    file: self.clone(),
                    trace: trace!(),
                }));
            }
        };
        match file_name_os_str.to_str() {
            Some(name) => Ok(name),
            None => Err(FileSysErr::UnknownFileNameErr(UnknownFileNameErr {
                file: self.clone(),
                trace: trace!(),
            })),
        }
    }

    pub fn parent(&self) -> Result<Dir, FileSysErr> {
        let parent = self
            .path
            .parent()
            .ok_or(FileSysErr::UnknownParentDirForFileErr(
                UnknownParentDirForFileErr {
                    file: self.clone(),
                    trace: trace!(),
                },
            ))?;
        Ok(Dir::new(parent))
    }
}

pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            // Allow alphanumeric and some safe characters
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            // Replace everything else with underscore
            _ => '_',
        })
        .collect()
}
