// standard crates
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::PathBuf;

// internal crates
use crate::privilege::system::{Errno, Gid, ResGid, ResUid, System, Uid, User};

/// In-memory state machine that drives the [`System`] trait deterministically.
///
/// Models `setres*` by mutating `euid` / `egid` cells. By default,
/// `getresuid` / `getresgid` report the same value for real, effective,
/// and saved (the post-drop steady state). The override hooks
/// (`override_getresuid`, `override_getresgid`) and the per-call errno
/// table (`inject_errno`) allow tests to drive the kernel-bug branch
/// (`PostDropMismatch`) and the syscall-failure branches deterministically.
pub(super) struct FakeSystem {
    pub(super) euid: RefCell<u32>,
    pub(super) egid: RefCell<u32>,
    pub(super) users: Vec<User>,
    pub(super) argv0: String,
    pub(super) errno_on: RefCell<HashMap<&'static str, Errno>>,
    pub(super) getresuid_override: RefCell<Option<ResUid>>,
    pub(super) getresgid_override: RefCell<Option<ResGid>>,
    pub(super) supplementary_groups: RefCell<Vec<Gid>>,
    pub(super) calls: RefCell<Vec<&'static str>>,
}

impl FakeSystem {
    pub(super) fn new(euid: u32, egid: u32, users: Vec<User>) -> Self {
        Self {
            euid: RefCell::new(euid),
            egid: RefCell::new(egid),
            users,
            argv0: "miru-agent".into(),
            errno_on: RefCell::new(HashMap::new()),
            getresuid_override: RefCell::new(None),
            getresgid_override: RefCell::new(None),
            supplementary_groups: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn with_argv0(mut self, argv0: impl Into<String>) -> Self {
        self.argv0 = argv0.into();
        self
    }

    pub(super) fn inject_errno(&self, call: &'static str, e: Errno) {
        self.errno_on.borrow_mut().insert(call, e);
    }

    pub(super) fn override_getresuid(&self, real: u32, effective: u32, saved: u32) {
        *self.getresuid_override.borrow_mut() = Some(ResUid {
            real: Uid::from_raw(real),
            effective: Uid::from_raw(effective),
            saved: Uid::from_raw(saved),
        });
    }

    pub(super) fn override_getresgid(&self, real: u32, effective: u32, saved: u32) {
        *self.getresgid_override.borrow_mut() = Some(ResGid {
            real: Gid::from_raw(real),
            effective: Gid::from_raw(effective),
            saved: Gid::from_raw(saved),
        });
    }

    pub(super) fn recorded_calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }

    pub(super) fn set_supplementary_groups(&self, groups: Vec<Gid>) {
        *self.supplementary_groups.borrow_mut() = groups;
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

    fn getgroups(&self) -> Result<Vec<Gid>, Errno> {
        self.calls.borrow_mut().push("getgroups");
        if let Some(&e) = self.errno_on.borrow().get("getgroups") {
            return Err(e);
        }
        Ok(self.supplementary_groups.borrow().clone())
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

pub(super) fn fixture_user(name: &str, uid: u32, gid: u32) -> User {
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
