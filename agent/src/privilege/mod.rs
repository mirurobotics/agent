// internal crates
pub mod errors;
pub use self::errors::PrivilegeErr;
use crate::trace;

// external crates
use nix::errno::Errno;
use nix::unistd::{Gid, Uid, User};

/// Verify that the current effective user matches `name`. Returns
/// `Ok(())` on match. Returns `WrongUser` if euid or egid does not match,
/// `UserNotFound` if `name` has no passwd entry, or `Syscall` if the
/// passwd lookup itself fails.
pub fn verify_effective_user(name: &str) -> Result<(), PrivilegeErr> {
    let user = lookup_user(name)?;
    let euid = nix::unistd::geteuid();
    let egid = nix::unistd::getegid();
    let argv0 = std::env::args()
        .next()
        .unwrap_or_else(|| "miru-agent".into());
    verify(euid, egid, &user, name, argv0)
}

fn lookup_user(name: &str) -> Result<User, PrivilegeErr> {
    let not_found = || PrivilegeErr::UserNotFound {
        name: name.to_string(),
        trace: trace!(),
    };
    match User::from_name(name) {
        Ok(Some(u)) => Ok(u),
        // Some libc implementations report missing entries via ENOENT/ESRCH
        // rather than `Ok(None)`.
        Ok(None) => Err(not_found()),
        Err(Errno::ENOENT | Errno::ESRCH) => Err(not_found()),
        Err(e) => Err(PrivilegeErr::Syscall {
            call: "getpwnam_r",
            errno: e,
            trace: trace!(),
        }),
    }
}

fn verify(
    euid: Uid,
    egid: Gid,
    user: &User,
    name: &str,
    argv0: String,
) -> Result<(), PrivilegeErr> {
    if user.uid != euid || user.gid != egid {
        return Err(PrivilegeErr::WrongUser {
            expected: name.to_string(),
            actual_uid: euid.as_raw(),
            actual_gid: egid.as_raw(),
            expected_uid: user.uid.as_raw(),
            expected_gid: user.gid.as_raw(),
            argv0,
            trace: trace!(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::ffi::CString;
    use std::path::PathBuf;

    // internal crates
    use super::*;

    fn fixture_user(name: &str, uid: u32, gid: u32) -> User {
        User {
            name: name.to_string(),
            passwd: CString::new("x").unwrap(),
            uid: Uid::from_raw(uid),
            gid: Gid::from_raw(gid),
            gecos: CString::new("").unwrap(),
            dir: PathBuf::from("/nonexistent"),
            shell: PathBuf::from("/bin/false"),
        }
    }

    #[test]
    fn lookup_user_returns_root_for_root() {
        // Exercises the production path against the host passwd database;
        // root is guaranteed present on every Linux system.
        let user = lookup_user("root").expect("root should always be present");
        assert_eq!(user.uid.as_raw(), 0);
        assert_eq!(user.gid.as_raw(), 0);
        assert_eq!(user.name, "root");
    }

    #[test]
    fn lookup_user_returns_user_not_found_for_missing_name() {
        // Exercises the `Ok(None)` / ENOENT branch against the host passwd
        // database. The name is constructed so that no real account collides
        // (NUL bytes in passwd entries are forbidden, so a deliberately long
        // unlikely name is used).
        let err = lookup_user("__miru_nonexistent_user_for_tests__")
            .expect_err("synthetic name must not resolve");
        let PrivilegeErr::UserNotFound { name, .. } = err else {
            panic!("expected UserNotFound, got {err:?}");
        };
        assert_eq!(name, "__miru_nonexistent_user_for_tests__");
    }

    #[test]
    fn verify_effective_user_returns_user_not_found_when_name_is_missing() {
        // Drives the `pub fn verify_effective_user` entry point through the
        // lookup-failure path so the early-return `?` branch is exercised.
        let err = verify_effective_user("__miru_nonexistent_user_for_tests__")
            .expect_err("synthetic name must not resolve");
        let PrivilegeErr::UserNotFound { name, .. } = err else {
            panic!("expected UserNotFound, got {err:?}");
        };
        assert_eq!(name, "__miru_nonexistent_user_for_tests__");
    }

    #[test]
    fn verify_returns_ok_when_euid_and_egid_match() {
        let user = fixture_user("miru", 1234, 5678);
        let result = verify(
            Uid::from_raw(1234),
            Gid::from_raw(5678),
            &user,
            "miru",
            "miru-agent".to_string(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn verify_returns_wrong_user_when_uid_mismatches() {
        let user = fixture_user("miru", 1234, 5678);
        let err = verify(
            Uid::from_raw(2000),
            Gid::from_raw(5678),
            &user,
            "miru",
            "/usr/sbin/test-agent".to_string(),
        )
        .expect_err("uid mismatch must produce WrongUser");
        let PrivilegeErr::WrongUser {
            expected,
            actual_uid,
            actual_gid,
            expected_uid,
            expected_gid,
            argv0,
            ..
        } = err
        else {
            panic!("expected WrongUser, got {err:?}");
        };
        // lint:allow(field-by-field-assert)
        assert_eq!(expected, "miru");
        assert_eq!(actual_uid, 2000);
        assert_eq!(actual_gid, 5678);
        assert_eq!(expected_uid, 1234);
        assert_eq!(expected_gid, 5678);
        assert_eq!(argv0, "/usr/sbin/test-agent");
    }

    #[test]
    fn verify_returns_wrong_user_when_gid_mismatches() {
        let user = fixture_user("miru", 1234, 5678);
        let err = verify(
            Uid::from_raw(1234),
            Gid::from_raw(9999),
            &user,
            "miru",
            "miru-agent".to_string(),
        )
        .expect_err("gid mismatch must produce WrongUser");
        let PrivilegeErr::WrongUser {
            actual_uid,
            actual_gid,
            ..
        } = err
        else {
            panic!("expected WrongUser, got {err:?}");
        };
        assert_eq!(actual_uid, 1234);
        assert_eq!(actual_gid, 9999);
    }
}
