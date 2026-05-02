// standard crates
use std::env;

// internal crates
use crate::filesys;
use crate::network::{BackendUrl, MqttHost};
use crate::provisioning::errors::*;
use crate::storage::settings;

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
    if let Err(e) = temp_dir.delete().await {
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
        let raw = format!("{host}/agent/v1");
        settings.backend.base_url = BackendUrl::new(&raw).unwrap_or_else(|msg| {
            let fallback = BackendUrl::default();
            warn!(
                "backend host override `{raw}` rejected ({msg}); falling back to default `{fallback}`"
            );
            fallback
        });
    }
    if let Some(host) = mqtt_broker_host {
        settings.mqtt_broker.host = MqttHost::new(host).unwrap_or_else(|msg| {
            let fallback = MqttHost::default();
            warn!(
                "mqtt broker host override `{host}` rejected ({msg}); falling back to default `{fallback}`"
            );
            fallback
        });
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
