// standard crates
use std::ffi::CStr;

// external crates
use nix::errno::Errno;
use nix::unistd::{Gid, ResGid, ResUid, Uid, User};

/// Seam for every external interaction in the privilege module.
///
/// Production code constructs [`RealSystem`] which delegates to `nix::unistd`
/// and `std::env::args()`. Tests construct a fake that drives the same trait
/// against in-memory state, allowing branches like `WrongUser`,
/// `PostDropMismatch`, and per-syscall errno propagation to be exercised
/// deterministically without root.
#[allow(dead_code)] // wired up in M2 of the System trait refactor
pub(crate) trait System {
    fn geteuid(&self) -> Uid;
    fn getegid(&self) -> Gid;
    fn getresuid(&self) -> Result<ResUid, Errno>;
    fn getresgid(&self) -> Result<ResGid, Errno>;
    fn setresuid(&self, real: Uid, eff: Uid, saved: Uid) -> Result<(), Errno>;
    fn setresgid(&self, real: Gid, eff: Gid, saved: Gid) -> Result<(), Errno>;
    fn initgroups(&self, user: &CStr, primary_gid: Gid) -> Result<(), Errno>;
    fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno>;
    fn argv0(&self) -> String;
}

/// Production implementation of [`System`] that delegates to `nix::unistd`
/// and `std::env::args()`.
#[allow(dead_code)] // wired up in M2 of the System trait refactor
pub(crate) struct RealSystem;

impl System for RealSystem {
    fn geteuid(&self) -> Uid {
        nix::unistd::geteuid()
    }

    fn getegid(&self) -> Gid {
        nix::unistd::getegid()
    }

    fn getresuid(&self) -> Result<ResUid, Errno> {
        nix::unistd::getresuid()
    }

    fn getresgid(&self) -> Result<ResGid, Errno> {
        nix::unistd::getresgid()
    }

    fn setresuid(&self, real: Uid, eff: Uid, saved: Uid) -> Result<(), Errno> {
        nix::unistd::setresuid(real, eff, saved)
    }

    fn setresgid(&self, real: Gid, eff: Gid, saved: Gid) -> Result<(), Errno> {
        nix::unistd::setresgid(real, eff, saved)
    }

    fn initgroups(&self, user: &CStr, primary_gid: Gid) -> Result<(), Errno> {
        nix::unistd::initgroups(user, primary_gid)
    }

    fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno> {
        User::from_name(name)
    }

    fn argv0(&self) -> String {
        std::env::args()
            .next()
            .unwrap_or_else(|| "miru-agent".into())
    }
}
