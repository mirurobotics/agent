// internal crates
pub use self::os::Denied;
use miru_daemon::filesys::{Dir, File, PathExt};

/// Denies this process read access to `file` until the returned guard drops,
/// on every OS. Only reading is guaranteed to fail; the file can still be
/// inspected. Bind the guard to a named variable, since `let _ =` drops it at
/// once.
pub fn make_unreadable(file: &File) -> Denied {
    Denied::new(file.path(), Access::Read)
}

/// Denies this process creating, renaming or deleting entries in `dir` until
/// the returned guard drops, on every OS. Existing entries can still be read.
/// Bind the guard to a named variable, since `let _ =` drops it at once.
pub fn make_read_only(dir: &Dir) -> Denied {
    Denied::new(dir.path(), Access::Write)
}

/// The access a [`Denied`] guard takes away.
enum Access {
    Read,
    Write,
}

#[cfg(unix)]
mod os {
    // standard crates
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    // internal crates
    use super::Access;

    pub struct Denied {
        path: PathBuf,
        prev: Permissions,
    }

    impl Denied {
        pub(super) fn new(path: &Path, access: Access) -> Self {
            let mode = match access {
                Access::Read => 0o000,
                Access::Write => 0o555,
            };
            let prev = std::fs::metadata(path).unwrap().permissions();
            std::fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
            Self {
                path: path.to_path_buf(),
                prev,
            }
        }
    }

    impl Drop for Denied {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, self.prev.clone());
        }
    }
}

// Windows ignores a mode's read and write bits, so access is denied with ACL
// entries instead.
#[cfg(windows)]
mod os {
    // standard crates
    use std::ffi::OsStr;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    // internal crates
    use super::Access;

    // Everyone, which includes this process's user.
    const EVERYONE: &str = "*S-1-1-0";

    pub struct Denied {
        path: PathBuf,
    }

    impl Denied {
        pub(super) fn new(path: &Path, access: Access) -> Self {
            let rights: &[&str] = match access {
                Access::Read => &["(RD)"],
                // Direct entries also inherit a deny on delete, since an entry's
                // own delete right lets it be deleted despite the directory's deny.
                Access::Write => &["(WD,AD,DC)", "(OI)(CI)(NP)(IO)(DE)"],
            };
            let mut args = vec!["/deny".to_string()];
            args.extend(rights.iter().map(|r| format!("{EVERYONE}:{r}")));
            let out = icacls(path, &args).unwrap();
            assert!(
                out.status.success(),
                "icacls /deny on {}: {}\n{}{}",
                path.display(),
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
            Self {
                path: path.to_path_buf(),
            }
        }
    }

    impl Drop for Denied {
        fn drop(&mut self) {
            let _ = icacls(&self.path, &["/remove:d", EVERYONE]);
        }
    }

    fn icacls<S: AsRef<OsStr>>(path: &Path, args: &[S]) -> io::Result<Output> {
        Command::new("icacls").arg(path).args(args).output()
    }
}
