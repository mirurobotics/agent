// internal crates
use crate::errors::Trace;

#[derive(Debug, thiserror::Error)]
pub enum PrivilegeErr {
    #[error("user '{name}' not found in /etc/passwd; is the miru .deb installed?")]
    UserNotFound { name: String, trace: Box<Trace> },

    #[error(
        "miru-agent must be run as root or the '{expected}' user, but is running as \
         uid {actual_uid} gid {actual_gid} (expected uid {expected_uid} gid \
         {expected_gid}).\nTry: sudo {argv0} ..."
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
        errno: i32,
        trace: Box<Trace>,
    },

    #[error(
        "post-drop verification failed: expected uid={expected_uid} gid={expected_gid}, got \
         ruid={actual_ruid} euid={actual_euid} suid={actual_suid} \
         rgid={actual_rgid} egid={actual_egid} sgid={actual_sgid}"
    )]
    PostDropMismatch {
        expected_uid: u32,
        expected_gid: u32,
        actual_ruid: u32,
        actual_euid: u32,
        actual_suid: u32,
        actual_rgid: u32,
        actual_egid: u32,
        actual_sgid: u32,
        trace: Box<Trace>,
    },
}

impl crate::errors::Error for PrivilegeErr {}
