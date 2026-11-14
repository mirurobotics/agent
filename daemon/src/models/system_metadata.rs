// internal crates
use crate::telemetry;
use backend_api::models as backend_client;

// external crates
use serde::{Deserialize, Serialize};

/// Optional device system metadata: the OS family and CPU architecture from the
/// build-time target vocabulary, plus the host's hostname and OS/kernel version
/// strings. Any field that cannot be determined on the running host is `None`, so
/// it is omitted from the serialized request body rather than sent as an empty
/// string.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemMetadata {
    pub os: Option<backend_client::Os>,
    pub arch: Option<backend_client::Arch>,
    pub hostname: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
}

impl SystemMetadata {
    /// Build a device-update request carrying only these system-metadata fields.
    /// `agent_version` is left `None` (this request never sets the running
    /// version), and every `None` field is omitted from the serialized body.
    pub fn to_update_request(&self) -> backend_client::UpdateDeviceFromAgentRequest {
        backend_client::UpdateDeviceFromAgentRequest {
            agent_version: None,
            os: self.os,
            hostname: self.hostname.clone(),
            arch: self.arch,
            os_version: self.os_version.clone(),
            kernel_version: self.kernel_version.clone(),
        }
    }
}

/// Map the build-time OS family ([`std::env::consts::OS`]) to the backend enum.
/// An unrecognized OS yields `None` so provisioning never fails on it.
fn map_os(os: &str) -> Option<backend_client::Os> {
    match os {
        "linux" => Some(backend_client::Os::OS_LINUX),
        "windows" => Some(backend_client::Os::OS_WINDOWS),
        _ => None,
    }
}

/// Map the build-time CPU architecture ([`std::env::consts::ARCH`]) to the
/// backend enum. An unrecognized architecture yields `None`.
fn map_arch(arch: &str) -> Option<backend_client::Arch> {
    match arch {
        "x86_64" => Some(backend_client::Arch::ARCH_X86_64),
        "aarch64" => Some(backend_client::Arch::ARCH_AARCH64),
        _ => None,
    }
}

/// Treat a blank string as absent so empty telemetry values are omitted from
/// the request rather than sent as empty strings.
fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Pure builder over explicit inputs so it can be driven by tests with known
/// values; the host is read only by [`system_metadata`].
fn build_system_metadata(
    os: &str,
    arch: &str,
    hostname: String,
    os_version: String,
    kernel_version: String,
) -> SystemMetadata {
    SystemMetadata {
        os: map_os(os),
        arch: map_arch(arch),
        hostname: non_empty(hostname),
        os_version: non_empty(os_version),
        kernel_version: non_empty(kernel_version),
    }
}

/// Gather the running host's system metadata. `os`/`arch` come from the
/// compile-time [`std::env::consts::OS`] / [`std::env::consts::ARCH`]
/// vocabulary; the human-readable strings come from telemetry.
pub fn system_metadata() -> SystemMetadata {
    build_system_metadata(
        std::env::consts::OS,
        std::env::consts::ARCH,
        telemetry::SystemInfo::host_name(),
        telemetry::SystemInfo::os(),
        telemetry::SystemInfo::kernel_version(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_os_maps_supported_families() {
        assert_eq!(map_os("linux"), Some(backend_client::Os::OS_LINUX));
        assert_eq!(map_os("windows"), Some(backend_client::Os::OS_WINDOWS));
    }

    #[test]
    fn map_os_returns_none_for_unsupported() {
        assert_eq!(map_os("macos"), None);
        assert_eq!(map_os("freebsd"), None);
        assert_eq!(map_os(""), None);
    }

    #[test]
    fn map_arch_maps_supported_architectures() {
        assert_eq!(map_arch("x86_64"), Some(backend_client::Arch::ARCH_X86_64));
        assert_eq!(
            map_arch("aarch64"),
            Some(backend_client::Arch::ARCH_AARCH64)
        );
    }

    #[test]
    fn map_arch_returns_none_for_unsupported() {
        assert_eq!(map_arch("arm"), None);
        assert_eq!(map_arch("x86"), None);
        assert_eq!(map_arch(""), None);
    }

    #[test]
    fn build_populates_all_fields_from_known_inputs() {
        let meta = build_system_metadata(
            "linux",
            "x86_64",
            "robot-1.local".to_string(),
            "Ubuntu 22.04".to_string(),
            "5.15.0-91-generic".to_string(),
        );

        assert_eq!(
            meta,
            SystemMetadata {
                os: Some(backend_client::Os::OS_LINUX),
                arch: Some(backend_client::Arch::ARCH_X86_64),
                hostname: Some("robot-1.local".to_string()),
                os_version: Some("Ubuntu 22.04".to_string()),
                kernel_version: Some("5.15.0-91-generic".to_string()),
            }
        );
    }

    #[test]
    fn build_omits_unsupported_and_blank_fields() {
        let meta = build_system_metadata(
            "macos",
            "arm",
            String::new(),
            "   ".to_string(),
            String::new(),
        );

        assert_eq!(meta, SystemMetadata::default());
    }

    #[test]
    fn system_metadata_reads_host_os_and_arch() {
        let meta = system_metadata();

        assert_eq!(meta.os, map_os(std::env::consts::OS));
        assert_eq!(meta.arch, map_arch(std::env::consts::ARCH));
    }

    #[test]
    fn to_update_request_maps_all_fields_with_no_agent_version() {
        let meta = SystemMetadata {
            os: Some(backend_client::Os::OS_LINUX),
            arch: Some(backend_client::Arch::ARCH_X86_64),
            hostname: Some("robot-1.local".to_string()),
            os_version: Some("Ubuntu 22.04".to_string()),
            kernel_version: Some("5.15.0-91-generic".to_string()),
        };

        let request = meta.to_update_request();

        assert_eq!(
            request,
            backend_client::UpdateDeviceFromAgentRequest {
                agent_version: None,
                os: Some(backend_client::Os::OS_LINUX),
                hostname: Some("robot-1.local".to_string()),
                arch: Some(backend_client::Arch::ARCH_X86_64),
                os_version: Some("Ubuntu 22.04".to_string()),
                kernel_version: Some("5.15.0-91-generic".to_string()),
            }
        );
    }
}
