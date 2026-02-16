// standard library
use std::{
    env,
    path::{Component, Path, PathBuf},
};

// internal crates
use crate::filesys::errors::{
    FileSysErr, PathDoesNotExistErr, PathExistsErr, UnknownCurrentDirErr,
};
use crate::trace;

pub trait PathExt {
    fn path(&self) -> &PathBuf;

    fn abs_path(&self) -> Result<PathBuf, FileSysErr> {
        let path = match self.path().is_absolute() {
            true => self.path().clone(),
            false => {
                let current_dir = env::current_dir().map_err(|e| {
                    FileSysErr::UnknownCurrentDirErr(UnknownCurrentDirErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                })?;
                current_dir.join(self.path())
            }
        };
        Ok(clean(path))
    }

    fn exists(&self) -> bool {
        self.path().exists()
    }

    fn assert_exists(&self) -> Result<(), FileSysErr> {
        if !self.exists() {
            return Err(FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                path: self.path().clone(),
                trace: trace!(),
            }));
        }
        Ok(())
    }

    fn assert_doesnt_exist(&self) -> Result<(), FileSysErr> {
        if self.exists() {
            return Err(FileSysErr::PathExistsErr(PathExistsErr {
                path: self.path().clone(),
                trace: trace!(),
            }));
        }
        Ok(())
    }
}

/// Source code was taken from path-clean crate
/// https://github.com/danreeves/path-clean/blob/3876d7cb5367997bcda17ce165bf69c4f434cb93/src/lib.rs#L57
///
/// The core implementation. It performs the following, lexically:
/// 1. Reduce multiple slashes to a single slash.
/// 2. Eliminate `.` path name elements (the current directory).
/// 3. Eliminate `..` path name elements (the parent directory) and the non-`.` non-`..`, element that precedes them.
/// 4. Eliminate `..` elements that begin a rooted path, that is, replace `/..` by `/` at the beginning of a path.
/// 5. Leave intact `..` elements that begin a non-rooted path.
///
/// If the result of this process is an empty string, return the string `"."`, representing the current directory.
pub fn clean<P>(path: P) -> PathBuf
where
    P: AsRef<Path>,
{
    let mut abs_path = Vec::new();

    for comp in path.as_ref().components() {
        match comp {
            Component::CurDir => (),
            Component::ParentDir => match abs_path.last() {
                Some(Component::RootDir) => (),
                Some(Component::Normal(_)) => {
                    abs_path.pop();
                }
                None
                | Some(Component::CurDir)
                | Some(Component::ParentDir)
                | Some(Component::Prefix(_)) => abs_path.push(comp),
            },
            comp => abs_path.push(comp),
        }
    }

    if !abs_path.is_empty() {
        abs_path.iter().collect()
    } else {
        PathBuf::from(".")
    }
}
