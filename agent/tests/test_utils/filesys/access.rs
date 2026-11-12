// internal crates
pub use self::os::Denied;
use miru_agent::filesys::{Dir, File, PathExt};

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

/// Replaces `dir`'s ACL with a protected one like the installer's private
/// folders: full control for SYSTEM, Administrators and this process's user,
/// passed on to new entries, with nothing inherited. Use it on a temp dir,
/// since nothing restores the old ACL.
#[cfg(windows)]
pub fn make_private(dir: &Dir) {
    os::make_private(dir.path())
}

/// Asserts `file` has exactly the ACL a file created directly in its folder
/// gets, and that the ACL allows only SYSTEM, Administrators and this
/// process's user, all through inherited entries. Pair it with
/// [`make_private`].
#[cfg(windows)]
pub fn assert_inherits_private_dacl(file: &File) {
    os::assert_inherits_private_dacl(file.path())
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
    use miru_agent::filesys::PathExt;

    // Everyone, which includes this process's user.
    const EVERYONE: &str = "*S-1-1-0";
    const SYSTEM: &str = "*S-1-5-18";
    const ADMINISTRATORS: &str = "*S-1-5-32-544";

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
            icacls_ok(path, &args);
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

    pub(super) fn make_private(dir: &Path) {
        let user = format!("*{}", current_user_sid());
        let mut args = vec!["/inheritance:r".to_string(), "/grant:r".to_string()];
        for sid in [SYSTEM, ADMINISTRATORS, user.as_str()] {
            args.push(format!("{sid}:(OI)(CI)F"));
        }
        icacls_ok(dir, &args);
    }

    pub(super) fn assert_inherits_private_dacl(file: &Path) {
        let control = file.with_file_name("dacl-control");
        std::fs::write(&control, b"").unwrap();
        let want = dacl(&control);
        std::fs::remove_file(&control).unwrap();

        let got = dacl(file);
        assert_eq!(
            got,
            want,
            "DACL of {} differs from a new file's",
            file.display()
        );

        let start = got.find('(').expect("DACL has no entries");
        let aces: Vec<Vec<&str>> = got[start..]
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split(")(")
            .map(|ace| ace.split(';').collect())
            .collect();
        for ace in &aces {
            assert!(
                ace[0] == "A" && ace[1].contains("ID"),
                "{} has an entry that is not an inherited allow: {got}",
                file.display()
            );
        }

        // SDDL writes well-known SIDs as abbreviations, so SYSTEM shows as SY.
        let user = current_user_sid();
        let user = if user == "S-1-5-18" { "SY" } else { user.as_str() };
        let mut want_sids = vec!["SY", "BA", user];
        want_sids.sort_unstable();
        want_sids.dedup();
        let mut sids: Vec<&str> = aces.iter().map(|ace| ace[5]).collect();
        sids.sort_unstable();
        sids.dedup();
        assert_eq!(
            sids,
            want_sids,
            "{} grants other accounts: {got}",
            file.display()
        );
    }

    fn dacl(path: &Path) -> String {
        let tmp = super::super::dirs::temp("dacl").unwrap();
        let saved = tmp.path().join("dacl.txt");
        icacls_ok(path, &[OsStr::new("/save"), saved.as_os_str()]);
        let bytes = std::fs::read(&saved).unwrap();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let text = String::from_utf16(&units).unwrap();
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        text.lines().nth(1).unwrap().trim().to_string()
    }

    fn current_user_sid() -> String {
        let out = Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "whoami /user: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let sid = stdout.trim().rsplit(',').next().unwrap();
        sid.trim_matches('"').to_string()
    }

    fn icacls_ok<S: AsRef<OsStr>>(path: &Path, args: &[S]) {
        let out = icacls(path, args).unwrap();
        let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
        assert!(
            out.status.success(),
            "icacls {args:?} on {}: {}\n{}{}",
            path.display(),
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }

    fn icacls<S: AsRef<OsStr>>(path: &Path, args: &[S]) -> io::Result<Output> {
        Command::new("icacls").arg(path).args(args).output()
    }
}
