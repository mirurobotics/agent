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
pub fn run_as(name: &str) -> Result<(), PrivilegeErr> {
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
    // internal crates
    use super::*;

    #[test]
    fn lookup_user_returns_root_for_root() {
        // Exercises the production path against the host passwd database;
        // root is guaranteed present on every Linux system.
        let user = lookup_user("root").expect("root should always be present");
        assert_eq!(user.uid.as_raw(), 0);
        assert_eq!(user.gid.as_raw(), 0);
        assert_eq!(user.name, "root");
    }
}
