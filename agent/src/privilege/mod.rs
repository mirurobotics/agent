// standard crates
#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};

// internal crates
use crate::errors::Trace;
use crate::trace;

// external crates

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
    let c_name = CString::new(name).map_err(|_| PrivilegeErr::UserNotFound {
        name: name.to_string(),
        trace: trace!(),
    })?;

    // Start with a 1 KiB buffer; double on ERANGE up to a sane cap. Most
    // passwd entries fit in 256 bytes; the cap protects against a pathological
    // /etc/passwd.
    let mut buf_len: usize = 1024;
    const MAX_BUF_LEN: usize = 64 * 1024;

    loop {
        let mut buf: Vec<libc::c_char> = vec![0; buf_len];
        // SAFETY: passwd is filled in by getpwnam_r. We treat it as
        // uninitialized until the call returns 0 with a non-null result.
        let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result_ptr: *mut libc::passwd = std::ptr::null_mut();

        // SAFETY: c_name is a valid NUL-terminated string. buf has length
        // buf_len, and we pass that length to getpwnam_r. result_ptr is set
        // by the function on success.
        let rc = unsafe {
            libc::getpwnam_r(
                c_name.as_ptr(),
                &mut passwd,
                buf.as_mut_ptr(),
                buf_len,
                &mut result_ptr,
            )
        };

        if rc == 0 {
            if result_ptr.is_null() {
                // No entry found, no error.
                return Err(PrivilegeErr::UserNotFound {
                    name: name.to_string(),
                    trace: trace!(),
                });
            }
            // SAFETY: result_ptr is non-null and points at `passwd`, whose
            // string fields point into `buf`. Both live until the end of this
            // iteration. We copy the name out before `buf` is dropped.
            let resolved_name = unsafe {
                CStr::from_ptr((*result_ptr).pw_name)
                    .to_string_lossy()
                    .into_owned()
            };
            return Ok(UserInfo {
                uid: passwd.pw_uid as u32,
                gid: passwd.pw_gid as u32,
                name: resolved_name,
            });
        }

        if rc == libc::ERANGE {
            buf_len = buf_len.saturating_mul(2);
            if buf_len > MAX_BUF_LEN {
                return Err(PrivilegeErr::Syscall {
                    call: "getpwnam_r",
                    errno: libc::ERANGE,
                    trace: trace!(),
                });
            }
            continue;
        }

        // POSIX says non-ERANGE non-zero rc means an actual error. ENOENT /
        // ESRCH / 0-return-with-null-result all mean "not found"; some libc
        // implementations may return them as rc instead of via result_ptr.
        if rc == libc::ENOENT || rc == libc::ESRCH {
            return Err(PrivilegeErr::UserNotFound {
                name: name.to_string(),
                trace: trace!(),
            });
        }

        return Err(PrivilegeErr::Syscall {
            call: "getpwnam_r",
            errno: rc,
            trace: trace!(),
        });
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
    // SAFETY: geteuid is always safe to call.
    let euid = unsafe { libc::geteuid() };

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

    // SAFETY: c_name is a valid NUL-terminated string; gid is a valid gid.
    let rc = unsafe { libc::initgroups(c_name.as_ptr(), info.gid as libc::gid_t) };
    if rc != 0 {
        return Err(PrivilegeErr::Syscall {
            call: "initgroups",
            errno: errno_now(),
            trace: trace!(),
        });
    }

    // SAFETY: setgid is always safe to call; we check the return value.
    let rc = unsafe { libc::setgid(info.gid as libc::gid_t) };
    if rc != 0 {
        return Err(PrivilegeErr::Syscall {
            call: "setgid",
            errno: errno_now(),
            trace: trace!(),
        });
    }

    // SAFETY: setuid is always safe to call; we check the return value.
    let rc = unsafe { libc::setuid(info.uid as libc::uid_t) };
    if rc != 0 {
        return Err(PrivilegeErr::Syscall {
            call: "setuid",
            errno: errno_now(),
            trace: trace!(),
        });
    }

    // Verify that the drop took effect. setuid succeeding with EUID still 0
    // would be a kernel bug, but the cost of checking is one syscall and the
    // failure mode (still root after we expected to drop) is catastrophic.
    // SAFETY: geteuid / getegid are always safe to call.
    let actual_uid = unsafe { libc::geteuid() };
    let actual_gid = unsafe { libc::getegid() };
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

#[cfg(target_os = "linux")]
fn errno_now() -> i32 {
    // SAFETY: __errno_location is always safe to call and returns a pointer
    // to thread-local storage.
    unsafe { *libc::__errno_location() }
}
