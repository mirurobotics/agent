// standard library
use std::collections::HashMap;
use std::env;

// internal
use miru_agent::app::options::{AppOptions, LifecycleOptions};
use miru_agent::app::run::run;
use miru_agent::installer::install::install;
use miru_agent::logs::{init, LogOptions};
use miru_agent::mqtt::client::ConnectAddress;
use miru_agent::storage::device::assert_activated;
use miru_agent::storage::layout::StorageLayout;
use miru_agent::storage::settings::Settings;
use miru_agent::version;
use miru_agent::workers::mqtt;

// external
use tokio::signal::unix::signal;
use tracing::{error, info};

#[tokio::main]
async fn main() {
    // parse the command line arguments
    let args: Vec<String> = env::args().collect();
    let mut cli_args: HashMap<String, String> = HashMap::new();
    for arg in args.iter().skip(1) {
        if let Some((key, value)) = arg.split_once('=') {
            // Handle --key=value format
            let clean_key = key.trim_start_matches('-');
            cli_args.insert(clean_key.to_string(), value.to_string());
        } else if arg.starts_with("--") {
            // Handle standalone flags like --version
            let clean_key = arg.trim_start_matches('-');
            cli_args.insert(clean_key.to_string(), "true".to_string());
        }
    }

    // print the version & exit
    let version_info = version::build_info();
    if cli_args.contains_key("version") {
        println!("{:?}", version_info);
        return;
    }

    // run the installer
    if cli_args.contains_key("install") {
        return install(&cli_args).await;
    }

    // run the agent starting here

    // check the agent has been activated
    let layout = StorageLayout::default();
    let device_file = layout.device_file();
    if let Err(e) = assert_activated(&device_file).await {
        error!("Device is not yet activated: {}", e);
        return;
    }

    // retrieve the settings files
    let settings_file = layout.settings_file();
    let settings = match settings_file.read_json::<Settings>().await {
        Ok(settings) => settings,
        Err(e) => {
            error!("Unable to read settings file: {}", e);
            return;
        }
    };

    // initialize the logging
    let log_options = LogOptions {
        log_level: settings.log_level,
        ..Default::default()
    };
    let result = init(log_options);
    if let Err(e) = result {
        println!("Failed to initialize logging: {e}");
    }

    // run the server
    let options = AppOptions {
        lifecycle: LifecycleOptions {
            is_persistent: settings.is_persistent,
            ..Default::default()
        },
        backend_base_url: settings.backend.base_url,
        enable_socket_server: settings.enable_socket_server,
        enable_mqtt_worker: settings.enable_mqtt_worker,
        enable_poller: settings.enable_poller,
        mqtt_worker: mqtt::Options {
            broker_address: ConnectAddress {
                broker: settings.mqtt_broker.host,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    info!("Running the server with options: {:?}", options);
    let build_info = version::build_info();
    let result = run(build_info.version, options, await_shutdown_signal()).await;
    if let Err(e) = result {
        error!("Failed to run the server: {e}");
    }
}

async fn await_shutdown_signal() {
    let mut sigterm = signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    let mut sigint = signal(tokio::signal::unix::SignalKind::interrupt()).unwrap();

    tokio::select! {
        _ = sigterm.recv() => {
            info!("SIGTERM received, shutting down...");
        }
        _ = sigint.recv() => {
            info!("SIGINT received, shutting down...");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("received ctrl-c, shutting down...");
        }
    }
}
