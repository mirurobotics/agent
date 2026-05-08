// standard crates
use std::ffi::CStr;

pub(crate) type Gid = nix::unistd::Gid;
pub(crate) type ResGid = nix::unistd::ResGid;
pub(crate) type ResUid = nix::unistd::ResUid;
pub(crate) type Uid = nix::unistd::Uid;
pub(crate) type User = nix::unistd::User;
pub(crate) type Errno = nix::errno::Errno;

/// Seam for every external interaction in the privilege module.
///
/// Production code constructs [`RealSystem`] which delegates to `nix::unistd`
/// and `std::env::args()`. Tests construct a fake that drives the same trait
/// against in-memory state, allowing branches like `WrongUser`,
/// `PostDropMismatch`, and per-syscall errno propagation to be exercised
/// deterministically without root.
pub(crate) trait System {
    fn geteuid(&self) -> Uid;
    fn getegid(&self) -> Gid;
    fn getresuid(&self) -> Result<ResUid, Errno>;
    fn getresgid(&self) -> Result<ResGid, Errno>;
    fn setresuid(&self, real: Uid, eff: Uid, saved: Uid) -> Result<(), Errno>;
    fn setresgid(&self, real: Gid, eff: Gid, saved: Gid) -> Result<(), Errno>;
    fn initgroups(&self, user: &CStr, primary_gid: Gid) -> Result<(), Errno>;
    fn getgroups(&self) -> Result<Vec<Gid>, Errno>;
    fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno>;
    fn argv0(&self) -> String;
}

/// Production implementation of [`System`] that delegates to `nix::unistd`
/// and `std::env::args()`.
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

    fn getgroups(&self) -> Result<Vec<Gid>, Errno> {
        nix::unistd::getgroups()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test for the read-only methods of `RealSystem`. The mutating
    /// methods (`setresuid`, `setresgid`, `initgroups`) are deliberately
    /// excluded — invoking them in the test process would actually drop
    /// privileges or rewrite supplementary groups, and there is no safe way
    /// to recover. Their behavior is exercised by the shell-driven smoke
    /// tests in `plans/completed/20260507-self-privilege-drop.md`.
    #[test]
    fn real_system_read_only_methods_are_self_consistent() {
        let sys = RealSystem;

        // euid/getresuid agree.
        let euid = sys.geteuid();
        let resuid = sys.getresuid().expect("getresuid must succeed");
        assert_eq!(resuid.effective, euid);

        // egid/getresgid agree.
        let egid = sys.getegid();
        let resgid = sys.getresgid().expect("getresgid must succeed");
        assert_eq!(resgid.effective, egid);

        // getgroups returns the supplementary-group list; semantics are
        // host-specific so we only assert the call succeeds.
        let _ = sys.getgroups().expect("getgroups must succeed");

        // argv0 always returns something — under cargo test it's the test
        // binary path; the env::args fallback to "miru-agent" guarantees
        // non-empty even in degenerate environments.
        assert!(!sys.argv0().is_empty());

        // lookup_user resolves root (guaranteed present on every Linux host).
        let root = sys
            .lookup_user("root")
            .expect("getpwnam_r succeeds")
            .expect("root is in /etc/passwd");
        assert_eq!(root.name, "root");
        assert_eq!(root.uid.as_raw(), 0);

        // lookup_user reports missing entries via Ok(None) on glibc; some
        // libcs use Err(ENOENT|ESRCH) which the caller in mod.rs maps the
        // same way. Either outcome is acceptable here.
        let missing = sys.lookup_user("nonexistent_user_xyz_123_miru_test");
        match missing {
            Ok(None) => {}
            Err(Errno::ENOENT | Errno::ESRCH) => {}
            other => panic!("expected Ok(None) or ENOENT/ESRCH, got {other:?}"),
        }
    }
}
