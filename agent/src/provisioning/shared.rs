// standard crates
use std::env;

// internal crates
use crate::disk::settings;
use crate::filesys::{self, dirs};
use crate::network::{BackendHost, MqttHost};
use crate::provisioning::errors::*;
use crate::telemetry;
use backend_api::models as backend_client;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

const TOKEN_ENV_VAR: &str = "MIRU_PROVISIONING_TOKEN";

pub fn read_token_from_env() -> Result<String, ProvisionErr> {
    if let Ok(token) = env::var(TOKEN_ENV_VAR) {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    error!("The {TOKEN_ENV_VAR} environment variable is not set");
    Err(ProvisionErr::MissingEnvVarErr(MissingEnvVarErr {
        name: TOKEN_ENV_VAR.to_string(),
        trace: crate::trace!(),
    }))
}

pub(super) async fn cleanup_temp_dir(temp_dir: &filesys::Dir) {
    if let Err(e) = dirs::delete(temp_dir).await {
        debug_assert!(false, "failed to clean up temp dir: {e}");
        warn!("failed to clean up temp dir: {e}");
    }
}

pub(super) fn determine_settings(
    backend_host: Option<&str>,
    mqtt_broker_host: Option<&str>,
) -> settings::Settings {
    let mut settings = settings::Settings::default();
    if let Some(host) = backend_host {
        settings.backend.host = BackendHost::new_or(host, BackendHost::default());
    }
    if let Some(host) = mqtt_broker_host {
        settings.mqtt_broker.host = MqttHost::new_or(host, MqttHost::default());
    }
    settings
}

/// Optional device system metadata reported to the backend on provision and
/// reprovision. Any field that cannot be determined on the running host is
/// `None`, so it is omitted from the request body rather than sent as an
/// empty string.
#[derive(Debug, Default, PartialEq)]
pub(super) struct SystemMetadata {
    pub os: Option<backend_client::Os>,
    pub arch: Option<backend_client::Arch>,
    pub hostname: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
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
pub(super) fn system_metadata() -> SystemMetadata {
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
    use std::sync::{Mutex, OnceLock};

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock should not be poisoned")
    }

    mod read_token_from_env {
        use super::*;

        #[test]
        fn returns_token_when_set() {
            let _env_lock = lock_env();
            env::set_var("MIRU_PROVISIONING_TOKEN", "test-token-123");
            let result = read_token_from_env();
            assert_eq!(result.unwrap(), "test-token-123");
            env::remove_var("MIRU_PROVISIONING_TOKEN");
        }

        #[test]
        fn returns_error_when_not_set() {
            let _env_lock = lock_env();
            env::remove_var("MIRU_PROVISIONING_TOKEN");
            let result = read_token_from_env();
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                matches!(err, ProvisionErr::MissingEnvVarErr(ref e) if e.name == "MIRU_PROVISIONING_TOKEN"),
                "expected MissingEnvVarErr, got: {err:?}"
            );
        }

        #[test]
        fn returns_error_when_empty() {
            let _env_lock = lock_env();
            env::set_var("MIRU_PROVISIONING_TOKEN", "");
            let result = read_token_from_env();
            env::remove_var("MIRU_PROVISIONING_TOKEN");
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                matches!(err, ProvisionErr::MissingEnvVarErr(ref e) if e.name == "MIRU_PROVISIONING_TOKEN"),
                "expected MissingEnvVarErr, got: {err:?}"
            );
        }
    }

    mod system_metadata {
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
    }
}
