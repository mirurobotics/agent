// standard crates
use std::fmt::Display;
use std::path::{Path, PathBuf};

// internal crates
use crate::filesys::{
    errors::{FileSysErr, InvalidDirNameErr, UnknownDirNameErr, UnknownParentDirForDirErr},
    file::File,
    path::PathExt,
};
use crate::trace;

/// Dir struct for interacting with directories
#[derive(Clone, Debug)]
pub struct Dir {
    path: PathBuf,
}

impl Display for Dir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.to_str().unwrap_or_default())
    }
}

impl PathExt for Dir {
    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Dir {
    pub fn new<T: Into<PathBuf>>(path: T) -> Dir {
        Dir { path: path.into() }
    }

    pub fn name(&self) -> Result<&str, FileSysErr> {
        let file_name_os_str = match self.path.file_name() {
            Some(name) => name,
            None => {
                return Err(FileSysErr::UnknownDirNameErr(UnknownDirNameErr {
                    dir: self.clone(),
                    trace: trace!(),
                }));
            }
        };
        match file_name_os_str.to_str() {
            Some(name) => Ok(name),
            None => Err(FileSysErr::UnknownDirNameErr(UnknownDirNameErr {
                dir: self.clone(),
                trace: trace!(),
            })),
        }
    }

    pub fn parent(&self) -> Result<Dir, FileSysErr> {
        let abs_path = self.abs_path()?;
        let parent = abs_path
            .parent()
            .ok_or(FileSysErr::UnknownParentDirForDirErr(
                UnknownParentDirForDirErr {
                    dir: self.clone(),
                    trace: trace!(),
                },
            ))?;
        Ok(Dir::new(parent))
    }

    pub fn is_valid_dir_name(dir_name: &str) -> bool {
        !dir_name.is_empty()
            && !dir_name.contains('/')
            && !dir_name.contains('\0')
            && dir_name.len() <= 255
    }

    pub fn assert_valid_dir_name(dir_name: &str) -> Result<(), FileSysErr> {
        if !Dir::is_valid_dir_name(dir_name) {
            return Err(FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
                name: dir_name.to_string(),
                trace: trace!(),
            }));
        }
        Ok(())
    }

    pub fn subdir<T: Into<PathBuf>>(&self, rel_path: T) -> Dir {
        let rel_path = rel_path.into();
        let rel_path_stripped = rel_path
            .strip_prefix(std::path::MAIN_SEPARATOR.to_string())
            .unwrap_or(&rel_path);
        let mut new_dir = self.path.clone();
        new_dir = new_dir.join(rel_path_stripped);
        Dir::new(new_dir)
    }

    pub fn file(&self, file_name: &str) -> File {
        let file_name_path = Path::new(file_name);
        let file_name_stripped = file_name_path
            .strip_prefix(std::path::MAIN_SEPARATOR.to_string())
            .unwrap_or(file_name_path);
        let file_path = self.path.join(file_name_stripped);
        File::new(file_path)
    }
}
