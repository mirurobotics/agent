// internal crates
use crate::models::status::impl_status_enum;
use backend_api::models as backend_client;

// external crates
use serde::Serialize;
use tracing::warn;

// ======================================= OS ======================================= //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    #[default]
    Linux,
    Windows,
}

impl_status_enum!(
    enum Os,
    default: Linux,
    label: "os",
    log: warn,
    backend_type: backend_client::Os,
    unknown_backend: backend_client::Os::OsUnknown,
    mappings: [
        Linux => "linux" => backend_client::Os::OS_LINUX,
        Windows => "windows" => backend_client::Os::OS_WINDOWS,
    ]
);

impl Os {
    /// The OS family this agent was built for. The backend rejects deployments whose
    /// release OS differs from the device's, so this is also the OS of every release
    /// and file rule the agent caches, which makes it the fallback for cache entries
    /// written before `os` existed.
    pub const HOST: Os = if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    };
}
