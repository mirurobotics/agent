// standard crates
use std::env;

// internal crates
use backend_api::models as backend_client;
use miru_agent::app::run::run;
use miru_agent::app::{
    options::{AppOptions, LifecycleOptions},
    upgrade,
};
use miru_agent::cli;
use miru_agent::filesys::{dir::Dir, path::PathExt};
use miru_agent::http;
use miru_agent::logs;
use miru_agent::mqtt::options::{ConnectAddress, Protocol};
use miru_agent::network::BackendUrl;
use miru_agent::provisioning::{self, display, errors::*, provision, reprovision};
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

    if let Some(reprovision_args) = cli_args.reprovision_args {
        let result = run_reprovision(reprovision_args).await;
        handle_reprovision_result(result);
        return;
    }

    run_agent().await;
}

async fn run_provision(args: cli::ProvisionArgs) -> Result<provision::Outcome, ProvisionErr> {
    // initialize logging
    let tmp_dir = Dir::create_temp_dir("miru-agent-provision-logs").await?;
    let options = logs::Options {
        // sending logs to stdout will interfere with the provision outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let _guard = logs::init(options)?;

    let settings = provision::determine_settings(&args);
    let http_client = http::Client::new(settings.backend.base_url.as_str())?;
    let layout = storage::Layout::default();
    let token = provisioning::read_token_from_env()?;

    let result =
        provision::provision(&http_client, &layout, &settings, &token, args.device_name).await;

    drop(_guard);
    if let Err(e) = tmp_dir.delete().await {
        eprintln!("failed to clean up provision log dir: {e}");
    }

    result
}

fn handle_provision_result(result: Result<provision::Outcome, ProvisionErr>) {
    match result {
        Ok(outcome) if outcome.already_provisioned => {
            let msg = format!(
                "Device is already provisioned as {}!",
                display::color(&outcome.device_name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Ok(outcome) => {
            let msg = format!(
                "Successfully provisioned this device as {}!",
                display::color(&outcome.device_name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Err(e) => {
            error!("Provisioning failed: {:?}", e);
            println!("An error occurred during provisioning.\n\nError: {e}\n");
            std::process::exit(1);
        }
    }
}

async fn run_reprovision(
    args: cli::ReprovisionArgs,
) -> Result<backend_client::Device, ProvisionErr> {
    // initialize logging
    let tmp_dir = Dir::create_temp_dir("miru-agent-reprovision-logs").await?;
    let options = logs::Options {
        // sending logs to stdout will interfere with the reprovision outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let _guard = logs::init(options)?;

    let settings = reprovision::determine_settings(&args);
    let http_client = http::Client::new(settings.backend.base_url.as_str())?;
    let layout = storage::Layout::default();
    let token = provisioning::read_token_from_env()?;

    let result = reprovision::reprovision(&http_client, &layout, &settings, &token).await;

    drop(_guard);
    if let Err(e) = tmp_dir.delete().await {
        eprintln!("failed to clean up reprovision log dir: {e}");
    }

    result
}

fn handle_reprovision_result(result: Result<backend_client::Device, ProvisionErr>) {
    match result {
        Ok(device) => {
            let msg = format!(
                "Successfully reprovisioned this device as {}!",
                display::color(&device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Err(e) => {
            error!("Reprovisioning failed: {:?}", e);
            println!("An error occurred during reprovisioning.\n\nError: {e}\n");
            std::process::exit(1);
        }
    }
}

async fn run_agent() {
    let layout = storage::Layout::default();

    // initialize logging early so reconciliation and pre-settings activity are
    // observable. The level is reloaded once settings are read below.
    let log_guard = match logs::init(logs::Options::default()) {
        Ok(g) => g,
        Err(e) => {
            // tracing is not yet installed if init failed, so use eprintln!
            eprintln!("Failed to initialize logging: {e}");
            return;
        }
    };

    // check the agent has been activated
    if let Err(e) = storage::assert_activated(&layout).await {
        error!("Device is not yet activated: {}", e);
        return;
    }

    // reconcile the agent package version to ensure the file system storage state
    // is compatible with the running version
    let url = get_bootstrap_base_url().await;
    let bootstrap_http_client = match http::Client::new(url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            error!("upgrade: failed to construct http client: {e}");
            return;
        }
    };
    if let Err(e) = upgrade::reconcile(
        &layout,
        &bootstrap_http_client,
        version::VERSION,
        tokio::time::sleep,
    )
    .await
    {
        error!("upgrade: failed to reconcile agent package version: {e}");
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

    // apply the configured log level to the running subscriber
    if let Err(e) = log_guard.reload_level(settings.log_level.clone()) {
        tracing::warn!("Failed to apply settings.log_level to running logger: {e}");
    }

    let broker_address = ConnectAddress::new_or(
        settings.mqtt_broker.host,
        Protocol::SSL,
        8883,
        ConnectAddress::default(),
    );

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
            broker_address,
            ..Default::default()
        },
        ..Default::default()
    };
    info!("Running the server with options: {:?}", options);
    let result = run(options, await_shutdown_signal()).await;
    if let Err(e) = result {
        error!("Failed to run the server: {e}");
    }
}

async fn get_bootstrap_base_url() -> BackendUrl {
    let settings_file = storage::Layout::default().settings();
    if let Ok(settings) = settings_file.read_json::<storage::Settings>().await {
        return settings.backend.base_url;
    }

    storage::Backend::default().base_url
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
