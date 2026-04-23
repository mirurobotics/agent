// standard crates
use std::env;

// internal crates
use backend_api::models as backend_client;
use miru_agent::app::options::{AppOptions, LifecycleOptions};
use miru_agent::app::run::run;
use miru_agent::cli;
use miru_agent::http;
use miru_agent::installer::{
    self, display,
    errors::*,
    install,
    provision::{self, ProvisionErr},
};
use miru_agent::logs;
use miru_agent::mqtt::options::ConnectAddress;
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

    if let Some(install_args) = cli_args.install_args {
        let result = run_installer(install_args).await;
        handle_install_result(result);
        return;
    }

    if let Some(provision_args) = cli_args.provision_args {
        let exit_code = run_provision(provision_args).await;
        std::process::exit(exit_code);
    }

    run_agent().await;
}

async fn run_provision(args: cli::ProvisionArgs) -> i32 {
    // Privilege check is the very first action — before logging init, env
    // reads, or arg validation — so non-root callers fail fast with a clear
    // message and zero side effects.
    if provision::assert_root().is_err() {
        eprintln!("miru-agent provision must be run as root (sudo -E)");
        return cli::exit_codes::GENERIC_FAILURE;
    }

    let (_guard, tmp_dir) = match installer::init_installer_logging().await {
        Ok(pair) => pair,
        Err(_) => return cli::exit_codes::GENERIC_FAILURE,
    };

    let api_key = match provision::read_api_key_from_env() {
        Ok(k) => k,
        Err(_) => {
            eprintln!("MIRU_API_KEY environment variable is not set");
            return cli::exit_codes::MISSING_API_KEY;
        }
    };

    let device_name = match args.device_name.as_deref() {
        Some(n) => n,
        None => {
            eprintln!("--device-name is required");
            return cli::exit_codes::GENERIC_FAILURE;
        }
    };

    let settings = install::determine_settings_from(
        args.backend_host.as_deref(),
        args.mqtt_broker_host.as_deref(),
    );

    let backend_host = args
        .backend_host
        .as_deref()
        .unwrap_or(install::DEFAULT_BACKEND_HOST);
    let agent_http_client = match http::Client::new(&settings.backend.base_url) {
        Ok(c) => c,
        Err(_) => return cli::exit_codes::GENERIC_FAILURE,
    };
    let public_api_http_client = match http::Client::new(&format!("{}/v1", backend_host)) {
        Ok(c) => c,
        Err(_) => return cli::exit_codes::GENERIC_FAILURE,
    };

    let layout = storage::Layout::default();
    let systemctl = provision::RealSystemctl;
    let result = provision::provision(
        &public_api_http_client,
        &agent_http_client,
        &systemctl,
        &layout,
        &settings,
        &api_key,
        device_name,
        args.allow_reactivation.unwrap_or(false),
    )
    .await;

    drop(_guard);
    if let Err(e) = tmp_dir.delete().await {
        eprintln!("failed to clean up provision log dir: {e}");
    }

    handle_provision_result(result)
}

async fn run_installer(args: cli::InstallArgs) -> Result<backend_client::Device, InstallErr> {
    let (_guard, tmp_dir) = installer::init_installer_logging().await?;

    let settings = install::determine_settings(&args);
    let http_client = http::Client::new(&settings.backend.base_url)?;
    let layout = storage::Layout::default();
    let token = install::read_token_from_env()?;

    let result = install::install(&http_client, &layout, &settings, &token, args.device_name).await;

    drop(_guard);
    if let Err(e) = tmp_dir.delete().await {
        eprintln!("failed to clean up installer log dir: {e}");
    }

    result
}

fn handle_install_result(result: Result<backend_client::Device, InstallErr>) {
    match result {
        Ok(device) => {
            let msg = format!(
                "Successfully activated this device as {}!",
                display::color(&device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Err(e) => {
            error!("Installation failed: {:?}", e);
            println!("An error occurred during your installation. Contact us at ben@mirurobotics.com for immediate support.\n\nError: {e}\n");
            std::process::exit(1);
        }
    }
}

fn handle_provision_result(result: Result<backend_client::Device, ProvisionErr>) -> i32 {
    match result {
        Ok(device) => {
            let msg = format!(
                "Successfully activated this device as {}!",
                display::color(&device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
            cli::exit_codes::SUCCESS
        }
        Err(e) => {
            error!("Provision failed: {:?}", e);
            println!("An error occurred during provisioning. Contact us at ben@mirurobotics.com for immediate support.\n\nError: {e}\n");
            match e {
                ProvisionErr::MissingApiKeyErr(_) => cli::exit_codes::MISSING_API_KEY,
                ProvisionErr::BackendErr(_) => cli::exit_codes::BACKEND_ERROR,
                ProvisionErr::ReactivationNotAllowedErr(_) => {
                    cli::exit_codes::REACTIVATION_NOT_ALLOWED
                }
                ProvisionErr::InstallErr(_) => cli::exit_codes::INSTALL_FAILURE,
                ProvisionErr::NotRootErr(_) => cli::exit_codes::GENERIC_FAILURE,
                ProvisionErr::SystemdErr(_) => cli::exit_codes::INSTALL_FAILURE,
            }
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
