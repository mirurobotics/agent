// standard crates
use std::env;

// internal crates
use crate::disk::{self, settings};
use crate::filesys::{self, files, PathExt};
use crate::network::{BackendHost, MqttHost};
use crate::provisioning::errors::*;

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

/// Refuses to provision unless every installer sentinel is a real directory.
///
/// On Windows the MSI creates the state folders with a service-specific ACL;
/// folders recreated by the agent after a wipe inherit ProgramData's defaults,
/// and the service later fails to replace `device.json` with OS error 5. This
/// check only reads metadata and creates nothing.
pub fn assert_installer_layout(layout: &disk::Layout) -> Result<(), ProvisionErr> {
    for dir in layout.installer_sentinels() {
        match std::fs::symlink_metadata(dir.path()) {
            Ok(m) if m.is_dir() => {}
            _ => {
                return Err(ProvisionErr::InstallerLayoutErr(InstallerLayoutErr {
                    missing: dir.path().clone(),
                    trace: crate::trace!(),
                }));
            }
        }
    }
    Ok(())
}

// tmp\ itself is installer-owned on Windows; delete only what provisioning wrote.
pub(super) async fn cleanup_temp_files(temp_files: &[&filesys::File]) {
    for file in temp_files {
        if let Err(e) = files::delete(file).await {
            debug_assert!(false, "failed to clean up temp file: {e}");
            warn!("failed to clean up temp file: {e}");
        }
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
}
