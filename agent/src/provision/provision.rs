// internal crates
use crate::cli;
use crate::crypt::rsa;
use crate::filesys::{self, Overwrite};
use crate::http;
use crate::models;
use crate::provision::errors::*;
use crate::storage::{self, settings};
use crate::version;
use backend_api::models as backend_client;

use super::shared::{build_settings, cleanup_temp_dir};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

/// The result of a `provision()` call.
///
/// `is_provisioned` is `true` when the machine was already provisioned
/// before this call — i.e., the call was a no-op and `device` is the
/// cached state read from `device.json`. It is `false` when this call
/// performed the full provisioning flow (keypair gen, backend POST,
/// bootstrap), in which case `device` is the freshly-issued backend
/// record.
#[derive(Debug)]
pub struct ProvisionOutcome {
    pub is_provisioned: bool,
    pub device: backend_client::Device,
}

pub async fn provision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
    device_name: Option<String>,
) -> Result<ProvisionOutcome, ProvisionErr> {
    // Idempotency short-circuit: if the machine is already activated and
    // device.json is parseable, return the cached device with `is_provisioned`
    // set so the caller can render an "already provisioned" message. We need
    // device.json to populate the outcome's device field. If it's missing
    // despite keys being present, the bootstrap was interrupted mid-way; fall
    // through and let the backend tell us whether re-provisioning is possible.
    if storage::assert_activated(layout).await.is_ok() {
        if let Ok(local_device) = layout.device().read_json::<models::Device>().await {
            return Ok(ProvisionOutcome {
                is_provisioned: true,
                device: backend_client::Device {
                    id: local_device.id,
                    name: local_device.name,
                    session_id: local_device.session_id,
                    ..backend_client::Device::default()
                },
            });
        }
    }

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
            version::VERSION,
        )
        .await?;
        Ok(ProvisionOutcome {
            is_provisioned: false,
            device,
        })
    }
    .await;

    cleanup_temp_dir(&temp_dir).await;
    result
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
    build_settings(
        args.backend_host.as_deref(),
        args.mqtt_broker_host.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
