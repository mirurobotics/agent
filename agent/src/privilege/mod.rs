// standard crates
#[cfg(target_os = "linux")]
use std::ffi::CString;

// internal crates
use crate::errors::Trace;
use crate::trace;

// external crates
#[cfg(target_os = "linux")]
use nix::errno::Errno;
#[cfg(target_os = "linux")]
use nix::unistd::{Gid, Uid, User, getegid, geteuid, initgroups, setgid, setuid};

/// The system user the agent runs as in production. Created by the `.deb`
/// `postinst` script (`useradd -r -g miru -s /bin/false miru`).
pub const TARGET_USER: &str = "miru";

/// The system group the agent runs as in production. Created by the `.deb`
/// `postinst` script (`groupadd -r miru`).
pub const TARGET_GROUP: &str = "miru";

/// Resolved passwd entry for a system user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PrivilegeErr {
    #[error("user '{name}' not found in /etc/passwd; is the miru .deb installed?")]
    UserNotFound { name: String, trace: Box<Trace> },

    #[error(
        "miru-agent must be run as root or the '{expected}' user, but is running as uid \
         {actual_uid}.\nTry: sudo {argv0} ..."
    )]
    WrongUser {
        expected: String,
        actual_uid: u32,
        argv0: String,
        trace: Box<Trace>,
    },

    #[error("syscall '{call}' failed: errno={errno}")]
    Syscall {
        call: &'static str,
        errno: i32,
        trace: Box<Trace>,
    },

    #[error(
        "post-drop verification failed: expected uid={expected_uid} gid={expected_gid}, got \
         uid={actual_uid} gid={actual_gid}"
    )]
    PostDropMismatch {
        expected_uid: u32,
        expected_gid: u32,
        actual_uid: u32,
        actual_gid: u32,
        trace: Box<Trace>,
    },
}

impl crate::errors::Error for PrivilegeErr {}

/// Look up the uid/gid for a passwd entry by name. Returns
/// `Err(PrivilegeErr::UserNotFound)` if the user is not present in the system
/// passwd database.
///
/// On non-Linux targets (e.g. macOS dev machines) this is a stub that always
/// returns `UserNotFound` so cross-platform `cargo test` keeps compiling. The
/// agent only ships on Linux.
#[cfg(target_os = "linux")]
pub fn lookup_user(name: &str) -> Result<UserInfo, PrivilegeErr> {
    // Pre-check NUL bytes. `nix::unistd::User::from_name` takes `&str` and
    // builds a CString internally; an embedded NUL would otherwise turn into
    // an opaque error variant. The intent is "no such user in /etc/passwd",
    // which maps onto `UserNotFound`.
    if name.contains('\0') {
        return Err(PrivilegeErr::UserNotFound {
            name: name.to_string(),
            trace: trace!(),
        });
    }

    match User::from_name(name) {
        Ok(Some(u)) => Ok(UserInfo {
            uid: u.uid.as_raw(),
            gid: u.gid.as_raw(),
            name: u.name,
        }),
        Ok(None) => Err(PrivilegeErr::UserNotFound {
            name: name.to_string(),
            trace: trace!(),
        }),
        // Some libc implementations surface "no such user" via ENOENT/ESRCH on
        // the return path rather than a null result pointer. Treat those as
        // `UserNotFound` to match the previous libc-direct behavior and the
        // semantic intent ("no entry in /etc/passwd").
        Err(Errno::ENOENT | Errno::ESRCH) => Err(PrivilegeErr::UserNotFound {
            name: name.to_string(),
            trace: trace!(),
        }),
        Err(e) => Err(PrivilegeErr::Syscall {
            call: "getpwnam_r",
            errno: e as i32,
            trace: trace!(),
        }),
    }
}

/// Non-Linux stub: the binary does not ship on these platforms, but tests
/// must compile. Always reports the user as not found.
#[cfg(not(target_os = "linux"))]
pub fn lookup_user(name: &str) -> Result<UserInfo, PrivilegeErr> {
    Err(PrivilegeErr::UserNotFound {
        name: name.to_string(),
        trace: trace!(),
    })
}

/// If running as root, drop privileges to `TARGET_USER`. If already running
/// as that user, no-op. Otherwise, return `WrongUser`.
///
/// Note on environment: the Linux `setuid(2)` and `setgid(2)` syscalls only
/// mutate process credentials; they do not touch the environ block. Env vars
/// set before this call (e.g. `MIRU_PROVISIONING_TOKEN`) remain readable
/// afterwards via `std::env::var`. Do not introduce explicit env preservation
/// logic — there is nothing to preserve.
#[cfg(target_os = "linux")]
pub fn ensure_dropped_or_already_unprivileged() -> Result<(), PrivilegeErr> {
    let euid = geteuid().as_raw();

    if euid != 0 {
        // Non-root: tolerate only the case where we are already running as
        // the target user. Look up the user and compare.
        match lookup_user(TARGET_USER) {
            Ok(info) if info.uid == euid => return Ok(()),
            Ok(_) | Err(PrivilegeErr::UserNotFound { .. }) => {
                let argv0 = std::env::args()
                    .next()
                    .unwrap_or_else(|| "miru-agent".into());
                return Err(PrivilegeErr::WrongUser {
                    expected: TARGET_USER.to_string(),
                    actual_uid: euid,
                    argv0,
                    trace: trace!(),
                });
            }
            Err(e) => return Err(e),
        }
    }

    // Running as root: drop to TARGET_USER. Order matters:
    //   1. initgroups — set supplementary groups (still root, can read /etc/group)
    //   2. setgid     — switch primary gid (still root)
    //   3. setuid     — switch uid; irreversible
    let info = lookup_user(TARGET_USER)?;
    let c_name = CString::new(info.name.as_str()).map_err(|_| PrivilegeErr::UserNotFound {
        name: info.name.clone(),
        trace: trace!(),
    })?;

    initgroups(&c_name, Gid::from_raw(info.gid)).map_err(|e| PrivilegeErr::Syscall {
        call: "initgroups",
        errno: e as i32,
        trace: trace!(),
    })?;

    setgid(Gid::from_raw(info.gid)).map_err(|e| PrivilegeErr::Syscall {
        call: "setgid",
        errno: e as i32,
        trace: trace!(),
    })?;

    setuid(Uid::from_raw(info.uid)).map_err(|e| PrivilegeErr::Syscall {
        call: "setuid",
        errno: e as i32,
        trace: trace!(),
    })?;

    // Verify that the drop took effect. setuid succeeding with EUID still 0
    // would be a kernel bug, but the cost of checking is two syscalls and the
    // failure mode (still root after we expected to drop) is catastrophic.
    let actual_uid = geteuid().as_raw();
    let actual_gid = getegid().as_raw();
    if actual_uid != info.uid || actual_gid != info.gid {
        return Err(PrivilegeErr::PostDropMismatch {
            expected_uid: info.uid,
            expected_gid: info.gid,
            actual_uid,
            actual_gid,
            trace: trace!(),
        });
    }

    Ok(())
}

/// Non-Linux stub: privilege drop is a no-op on platforms the agent does not
/// ship on. Production behavior is unaffected.
#[cfg(not(target_os = "linux"))]
pub fn ensure_dropped_or_already_unprivileged() -> Result<(), PrivilegeErr> {
    Ok(())
}
