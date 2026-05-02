// internal crates
use crate::cli;
use crate::crypt::rsa;
use crate::filesys::{self, Overwrite};
use crate::http;
use crate::models;
use crate::provisioning::{errors::*, shared};
use crate::storage::{self, settings};
use crate::telemetry;
use crate::version;
use backend_api::models as backend_client;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[derive(Debug)]
pub struct Outcome {
    pub already_provisioned: bool,
    pub device_name: String,
}

pub async fn provision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
    device_name: Option<String>,
) -> Result<Outcome, ProvisionErr> {
    // if a machine has already been provisioned, then just return the device's name
    if storage::assert_activated(layout).await.is_ok() {
        let device_name = match layout.device().read_json::<models::Device>().await {
            Ok(device) => device.name,
            Err(e) => {
                error!("unable to read device.json: {e}");
                "unknown".to_string()
            }
        };
        return Ok(Outcome {
            already_provisioned: true,
            device_name,
        });
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
        Ok(Outcome {
            already_provisioned: false,
            device_name: device.name,
        })
    }
    .await;

    shared::cleanup_temp_dir(&temp_dir).await;
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
        name: device_name.unwrap_or_else(telemetry::SystemInfo::host_name),
    };
    let params = http::devices::ProvisionParams {
        payload: &payload,
        token,
    };
    Ok(http::devices::provision(http_client, params).await?)
}

pub fn determine_settings(args: &cli::ProvisionArgs) -> settings::Settings {
    shared::determine_settings(
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
