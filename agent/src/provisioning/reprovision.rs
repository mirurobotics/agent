// internal crates
use crate::cli;
use crate::crypt::rsa;
use crate::filesys::{self, Overwrite};
use crate::http;
use crate::provisioning::{errors::*, shared};
use crate::storage::{self, settings};
use crate::version;
use backend_api::models as backend_client;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub async fn reprovision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
) -> Result<backend_client::Device, ProvisionErr> {
    let temp_dir = layout.temp_dir();

    let result = async {
        // generate new public and private keys in a temporary directory which
        // will become the device's new authentication if successful
        let private_key_file = temp_dir.file("private.key");
        let public_key_file = temp_dir.file("public.key");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow).await?;

        let device = reprovision_with_backend(http_client, &public_key_file, token).await?;
        storage::setup::bootstrap(
            layout,
            &(&device).into(),
            settings,
            &private_key_file,
            &public_key_file,
            version::VERSION,
        )
        .await?;
        Ok(device)
    }
    .await;

    shared::cleanup_temp_dir(&temp_dir).await;
    result
}

async fn reprovision_with_backend<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    public_key_file: &filesys::File,
    token: &str,
) -> Result<backend_client::Device, ProvisionErr> {
    let public_key_pem = public_key_file.read_string().await?;
    let payload = backend_client::ReprovisionDeviceRequest {
        public_key_pem,
        agent_version: version::VERSION.to_string(),
    };
    let params = http::devices::ReprovisionParams {
        payload: &payload,
        token,
    };
    Ok(http::devices::reprovision(http_client, params).await?)
}

pub fn determine_settings(args: &cli::ReprovisionArgs) -> settings::Settings {
    shared::determine_settings(
        args.backend_host.as_deref(),
        args.mqtt_broker_host.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    mod determine_reprovision_settings {
        use super::*;

        #[test]
        fn backend_host_appends_agent_v1_suffix() {
            let args = cli::ReprovisionArgs {
                backend_host: Some("https://custom.mirurobotics.com".to_string()),
                ..Default::default()
            };

            let settings = determine_settings(&args);

            assert_eq!(
                settings.backend.base_url.as_str(),
                "https://custom.mirurobotics.com/agent/v1"
            );
        }

        #[test]
        fn mqtt_broker_host_override() {
            let args = cli::ReprovisionArgs {
                mqtt_broker_host: Some("mqtt.custom.mirurobotics.com".to_string()),
                ..Default::default()
            };

            let settings = determine_settings(&args);

            assert_eq!(
                settings.mqtt_broker.host.as_str(),
                "mqtt.custom.mirurobotics.com"
            );
        }

        #[test]
        fn no_overrides_preserves_defaults() {
            let args = cli::ReprovisionArgs::default();
            let defaults = settings::Settings::default();

            let settings = determine_settings(&args);

            assert_eq!(settings.backend.base_url, defaults.backend.base_url);
            assert_eq!(settings.mqtt_broker.host, defaults.mqtt_broker.host);
        }
    }
}
