// internal crates
use crate::errors::Trace;

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
        expected_uid: nix::unistd::Uid,
        expected_gid: nix::unistd::Gid,
        actual_uid: nix::unistd::Uid,
        actual_gid: nix::unistd::Gid,
        trace: Box<Trace>,
    },
}

impl crate::errors::Error for PrivilegeErr {}
