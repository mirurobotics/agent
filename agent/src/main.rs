// standard crates
use std::env;

// internal crates
use backend_api::models as backend_client;
use miru_agent::app::await_activation::{await_activation, Outcome};
use miru_agent::app::run::run;
use miru_agent::app::{
    options::{AppOptions, LifecycleOptions},
    upgrade,
};
use miru_agent::cli;
use miru_agent::disk;
use miru_agent::filesys::{dirs, files, path::PathExt};
use miru_agent::http;
use miru_agent::logs;
use miru_agent::mqtt::options::{ConnectAddress, Protocol};
use miru_agent::network::BackendHost;
use miru_agent::platform;
use miru_agent::privilege;
use miru_agent::provisioning::{self, check, display, errors::*, provision, reprovision};
use miru_agent::shutdown::{Latch, RunOutcome};
use miru_agent::version;
#[cfg(windows)]
use miru_agent::windows;
use miru_agent::workers::mqtt;

// external crates
#[cfg(unix)]
use tokio::signal::unix::signal;
use tracing::{error, info};

fn main() {
    let cli_args = cli::Args::parse(&env::args().collect::<Vec<String>>());

    if cli_args.display_version {
        println!("{}", version::format());
        return;
    }

    if let Err(e) = privilege::verify_effective_user("miru") {
        eprintln!("miru-agent: {e}");
        std::process::exit(1);
    }

    if let Some(provision_args) = cli_args.provision_args {
        if provision_args.check {
            let report = check::check(&disk::Layout::default());
            if let Some(line) = report.stdout_line() {
                println!("{line}");
            }
            if let Some(line) = report.stderr_line() {
                eprintln!("{line}");
            }
            std::process::exit(report.exit_code());
        }

        handle_provision_result(runtime().block_on(run_provision(provision_args)));
        return;
    }

    if let Some(reprovision_args) = cli_args.reprovision_args {
        handle_reprovision_result(runtime().block_on(run_reprovision(reprovision_args)));
        return;
    }

    launch_agent(cli_args.console);
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
}

