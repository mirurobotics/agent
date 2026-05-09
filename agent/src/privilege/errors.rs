// internal crates
use crate::errors::Trace;

// external crates
use nix::errno::Errno;

#[derive(Debug, thiserror::Error)]
pub enum PrivilegeErr {
    #[error("user '{name}' not found in /etc/passwd; is the miru .deb installed?")]
    UserNotFound { name: String, trace: Box<Trace> },

    #[error(
        "miru-agent must be run as the '{expected}' user, but is running as \
         uid {actual_uid} gid {actual_gid} (expected uid {expected_uid} gid \
         {expected_gid}).\nTry: sudo -u {expected} {argv0} ..."
    )]
    WrongUser {
        expected: String,
        actual_uid: u32,
        actual_gid: u32,
        expected_uid: u32,
        expected_gid: u32,
        argv0: String,
        trace: Box<Trace>,
    },

    #[error("syscall '{call}' failed: errno={errno}")]
    Syscall {
        call: &'static str,
        errno: Errno,
        trace: Box<Trace>,
    },
}

impl crate::errors::Error for PrivilegeErr {}
