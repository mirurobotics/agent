// internal crates
pub use self::os::Unreadable;
use miru_agent::filesys::{File, PathExt};

/// Denies this process read access to `file` until the returned guard drops,
/// on every OS. Only reading is guaranteed to fail; the file can still be
/// inspected. Bind the guard to a named variable, since `let _ =` drops it at
/// once.
pub fn make_unreadable(file: &File) -> Unreadable {
    Unreadable::new(file.path())
}

#[cfg(unix)]
mod os {
    // standard crates
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    pub struct Unreadable {
        path: PathBuf,
        prev: Permissions,
    }

    impl Unreadable {
        pub(super) fn new(path: &Path) -> Self {
            let prev = std::fs::metadata(path).unwrap().permissions();
            std::fs::set_permissions(path, Permissions::from_mode(0o000)).unwrap();
            Self {
                path: path.to_path_buf(),
                prev,
            }
        }
    }

    impl Drop for Unreadable {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, self.prev.clone());
        }
    }
}

// Windows ignores a mode's read bits, so read access is denied with an ACL
// entry instead.
#[cfg(windows)]
mod os {
    // standard crates
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    // Everyone, which includes this process's user.
    const EVERYONE: &str = "*S-1-1-0";

    pub struct Unreadable {
        path: PathBuf,
    }

    impl Unreadable {
        pub(super) fn new(path: &Path) -> Self {
            let out = icacls(path, &["/deny", &format!("{EVERYONE}:(RD)")]).unwrap();
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

    impl Drop for Unreadable {
        fn drop(&mut self) {
            let _ = icacls(&self.path, &["/remove:d", EVERYONE]);
        }
    }

    fn icacls(path: &Path, args: &[&str]) -> io::Result<Output> {
        Command::new("icacls").arg(path).args(args).output()
    }
}
