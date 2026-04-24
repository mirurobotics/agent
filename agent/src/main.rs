// standard crates
use std::env;

// internal crates
use backend_api::models as backend_client;
use miru_agent::app::options::{AppOptions, LifecycleOptions};
use miru_agent::app::run::run;
use miru_agent::cli;
use miru_agent::filesys::{dir::Dir, path::PathExt};
use miru_agent::http;
use miru_agent::logs;
use miru_agent::mqtt::options::ConnectAddress;
use miru_agent::provision::{display, entry, errors::*};
use miru_agent::storage;
use miru_agent::version;
use miru_agent::workers::mqtt;

// external crates
use tokio::signal::unix::signal;
use tracing::{error, info};

#[tokio::main]
async fn main() {
    let cli_args = cli::Args::parse(&env::args().collect::<Vec<String>>());

    if cli_args.display_version {
        println!("{}", version::format());
        return;
    }

    if let Some(provision_args) = cli_args.provision_args {
        let result = run_provision(provision_args).await;
        handle_provision_result(result);
        return;
    }

    run_agent().await;
}

async fn run_provision(args: cli::ProvisionArgs) -> Result<backend_client::Device, ProvisionErr> {
    // initialize logging
    let tmp_dir = Dir::create_temp_dir("miru-agent-provision-logs").await?;
    let options = logs::Options {
        // sending logs to stdout will interfere with the provision outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let _guard = logs::init(options);

    let settings = entry::determine_settings(&args);
    let http_client = http::Client::new(&settings.backend.base_url)?;
    let layout = storage::Layout::default();
    let token = entry::read_token_from_env()?;

    let result = entry::provision(&http_client, &layout, &settings, &token, args.device_name).await;

    drop(_guard);
    if let Err(e) = tmp_dir.delete().await {
        eprintln!("failed to clean up provision log dir: {e}");
    }

    result
}

fn handle_provision_result(result: Result<backend_client::Device, ProvisionErr>) {
    match result {
        Ok(device) => {
            let msg = format!(
                "Successfully provisioned this device as {}!",
                display::color(&device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Err(e) => {
            error!("Provisioning failed: {:?}", e);
            println!("An error occurred during provisioning. Contact us at ben@mirurobotics.com for immediate support.\n\nError: {e}\n");
            std::process::exit(1);
        }
    }
}

async fn run_agent() {
    // check the agent has been activated
    let layout = storage::Layout::default();
    let device_file = layout.device();
    if let Err(e) = storage::assert_activated(&device_file).await {
        error!("Device is not yet activated: {}", e);
        return;
    }

    // retrieve the settings files
    let settings_file = layout.settings();
    let settings = match settings_file.read_json::<storage::Settings>().await {
        Ok(settings) => settings,
        Err(e) => {
            error!("Unable to read settings file: {}", e);
            return;
        }
    };

    // initialize the logging
    let log_options = logs::Options {
        log_level: settings.log_level,
        ..Default::default()
    };
    let _guard = logs::init(log_options);

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
    let result = run(
        version::VERSION.to_string(),
        options,
        await_shutdown_signal(),
    )
    .await;
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
