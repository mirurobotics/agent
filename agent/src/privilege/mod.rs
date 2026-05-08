// standard crates
use std::ffi::CString;

// internal crates
pub mod errors;
pub use self::errors::PrivilegeErr;
use crate::trace;

// external crates
use nix::errno::Errno;
use nix::unistd::{getegid, geteuid, initgroups, setgid, setuid};

pub type User = nix::unistd::User;

/// If running as root, drop to the user named `name`. If already running as
/// that user, no-op. Otherwise, return `WrongUser`.
///
/// `setuid(2)`/`setgid(2)` only mutate process credentials — they don't touch the
/// environ block — so env vars set before this call remain readable after the drop.
pub fn run_as(name: &str) -> Result<(), PrivilegeErr> {
    let euid = geteuid().as_raw();
    if !is_root_user(euid) {
        verify_effective_user(name)
    } else {
        let target = lookup_user(name)?;
        drop_to(&target)
    }
}

pub fn is_root_user(uid: u32) -> bool {
    uid == 0
}

pub fn lookup_user(name: &str) -> Result<User, PrivilegeErr> {
    let not_found = || PrivilegeErr::UserNotFound {
        name: name.to_string(),
        trace: trace!(),
    };

    match User::from_name(name) {
        Ok(Some(u)) => Ok(u),
        // Some libc implementations report missing entries via ENOENT/ESRCH rather than
        // `Ok(None)`.
        Ok(None) | Err(Errno::ENOENT | Errno::ESRCH) => Err(not_found()),
        Err(e) => Err(PrivilegeErr::Syscall {
            call: "getpwnam_r",
            errno: e as i32,
            trace: trace!(),
        }),
    }
}

fn verify_effective_user(name: &str) -> Result<(), PrivilegeErr> {
    let matches_effective_user = match lookup_user(name) {
        Ok(user) => user.uid == geteuid(),
        Err(PrivilegeErr::UserNotFound { .. }) => false,
        Err(e) => return Err(e),
    };
    if !matches_effective_user {
        return Err(PrivilegeErr::WrongUser {
            expected: name.to_string(),
            actual_uid: geteuid().as_raw(),
            argv0: std::env::args().next().unwrap_or_else(|| "miru-agent".into()),
            trace: trace!(),
        })
    } 
    Ok(())
}

fn drop_to(target: &User) -> Result<(), PrivilegeErr> {
    let c_name =
        CString::new(target.name.as_str()).expect("passwd entry name cannot contain NUL bytes");

    let syscall = |call: &'static str, e: Errno| PrivilegeErr::Syscall {
        call,
        errno: e as i32,
        trace: trace!(),
    };

    // Drop order matters: supplementary groups and gid first (still root, can
    // read /etc/group), then uid (irreversible after).
    initgroups(&c_name, target.gid).map_err(|e| syscall("initgroups", e))?;
    setgid(target.gid).map_err(|e| syscall("setgid", e))?;
    setuid(target.uid).map_err(|e| syscall("setuid", e))?;

    // setuid succeeding while euid stays 0 would be a kernel bug, but the
    // failure mode (still root) is catastrophic enough to be worth two
    // verifying syscalls.
    if geteuid() != target.uid || getegid() != target.gid {
        return Err(PrivilegeErr::PostDropMismatch {
            expected_uid: target.uid,
            expected_gid: target.gid,
            actual_uid: geteuid(),
            actual_gid: getegid(),
            trace: trace!(),
        });
    }

    Ok(())
}

