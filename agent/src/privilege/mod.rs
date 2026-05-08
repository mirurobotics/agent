// standard crates
use std::ffi::CString;

pub mod errors;
pub(crate) mod system;
pub use self::errors::PrivilegeErr;
use self::system::{RealSystem, System, User, Errno};
use crate::trace;

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
    use std::collections::HashMap;
    use std::ffi::{CStr, CString};
    use std::path::PathBuf;

    // internal crates
    use super::*;

    // external crates
    use nix::unistd::{Gid, ResGid, ResUid, Uid};

    /// In-memory state machine that drives the [`System`] trait deterministically.
    ///
    /// Models `setres*` by mutating `euid` / `egid` cells. By default,
    /// `getresuid` / `getresgid` report the same value for real, effective,
    /// and saved (the post-drop steady state). The override hooks
    /// (`override_getresuid`, `override_getresgid`) and the per-call errno
    /// table (`inject_errno`) allow tests to drive the kernel-bug branch
    /// (`PostDropMismatch`) and the syscall-failure branches deterministically.
    struct FakeSystem {
        euid: RefCell<u32>,
        egid: RefCell<u32>,
        users: Vec<User>,
        argv0: String,
        errno_on: RefCell<HashMap<&'static str, Errno>>,
        getresuid_override: RefCell<Option<ResUid>>,
        getresgid_override: RefCell<Option<ResGid>>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl FakeSystem {
        fn new(euid: u32, egid: u32, users: Vec<User>) -> Self {
            Self {
                euid: RefCell::new(euid),
                egid: RefCell::new(egid),
                users,
                argv0: "miru-agent".into(),
                errno_on: RefCell::new(HashMap::new()),
                getresuid_override: RefCell::new(None),
                getresgid_override: RefCell::new(None),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn with_argv0(mut self, argv0: impl Into<String>) -> Self {
            self.argv0 = argv0.into();
            self
        }

        fn inject_errno(&self, call: &'static str, e: Errno) {
            self.errno_on.borrow_mut().insert(call, e);
        }

        fn override_getresuid(&self, real: u32, effective: u32, saved: u32) {
            *self.getresuid_override.borrow_mut() = Some(ResUid {
                real: Uid::from_raw(real),
                effective: Uid::from_raw(effective),
                saved: Uid::from_raw(saved),
            });
        }

        fn override_getresgid(&self, real: u32, effective: u32, saved: u32) {
            *self.getresgid_override.borrow_mut() = Some(ResGid {
                real: Gid::from_raw(real),
                effective: Gid::from_raw(effective),
                saved: Gid::from_raw(saved),
            });
        }

        fn recorded_calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }

    impl System for FakeSystem {
        fn geteuid(&self) -> Uid {
            self.calls.borrow_mut().push("geteuid");
            Uid::from_raw(*self.euid.borrow())
        }

        fn getegid(&self) -> Gid {
            self.calls.borrow_mut().push("getegid");
            Gid::from_raw(*self.egid.borrow())
        }

        fn getresuid(&self) -> Result<ResUid, Errno> {
            self.calls.borrow_mut().push("getresuid");
            if let Some(&e) = self.errno_on.borrow().get("getresuid") {
                return Err(e);
            }
            if let Some(r) = *self.getresuid_override.borrow() {
                return Ok(r);
            }
            let u = Uid::from_raw(*self.euid.borrow());
            Ok(ResUid {
                real: u,
                effective: u,
                saved: u,
            })
        }

        fn getresgid(&self) -> Result<ResGid, Errno> {
            self.calls.borrow_mut().push("getresgid");
            if let Some(&e) = self.errno_on.borrow().get("getresgid") {
                return Err(e);
            }
            if let Some(r) = *self.getresgid_override.borrow() {
                return Ok(r);
            }
            let g = Gid::from_raw(*self.egid.borrow());
            Ok(ResGid {
                real: g,
                effective: g,
                saved: g,
            })
        }

        fn setresuid(&self, _real: Uid, eff: Uid, _saved: Uid) -> Result<(), Errno> {
            self.calls.borrow_mut().push("setresuid");
            if let Some(&e) = self.errno_on.borrow().get("setresuid") {
                return Err(e);
            }
            *self.euid.borrow_mut() = eff.as_raw();
            Ok(())
        }

        fn setresgid(&self, _real: Gid, eff: Gid, _saved: Gid) -> Result<(), Errno> {
            self.calls.borrow_mut().push("setresgid");
            if let Some(&e) = self.errno_on.borrow().get("setresgid") {
                return Err(e);
            }
            *self.egid.borrow_mut() = eff.as_raw();
            Ok(())
        }

        fn initgroups(&self, _user: &CStr, _primary_gid: Gid) -> Result<(), Errno> {
            self.calls.borrow_mut().push("initgroups");
            if let Some(&e) = self.errno_on.borrow().get("initgroups") {
                return Err(e);
            }
            Ok(())
        }

        fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno> {
            self.calls.borrow_mut().push("lookup_user");
            if let Some(&e) = self.errno_on.borrow().get("lookup_user") {
                return Err(e);
            }
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

    // ===== is_root_user =====

    #[test]
    fn is_root_user_truth_table() {
        let cases = [(0u32, true), (1, false), (1000, false), (u32::MAX, false)];
        for (uid, expected) in cases {
            assert_eq!(
                is_root_user(uid),
                expected,
                "is_root_user({uid}) should be {expected}",
            );
        }
    }

    // ===== lookup_user =====

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
    fn lookup_user_maps_enoent_to_user_not_found() {
        // Some libc implementations report missing entries via Err(ENOENT)
        // rather than Ok(None); both must funnel into UserNotFound.
        let fake = FakeSystem::new(1000, 1000, vec![fixture_user("miru", 999, 999)]);
        fake.inject_errno("lookup_user", Errno::ENOENT);
        let err = lookup_user(&fake, "miru").expect_err("ENOENT must map to UserNotFound");
        match err {
            PrivilegeErr::UserNotFound { name, .. } => assert_eq!(name, "miru"),
            other => panic!("expected UserNotFound, got {other:?}"),
        }
    }

    #[test]
    fn lookup_user_maps_esrch_to_user_not_found() {
        let fake = FakeSystem::new(1000, 1000, vec![fixture_user("miru", 999, 999)]);
        fake.inject_errno("lookup_user", Errno::ESRCH);
        let err = lookup_user(&fake, "miru").expect_err("ESRCH must map to UserNotFound");
        match err {
            PrivilegeErr::UserNotFound { name, .. } => assert_eq!(name, "miru"),
            other => panic!("expected UserNotFound, got {other:?}"),
        }
    }

    #[test]
    fn lookup_user_maps_other_errno_to_syscall() {
        let fake = FakeSystem::new(1000, 1000, vec![fixture_user("miru", 999, 999)]);
        fake.inject_errno("lookup_user", Errno::EIO);
        let err = lookup_user(&fake, "miru").expect_err("non-NotFound errno must map to Syscall");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "getpwnam_r");
                assert_eq!(errno, Errno::EIO as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
    }

    // ===== drop_to (reached via run_as_with with euid=0) =====

    #[test]
    fn drop_to_invokes_syscalls_in_correct_order() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        run_as_with(&fake, "miru").expect("clean drop");
        let drop_calls: Vec<&'static str> = fake
            .recorded_calls()
            .into_iter()
            .filter(|c| {
                matches!(
                    *c,
                    "initgroups" | "setresgid" | "setresuid" | "getresuid" | "getresgid",
                )
            })
            .collect();
        assert_eq!(
            drop_calls,
            vec![
                "initgroups",
                "setresgid",
                "setresuid",
                "getresuid",
                "getresgid",
            ],
            "drop must run groups → gid → uid → verify",
        );
    }

    #[test]
    fn drop_to_short_circuits_on_initgroups_failure() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        fake.inject_errno("initgroups", Errno::EPERM);
        let err = run_as_with(&fake, "miru").expect_err("initgroups failure must propagate");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "initgroups");
                assert_eq!(errno, Errno::EPERM as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
        let calls = fake.recorded_calls();
        assert!(
            !calls.contains(&"setresgid"),
            "setresgid must not run after initgroups failure",
        );
        assert!(
            !calls.contains(&"setresuid"),
            "setresuid must not run after initgroups failure",
        );
        assert_eq!(*fake.euid.borrow(), 0, "uid must remain root");
        assert_eq!(*fake.egid.borrow(), 0, "gid must remain root");
    }

    #[test]
    fn drop_to_short_circuits_on_setresgid_failure() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        fake.inject_errno("setresgid", Errno::EPERM);
        let err = run_as_with(&fake, "miru").expect_err("setresgid failure must propagate");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "setresgid");
                assert_eq!(errno, Errno::EPERM as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
        assert!(
            !fake.recorded_calls().contains(&"setresuid"),
            "setresuid must not run after setresgid failure — uid drop is irreversible",
        );
        assert_eq!(*fake.euid.borrow(), 0, "uid must remain root");
    }

    #[test]
    fn drop_to_propagates_setresuid_errno() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        fake.inject_errno("setresuid", Errno::EPERM);
        let err = run_as_with(&fake, "miru").expect_err("setresuid failure must propagate");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "setresuid");
                assert_eq!(errno, Errno::EPERM as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
        // Partial-drop reality: setresgid succeeded (gid mutated) but uid stays
        // root. The error surfaces; the caller is expected to abort the process.
        assert_eq!(*fake.egid.borrow(), 5678);
        assert_eq!(*fake.euid.borrow(), 0);
    }

    #[test]
    fn drop_to_propagates_getresuid_errno() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        fake.inject_errno("getresuid", Errno::EIO);
        let err = run_as_with(&fake, "miru").expect_err("getresuid failure must propagate");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "getresuid");
                assert_eq!(errno, Errno::EIO as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
    }

    #[test]
    fn drop_to_propagates_getresgid_errno() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        fake.inject_errno("getresgid", Errno::EIO);
        let err = run_as_with(&fake, "miru").expect_err("getresgid failure must propagate");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "getresgid");
                assert_eq!(errno, Errno::EIO as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
    }

    #[test]
    fn drop_to_returns_post_drop_mismatch_with_all_fields_populated() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        // Sentinel values distinct from each other and from target.uid/target.gid
        // so a field swap or off-by-one in the error construction is detectable.
        fake.override_getresuid(100, 101, 102);
        fake.override_getresgid(200, 201, 202);
        let err = run_as_with(&fake, "miru").expect_err("post-drop mismatch must propagate");
        match err {
            PrivilegeErr::PostDropMismatch {
                expected_uid,
                expected_gid,
                actual_ruid,
                actual_euid,
                actual_suid,
                actual_rgid,
                actual_egid,
                actual_sgid,
                ..
            } => {
                assert_eq!(expected_uid, 1234);
                assert_eq!(expected_gid, 5678);
                assert_eq!(actual_ruid, 100);
                assert_eq!(actual_euid, 101);
                assert_eq!(actual_suid, 102);
                assert_eq!(actual_rgid, 200);
                assert_eq!(actual_egid, 201);
                assert_eq!(actual_sgid, 202);
            }
            other => panic!("expected PostDropMismatch, got {other:?}"),
        }
    }

    // ===== run_as_with =====

    #[test]
    fn run_as_with_drops_to_target_when_root() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 1234)]);
        run_as_with(&fake, "miru").expect("drop succeeds");
        assert_eq!(*fake.euid.borrow(), 1234);
        assert_eq!(*fake.egid.borrow(), 1234);
    }

    #[test]
    fn run_as_with_no_op_when_already_target_user() {
        let fake = FakeSystem::new(1234, 5678, vec![fixture_user("miru", 1234, 5678)]);
        run_as_with(&fake, "miru").expect("already-target case must be a no-op");
        assert_eq!(*fake.euid.borrow(), 1234);
        assert_eq!(*fake.egid.borrow(), 5678);
        let calls = fake.recorded_calls();
        assert!(!calls.contains(&"initgroups"));
        assert!(!calls.contains(&"setresgid"));
        assert!(!calls.contains(&"setresuid"));
    }

    #[test]
    fn run_as_with_returns_wrong_user_when_uid_mismatches() {
        let fake = FakeSystem::new(2000, 5678, vec![fixture_user("miru", 1234, 5678)])
            .with_argv0("/usr/sbin/test-agent");
        let err = run_as_with(&fake, "miru").expect_err("uid mismatch must yield WrongUser");
        match err {
            PrivilegeErr::WrongUser {
                expected,
                actual_uid,
                actual_gid,
                expected_uid,
                expected_gid,
                argv0,
                ..
            } => {
                assert_eq!(expected, "miru");
                assert_eq!(actual_uid, 2000);
                assert_eq!(actual_gid, 5678);
                assert_eq!(expected_uid, 1234);
                assert_eq!(expected_gid, 5678);
                // argv0 must come from the System trait, not std::env::args().
                assert_eq!(argv0, "/usr/sbin/test-agent");
            }
            other => panic!("expected WrongUser, got {other:?}"),
        }
    }

    #[test]
    fn run_as_with_returns_wrong_user_when_gid_mismatches() {
        let fake = FakeSystem::new(1234, 9999, vec![fixture_user("miru", 1234, 5678)]);
        let err = run_as_with(&fake, "miru").expect_err("gid mismatch must yield WrongUser");
        match err {
            PrivilegeErr::WrongUser {
                expected,
                actual_uid,
                actual_gid,
                expected_uid,
                expected_gid,
                ..
            } => {
                assert_eq!(expected, "miru");
                assert_eq!(actual_uid, 1234);
                assert_eq!(actual_gid, 9999);
                assert_eq!(expected_uid, 1234);
                assert_eq!(expected_gid, 5678);
            }
            other => panic!("expected WrongUser, got {other:?}"),
        }
    }

    #[test]
    fn run_as_with_returns_user_not_found_when_root_and_user_missing() {
        let fake = FakeSystem::new(0, 0, Vec::new());
        let err = run_as_with(&fake, "miru").expect_err("missing target must yield UserNotFound");
        match err {
            PrivilegeErr::UserNotFound { name, .. } => assert_eq!(name, "miru"),
            other => panic!("expected UserNotFound, got {other:?}"),
        }
        // No drop attempted.
        let calls = fake.recorded_calls();
        assert!(!calls.contains(&"initgroups"));
        assert!(!calls.contains(&"setresgid"));
        assert!(!calls.contains(&"setresuid"));
        assert_eq!(*fake.euid.borrow(), 0);
    }

    #[test]
    fn run_as_with_returns_syscall_when_root_and_lookup_user_errors() {
        let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);
        fake.inject_errno("lookup_user", Errno::EIO);
        let err = run_as_with(&fake, "miru")
            .expect_err("lookup errno must propagate before any drop syscall");
        match err {
            PrivilegeErr::Syscall { call, errno, .. } => {
                assert_eq!(call, "getpwnam_r");
                assert_eq!(errno, Errno::EIO as i32);
            }
            other => panic!("expected Syscall, got {other:?}"),
        }
        // No drop attempted.
        let calls = fake.recorded_calls();
        assert!(!calls.contains(&"initgroups"));
        assert_eq!(*fake.euid.borrow(), 0);
    }
}
