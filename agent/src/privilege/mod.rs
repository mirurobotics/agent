// standard crates
use std::ffi::CString;

pub mod errors;
pub(crate) mod system;
pub use self::errors::PrivilegeErr;
use self::system::{RealSystem, System};
use crate::trace;

// external crates
use nix::errno::Errno;

pub(crate) type User = nix::unistd::User;

/// If running as root, drop to the user named `name`. If already running as
/// that user, no-op. Otherwise, return `WrongUser`.
pub fn run_as(name: &str) -> Result<(), PrivilegeErr> {
    run_as_with(&RealSystem, name)
}

pub(crate) fn run_as_with<S: System>(sys: &S, name: &str) -> Result<(), PrivilegeErr> {
    let euid = sys.geteuid().as_raw();
    if !is_root_user(euid) {
        verify_effective_user(sys, name)
    } else {
        let target = lookup_user(sys, name)?;
        drop_to(sys, &target)
    }
}

pub(crate) fn is_root_user(uid: u32) -> bool {
    uid == 0
}

pub(crate) fn lookup_user<S: System>(sys: &S, name: &str) -> Result<User, PrivilegeErr> {
    let not_found = || PrivilegeErr::UserNotFound {
        name: name.to_string(),
        trace: trace!(),
    };

    match sys.lookup_user(name) {
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

fn verify_effective_user<S: System>(sys: &S, name: &str) -> Result<(), PrivilegeErr> {
    let user = lookup_user(sys, name)?;
    if user.uid != sys.geteuid() || user.gid != sys.getegid() {
        return Err(PrivilegeErr::WrongUser {
            expected: name.to_string(),
            actual_uid: sys.geteuid().as_raw(),
            actual_gid: sys.getegid().as_raw(),
            expected_uid: user.uid.as_raw(),
            expected_gid: user.gid.as_raw(),
            argv0: sys.argv0(),
            trace: trace!(),
        });
    }
    Ok(())
}

/// Drop the current process from root to `target`.
///
/// Caller invariant: `geteuid() == 0`. Enforced by `debug_assert!` in debug
/// builds.
fn drop_to<S: System>(sys: &S, target: &User) -> Result<(), PrivilegeErr> {
    debug_assert!(
        sys.geteuid() == nix::unistd::Uid::from_raw(0),
        "drop_to requires euid=0",
    );

    let c_name =
        CString::new(target.name.as_str()).expect("passwd entry name cannot contain NUL bytes");

    let syscall = |call: &'static str, e: Errno| PrivilegeErr::Syscall {
        call,
        errno: e as i32,
        trace: trace!(),
    };

    // Drop order matters: supplementary groups and gid first (still root, can
    // read /etc/group), then uid (irreversible after). `setresgid`/`setresuid`
    // set real, effective, and saved ids in one syscall — using `setgid`/
    // `setuid` would leave the saved uid as 0, allowing a later `setuid(0)` to
    // re-acquire root.
    sys.initgroups(&c_name, target.gid)
        .map_err(|e| syscall("initgroups", e))?;
    sys.setresgid(target.gid, target.gid, target.gid)
        .map_err(|e| syscall("setresgid", e))?;
    sys.setresuid(target.uid, target.uid, target.uid)
        .map_err(|e| syscall("setresuid", e))?;

    // The drop syscalls succeeding without all three uids/gids matching would
    // be a kernel bug, but the failure mode (still partially root) is
    // catastrophic enough to be worth two verifying syscalls.
    let ruid = sys.getresuid().map_err(|e| syscall("getresuid", e))?;
    let rgid = sys.getresgid().map_err(|e| syscall("getresgid", e))?;
    if ruid.real != target.uid
        || ruid.effective != target.uid
        || ruid.saved != target.uid
        || rgid.real != target.gid
        || rgid.effective != target.gid
        || rgid.saved != target.gid
    {
        return Err(PrivilegeErr::PostDropMismatch {
            expected_uid: target.uid.as_raw(),
            expected_gid: target.gid.as_raw(),
            actual_ruid: ruid.real.as_raw(),
            actual_euid: ruid.effective.as_raw(),
            actual_suid: ruid.saved.as_raw(),
            actual_rgid: rgid.real.as_raw(),
            actual_egid: rgid.effective.as_raw(),
            actual_sgid: rgid.saved.as_raw(),
            trace: trace!(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::cell::RefCell;
    use std::ffi::{CStr, CString};
    use std::path::PathBuf;

    // internal crates
    use super::*;

    // external crates
    use nix::unistd::{Gid, ResGid, ResUid, Uid};

    /// In-memory state machine that drives the [`System`] trait deterministically.
    ///
    /// Models `setres*` by mutating `euid` / `egid` cells; `getresuid` /
    /// `getresgid` report the same value for real, effective, and saved (the
    /// post-drop state, which is the only state we model).
    struct FakeSystem {
        euid: RefCell<u32>,
        egid: RefCell<u32>,
        users: Vec<User>,
        argv0: String,
    }

    impl FakeSystem {
        fn new(euid: u32, egid: u32, users: Vec<User>) -> Self {
            Self {
                euid: RefCell::new(euid),
                egid: RefCell::new(egid),
                users,
                argv0: "miru-agent".into(),
            }
        }
    }

    impl System for FakeSystem {
        fn geteuid(&self) -> Uid {
            Uid::from_raw(*self.euid.borrow())
        }

        fn getegid(&self) -> Gid {
            Gid::from_raw(*self.egid.borrow())
        }

        fn getresuid(&self) -> Result<ResUid, Errno> {
            let u = Uid::from_raw(*self.euid.borrow());
            Ok(ResUid {
                real: u,
                effective: u,
                saved: u,
            })
        }

        fn getresgid(&self) -> Result<ResGid, Errno> {
            let g = Gid::from_raw(*self.egid.borrow());
            Ok(ResGid {
                real: g,
                effective: g,
                saved: g,
            })
        }

        fn setresuid(&self, _real: Uid, eff: Uid, _saved: Uid) -> Result<(), Errno> {
            *self.euid.borrow_mut() = eff.as_raw();
            Ok(())
        }

        fn setresgid(&self, _real: Gid, eff: Gid, _saved: Gid) -> Result<(), Errno> {
            *self.egid.borrow_mut() = eff.as_raw();
            Ok(())
        }

        fn initgroups(&self, _user: &CStr, _primary_gid: Gid) -> Result<(), Errno> {
            Ok(())
        }

        fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno> {
            Ok(self.users.iter().find(|u| u.name == name).cloned())
        }

        fn argv0(&self) -> String {
            self.argv0.clone()
        }
    }

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
        // Exercises the production `RealSystem` path against the host passwd
        // database; root is guaranteed present on every Linux system.
        let user = lookup_user(&RealSystem, "root").expect("root should always be present");
        assert_eq!(user.uid.as_raw(), 0);
        assert_eq!(user.gid.as_raw(), 0);
        assert_eq!(user.name, "root");
    }

    #[test]
    fn lookup_user_returns_user_not_found_for_nonexistent() {
        let fake = FakeSystem::new(1000, 1000, Vec::new());
        let err = lookup_user(&fake, "nonexistent_user_xyz_123_miru_test")
            .expect_err("an empty fake passwd table must reject any name");
        match err {
            PrivilegeErr::UserNotFound { name, .. } => {
                assert_eq!(name, "nonexistent_user_xyz_123_miru_test");
            }
            other => panic!("expected UserNotFound, got {other:?}"),
        }
    }

    #[test]
    fn run_as_with_drops_to_target_when_root() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 1234)]);
        run_as_with(&fake, "miru").expect("drop succeeds");
        assert_eq!(*fake.euid.borrow(), 1234);
        assert_eq!(*fake.egid.borrow(), 1234);
    }
}
