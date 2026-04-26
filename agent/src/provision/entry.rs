// standard crates
use std::env;

// internal crates
use crate::cli;
use crate::crypt::rsa;
use crate::filesys::{self, Overwrite};
use crate::http;
use crate::provision::errors::*;
use crate::storage::{self, settings};
use crate::version;
use backend_api::models as backend_client;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub async fn provision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
    device_name: Option<String>,
) -> Result<backend_client::Device, ProvisionErr> {
    let temp_dir = layout.temp_dir();

    let result = async {
        // generate new public and private keys in a temporary directory which will be
        // the device's new authentication if the activation is successful
        let private_key_file = temp_dir.file("private.key");
        let public_key_file = temp_dir.file("public.key");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow).await?;

        let device =
            provision_with_backend(http_client, &public_key_file, token, device_name).await?;
        storage::setup::bootstrap(
            layout,
            &(&device).into(),
            settings,
            &private_key_file,
            &public_key_file,
        )
        .await?;
        Ok(device)
    }
    .await;

    if let Err(e) = temp_dir.delete().await {
        debug_assert!(false, "failed to clean up temp dir: {e}");
        warn!("failed to clean up temp dir: {e}");
    }
    result
}

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

async fn provision_with_backend<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    public_key_file: &filesys::File,
    token: &str,
    device_name: Option<String>,
) -> Result<backend_client::Device, ProvisionErr> {
    let public_key_pem = public_key_file.read_string().await?;
    let payload = backend_client::ProvisionDeviceRequest {
        public_key_pem,
        agent_version: version::VERSION.to_string(),
        name: device_name,
    };
    let params = http::devices::ProvisionParams {
        payload: &payload,
        token,
    };
    Ok(http::devices::provision(http_client, params).await?)
}

pub fn determine_settings(args: &cli::ProvisionArgs) -> settings::Settings {
    let mut settings = settings::Settings::default();
    if let Some(backend_host) = &args.backend_host {
        settings.backend.base_url = format!("{}/agent/v1", backend_host);
    }
    if let Some(mqtt_broker_host) = &args.mqtt_broker_host {
        settings.mqtt_broker.host = mqtt_broker_host.to_string();
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
    }

    mod determine_settings {
        use super::*;

        #[test]
        fn backend_host_appends_agent_v1_suffix() {
            let args = cli::ProvisionArgs {
                backend_host: Some("https://custom.example.com".to_string()),
                ..Default::default()
            };

            let settings = determine_settings(&args);

            assert_eq!(
                settings.backend.base_url,
                "https://custom.example.com/agent/v1"
            );
        }

        #[test]
        fn mqtt_broker_host_override() {
            let args = cli::ProvisionArgs {
                mqtt_broker_host: Some("mqtt.custom.example.com".to_string()),
                ..Default::default()
            };

            let settings = determine_settings(&args);

            assert_eq!(settings.mqtt_broker.host, "mqtt.custom.example.com");
        }

        #[test]
        fn no_overrides_preserves_defaults() {
            let args = cli::ProvisionArgs::default();
            let defaults = settings::Settings::default();

            let settings = determine_settings(&args);

            assert_eq!(settings.backend.base_url, defaults.backend.base_url);
            assert_eq!(settings.mqtt_broker.host, defaults.mqtt_broker.host);
        }
    }
}
