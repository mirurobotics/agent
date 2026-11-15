// internal crates
use crate::models::status::impl_status_enum;
use backend_api::models as backend_client;

// external crates
use serde::Serialize;
use tracing::warn;

// ======================================= OS ======================================= //
// No `Default`: the only sensible fallback is `Os::HOST`, which the macro and the
// release and file rule deserializers use directly.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    Linux,
    Windows,
}

impl_status_enum!(
    enum Os,
    default: HOST,
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
    /// and file rule the agent caches. That makes it the one fallback for an `os` the
    /// agent can't read: an unknown wire or backend value, or a cache entry written
    /// before `os` existed.
    pub const HOST: Os = if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    };
}