async fn run_provision(args: cli::ProvisionArgs) -> Result<provision::Outcome, ProvisionErr> {
    // initialize logging
    let tmp_dir = dirs::create_temp("miru-agent-provision-logs").await?;
    let options = logs::Options {
        // sending logs to stdout will interfere with the provision outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let _guard = logs::init(options)?;

    let settings = provision::determine_settings(&args);
    let http_client = http::Client::new(&settings.backend.host.as_url())?;
    let layout = disk::Layout::default();
    let token = provisioning::read_token_from_env()?;

    let result =
        provision::provision(&http_client, &layout, &settings, &token, args.device_name).await;

    drop(_guard);
    if let Err(e) = dirs::delete(&tmp_dir).await {
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
    let tmp_dir = dirs::create_temp("miru-agent-reprovision-logs").await?;
    let options = logs::Options {
        // sending logs to stdout will interfere with the reprovision outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let _guard = logs::init(options)?;

    let settings = reprovision::determine_settings(&args);
    let http_client = http::Client::new(&settings.backend.host.as_url())?;
    let layout = disk::Layout::default();
    let token = provisioning::read_token_from_env()?;

    let result = reprovision::reprovision(&http_client, &layout, &settings, &token).await;

    drop(_guard);
    if let Err(e) = dirs::delete(&tmp_dir).await {
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

/// Starts the long-running agent: the Windows service unless `--console`,
/// otherwise the foreground process.
fn launch_agent(console: bool) {
    #[cfg(windows)]
    if !console {
        run_agent_as_windows_service();
        return;
    }
    let _ = console; // unix: the flag is a no-op (keeps clippy -D warnings quiet)
    runtime().block_on(run_agent_in_foreground());
}

/// Foreground path: trip a shared [`Latch`] from OS signals so every startup
/// and runtime phase, including upgrade reconcile, observes the same stop.
async fn run_agent_in_foreground() -> RunOutcome {
    let latch = Latch::new();
    let relay = latch.clone();
    tokio::spawn(async move {
        await_shutdown_signal().await;
        relay.trigger();
    });
    run_agent(logs::Options::default(), latch).await
}

/// Hands the process to the SCM. Exits 1 if this process was not started by
/// the service manager (use `--console` for foreground).
#[cfg(windows)]
fn run_agent_as_windows_service() {
    if let Err(e) = windows::scm::dispatch(windows_service_body) {
        eprintln!("miru-agent: {e}");
        std::process::exit(1);
    }
}

/// Service entry point: runs on the SCM's service thread with its own runtime
/// and logs to the rolling file only, since a service has no console.
#[cfg(windows)]
fn windows_service_body(latch: Latch) -> RunOutcome {
    let options = logs::Options {
        stdout: false,
        ..Default::default()
    };
    runtime().block_on(run_agent(options, latch))
}

/// Runs the agent to completion. `latch` is observed at every phase, including
/// upgrade reconcile (between attempts and during backoff, never mid-reset).
async fn run_agent(log_options: logs::Options, latch: Latch) -> RunOutcome {
    let layout = disk::Layout::default();

    // initialize logging early so reconciliation and pre-settings activity are
    // observable. The level is reloaded once settings are read below.
    let log_guard = match logs::init(log_options) {
        Ok(g) => g,
        Err(e) => {
            // tracing is not yet installed if init failed, so use eprintln!
            eprintln!("Failed to initialize logging: {e}");
            return RunOutcome::Failed;
        }
    };

    // wait for the device to be activated (or a shutdown signal)
    match await_activation(&layout, tokio::time::sleep, latch.wait()).await {
        Outcome::Activated => {}
        Outcome::ShutdownRequested => return RunOutcome::Completed,
    }

    if let Some(outcome) = reconcile_agent_version(&layout, &latch).await {
        return outcome;
    }

    // retrieve the settings files
    let Some(settings) = read_settings(&layout).await else {
        return RunOutcome::Failed;
    };

    // apply the configured log level to the running subscriber
    if let Err(e) = log_guard.reload_level(settings.log_level.clone()) {
        tracing::warn!("Failed to apply settings.log_level to running logger: {e}");
    }

    // run the server
    let options = build_app_options(settings);
    info!("Running the server with options: {:?}", options);
    match run(options, latch.wait()).await {
        Ok(()) => RunOutcome::Completed,
        Err(e) => {
            error!("Failed to run the server: {e}");
            RunOutcome::Failed
        }
    }
}

/// Reconcile on-disk state with the running version. `Some(outcome)` means
/// `run_agent` should return that outcome (stop or failure).
async fn reconcile_agent_version(layout: &disk::Layout, latch: &Latch) -> Option<RunOutcome> {
    let client = match http::Client::new(&get_bootstrap_backend_host().await.as_url()) {
        Ok(c) => c,
        Err(e) => {
            error!("upgrade: failed to construct http client: {e}");
            return Some(RunOutcome::Failed);
        }
    };
    match upgrade::reconcile(
        layout,
        &client,
        version::VERSION,
        tokio::time::sleep,
        latch.wait(),
    )
    .await
    {
        Ok(Some(_)) => None,
        Ok(None) => Some(RunOutcome::Completed),
        Err(e) => {
            error!("upgrade: failed to reconcile agent package version: {e}");
            Some(RunOutcome::Failed)
        }
    }
}

async fn read_settings(layout: &disk::Layout) -> Option<disk::Settings> {
    let settings_file = layout.settings();
    match files::read_json::<disk::Settings>(&settings_file).await {
        Ok(settings) => Some(settings),
        Err(e) => {
            error!("Unable to read settings file: {}", e);
            None
        }
    }
}

fn build_app_options(settings: disk::Settings) -> AppOptions {
    let broker_address = ConnectAddress::new_or(
        settings.mqtt_broker.host,
        Protocol::SSL,
        443,
        ConnectAddress::default(),
    );

    let is_persistent = LifecycleOptions::resolve_persistence(
        settings.is_persistent,
        platform::supports_idle_exit(),
    );
    AppOptions {
        lifecycle: LifecycleOptions {
            is_persistent,
            ..Default::default()
        },
        backend_host: settings.backend.host,
        enable_socket_server: settings.enable_socket_server,
        enable_mqtt_worker: settings.enable_mqtt_worker,
        enable_poller: settings.enable_poller,
        mqtt_worker: mqtt::Options {
            broker_address,
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn get_bootstrap_backend_host() -> BackendHost {
    let settings_file = disk::Layout::default().settings();
    if let Ok(settings) = files::read_json::<disk::Settings>(&settings_file).await {
        return settings.backend.host;
    }

    disk::Backend::default().host
}

#[cfg(windows)]
async fn await_shutdown_signal() {
    // Foreground only; service mode trips `Latch` from the SCM handler.
    let _ = tokio::signal::ctrl_c().await;
    info!("received ctrl-c, shutting down...");
}

#[cfg(unix)]
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
