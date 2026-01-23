// standard library
use std::collections::HashMap;
use std::env;

// internal crates
use crate::crypt::{jwt, rsa};
use crate::filesys::{dir::Dir, file::File, path::PathExt};
use crate::http::{client::HTTPClient, devices::DevicesExt};
use crate::installer::{display, errors::*};
use crate::logs::{init, LogOptions};
use crate::models::device::{Device, DeviceStatus};
use crate::storage::{layout::StorageLayout, settings, setup::clean_storage_setup};
use crate::utils::version_info;
use crate::trace;
use openapi_client::models::ActivateDeviceRequest;

// external crates
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub async fn install(cli_args: &HashMap<String, String>) {
    match install_helper(cli_args).await {
        Ok(_) => {
            info!("Installation successful");
        }
        Err(e) => {
            error!("Installation failed: {:?}", e);
            display::print_err_msg(Some(e.to_string()));
            std::process::exit(1);
        }
    }
}

async fn install_helper(
    cli_args: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = Dir::create_temp_dir("miru-agent-installer-logs").await?;
    let options = LogOptions {
        // sending logs to stdout will interfere with the installer outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let guard = init(options)?;

    let mut settings = settings::Settings::default();

    // retrieve the activation token
    let token_env_var = "MIRU_ACTIVATION_TOKEN";
    let activation_token = match env::var(token_env_var) {
        Ok(token) => token,
        Err(_) => {
            let msg = format!("The {} environment variable is not set", token_env_var);
            error!("{}", msg);
            return Err(msg.into());
        }
    };

    // set optional settings
    if let Some(backend_host) = cli_args.get("backend-host") {
        settings.backend.base_url = format!("{}/agent/v1", backend_host);
    }
    if let Some(mqtt_broker_host) = cli_args.get("mqtt-broker-host") {
        settings.mqtt_broker.host = mqtt_broker_host.to_string();
    }

    // run the installation
    let http_client = HTTPClient::new(&settings.backend.base_url).await;
    let layout = StorageLayout::default();
    bootstrap(
        &layout, &http_client, &settings, activation_token.as_str(),
        cli_args.get("device-name").map(|name| name.to_string()),
    )
    .await?;

    drop(guard);

    Ok(())
}

// walks user through the installation process
pub async fn bootstrap<HTTPClientT: DevicesExt>(
    layout: &StorageLayout,
    http_client: &HTTPClientT,
    settings: &settings::Settings,
    token: &str,
    device_name: Option<String>,
) -> Result<(), InstallErr> {
    // generate new public and private keys in a temporary directory which will be the
    // device's new authentication if the activation is successful
    let temp_dir = layout.temp_dir();
    let private_key_file = temp_dir.file("private.key");
    let public_key_file = temp_dir.file("public.key");
    rsa::gen_key_pair(
        4096, &private_key_file, &public_key_file, true,
    ).await.map_err(|e| {
        InstallErr::CryptErr(InstallCryptErr {source: e, trace: trace!()})
    })?;

    // activate the device
    let device = activate(
        http_client, &public_key_file, token, device_name,
    ).await?;

    // setup a clean storage layout with the new authentication & device id
    clean_storage_setup(
        layout,
        &Device {
            id: device.id,
            name: device.name,
            session_id: device.session_id,
            agent_version: version_info().version,
            activated: true,
            status: DeviceStatus::Online,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
            last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
        },
        settings,
        &private_key_file,
        &public_key_file,
    ).await.map_err(|e| {
        InstallErr::StorageErr(InstallStorageErr {source: e, trace: trace!()})
    })?;

    // delete the temporary directory
    temp_dir.delete().await.map_err(|e| {
        InstallErr::FileSysErr(InstallFileSysErr {source: e, trace: trace!()})
    })?;

    Ok(())
}

pub async fn activate<HTTPClientT: DevicesExt>(
    http_client: &HTTPClientT,
    public_key_file: &File,
    token: &str,
    device_name: Option<String>,
) -> Result<openapi_client::models::Device, InstallErr> {
    let device_id = jwt::extract_device_id(token).map_err(|e| {
        InstallErr::CryptErr(InstallCryptErr {source: e, trace: trace!()})
    })?;

    // activate the device with the server
    let public_key_pem = public_key_file.read_string().await.map_err(|e| {
        InstallErr::FileSysErr(InstallFileSysErr {source: e, trace: trace!()})
    })?;
    let payload = ActivateDeviceRequest {
        public_key_pem,
        name: device_name,
        agent_version: Some(version_info().version),
    };
    let device = http_client
        .activate_device(&device_id, &payload, token)
        .await
        .map_err(|e| {
            InstallErr::HTTPErr(InstallHTTPErr {source: e, trace: trace!()})
        })?;

    // complete
    display::info(format!(
        "Successfully activated this device as {}!",
        display::color(&device.name, display::Colors::Green)
    ).as_str());

    Ok(device)
}
