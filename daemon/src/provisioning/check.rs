// internal crates
use crate::disk::{self, Activation, DiskErr};

// The exit codes below are public interface. Ansible playbooks encode
// `rc in [0, 3]` as success, so changing them is a breaking change.
pub const EXIT_PROVISIONED: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_NOT_PROVISIONED: i32 = 3;

#[derive(Debug)]
pub enum Report {
    Provisioned,
    NotProvisioned,
    Undeterminable(DiskErr),
}

pub fn check(layout: &disk::Layout) -> Report {
    match disk::activation_state(layout) {
        Ok(Activation::Activated) => Report::Provisioned,
        Ok(Activation::NotActivated) => Report::NotProvisioned,
        Err(e) => Report::Undeterminable(e),
    }
}

impl Report {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Provisioned => EXIT_PROVISIONED,
            Self::NotProvisioned => EXIT_NOT_PROVISIONED,
            Self::Undeterminable(_) => EXIT_ERROR,
        }
    }

    pub fn stdout_line(&self) -> Option<&'static str> {
        match self {
            Self::Provisioned => Some("provisioned"),
            Self::NotProvisioned => Some("not provisioned"),
            Self::Undeterminable(_) => None,
        }
    }

    pub fn stderr_line(&self) -> Option<String> {
        match self {
            Self::Provisioned | Self::NotProvisioned => None,
            Self::Undeterminable(e) => Some(format!("miru-agent: {e}")),
        }
    }
}
