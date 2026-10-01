// standard crates
use std::path::PathBuf;

// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use device_api::models::ApiVersion;
use miru_agent::app::options::{AppOptions, LifecycleOptions, StorageOptions};
use miru_agent::app::run::run;
use miru_agent::disk::Layout;
use miru_agent::filesys::{self, files, PathExt, WriteOptions};
use miru_agent::models::Device;
use miru_agent::server::{tcp, Options, ServerErr};

// external crates
use serde_json::Value;
use serial_test::serial;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::Duration;

// Outer wall-clock net around run() in each test. Purely hang
// protection -- its value is NOT part of the verified behavior. It
// must absorb coverage-instrumented, loaded-machine runs, so keep it
// generous; on success it never elapses and costs nothing.
const HANG_GUARD: Duration = Duration::from_secs(60);

// ShutdownManager::shutdown calls std::process::exit(1) if teardown
// exceeds max_shutdown_delay, which would kill the whole test binary.
// Keep it above HANG_GUARD so a hung shutdown fails only the
// offending test via the outer timeout instead.
const SHUTDOWN_WATCHDOG: Duration = Duration::from_secs(300);

// ================================== HELPERS ====================================== //

/// A layout holding the keys and device file the agent needs to start. The
/// layout is valid only while the returned `TempDir` lives.
async fn activated_layout() -> (test_dirs::TempDir, Layout) {
    let dir = test_dirs::temp("testing").unwrap();
    let layout = Layout::new(dir.to_dir());
    let opts = WriteOptions::default();
    files::write_string(&layout.auth().private_key(), "test", opts)
        .await
        .unwrap();
    files::write_string(&layout.auth().public_key(), "test", opts)
        .await
        .unwrap();
    files::write_json(&layout.device(), &Device::default(), opts)
        .await
        .unwrap();
    (dir, layout)
}

/// Options for `layout` with the socket under /tmp. `tcp_port` enables the
/// TCP server on that port.
fn options(layout: &Layout, lifecycle: LifecycleOptions, tcp_port: Option<u16>) -> AppOptions {
    let mut server = Options {
        socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
        ..Default::default()
    };
    if let Some(port) = tcp_port {
        server.tcp_port = port;
    }
    AppOptions {
        storage: StorageOptions {
            layout: layout.clone(),
            ..Default::default()
        },
        lifecycle,
        enable_tcp_server: tcp_port.is_some(),
        server,
        ..Default::default()
    }
}

/// Lifecycle options whose shutdown watchdog outlasts HANG_GUARD.
fn lifecycle() -> LifecycleOptions {
    LifecycleOptions {
        max_shutdown_delay: SHUTDOWN_WATCHDOG,
    }
}

/// Run the agent until it exits on its own; ctrl-c is the only external
/// shutdown signal. HANG_GUARD turns a hang into a test failure.
async fn run_to_exit(options: AppOptions) -> Result<(), ServerErr> {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    tokio::time::timeout(HANG_GUARD, run(options, ctrl_c))
        .await
        .expect("run did not exit within HANG_GUARD")
}

/// An agent run in a background task that the test shuts down.
struct RunningAgent {
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<Result<(), ServerErr>>,
}

fn spawn_run(options: AppOptions) -> RunningAgent {
    let (shutdown, rx) = oneshot::channel();
    let handle = tokio::spawn(run(options, async move {
        let _ = rx.await;
    }));
    RunningAgent { shutdown, handle }
}

impl RunningAgent {
    /// Send the shutdown signal and wait for the run to finish.
    /// run() polls the signal only after init() finishes and the oneshot
    /// buffers it, so Ok means startup completed.
    async fn stop(self) -> Result<(), ServerErr> {
        let _ = self.shutdown.send(());
        tokio::time::timeout(HANG_GUARD, self.handle)
            .await
            .expect("run did not stop within HANG_GUARD")
            .expect("run task panicked")
    }
}

async fn write_stale_discovery_file(layout: &Layout) {
    files::write_string(&layout.device_api(), "stale", WriteOptions::default())
        .await
        .unwrap();
}

async fn wait_for_file(file: &filesys::File) {
    let poll = async {
        while !file.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), poll)
        .await
        .expect("file not written within 10s");
}

fn no_proxy_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

// =================================== TESTS ======================================= //

#[tokio::test]
async fn invalid_app_state_initialization() {
    let dir = test_dirs::temp("testing").unwrap();
    let layout = Layout::new(dir.to_dir());
    write_stale_discovery_file(&layout).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: layout.clone(),
            ..Default::default()
        },
        ..Default::default()
    };

    run_to_exit(options).await.unwrap_err();

    // the stale file goes before the failing init step
    assert!(!layout.device_api().exists());
}

#[serial]
#[tokio::test]
async fn tcp_port_in_use_does_not_abort_startup() {
    let (_dir, layout) = activated_layout().await;
    write_stale_discovery_file(&layout).await;
    let (_taken, port) = tcp::bind(0).await.unwrap();

    // the bind fails and the agent keeps running
    let agent = spawn_run(options(&layout, lifecycle(), Some(port)));
    agent.stop().await.unwrap();

    // no server wrote a fresh file, so the stale one stays removed
    assert!(!layout.device_api().exists());
}

#[serial]
#[tokio::test]
async fn discovery_write_failure_does_not_abort_startup() {
    let (_dir, layout) = activated_layout().await;
    // a file where the discovery directory should be makes the write fail
    let parent = layout.device_api().parent().unwrap();
    files::write_string(
        &filesys::File::new(parent.path().clone()),
        "x",
        WriteOptions::default(),
    )
    .await
    .unwrap();

    // the tcp server fails to start and the agent keeps running
    let agent = spawn_run(options(&layout, lifecycle(), Some(0)));
    agent.stop().await.unwrap();
}

#[serial]
#[tokio::test]
async fn tcp_requires_bearer_and_cleans_up_discovery_file() {
    let (_dir, layout) = activated_layout().await;
    let discovery_file = layout.device_api();
    let agent = spawn_run(options(&layout, lifecycle(), Some(0)));

    // the discovery file appears once the tcp server is serving
    wait_for_file(&discovery_file).await;
    let discovery: Value = files::read_json(&discovery_file).await.unwrap();
    let port = discovery["port"].as_u64().unwrap();
    let token = discovery["token"].as_str().unwrap();
    let url = format!("http://127.0.0.1:{port}/{}/health", ApiVersion::API_VERSION);

    let client = no_proxy_client();
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let response = client.get(&url).bearer_auth(token).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    drop(client);

    agent.stop().await.unwrap();
    assert!(!discovery_file.exists());
}

#[serial]
#[tokio::test]
async fn stale_discovery_file_removed_when_tcp_disabled() {
    let (_dir, layout) = activated_layout().await;
    write_stale_discovery_file(&layout).await;

    let agent = spawn_run(options(&layout, lifecycle(), None));
    agent.stop().await.unwrap();

    assert!(!layout.device_api().exists());
}

#[serial]
#[tokio::test]
async fn shutdown_signal_received() {
    let (_dir, layout) = activated_layout().await;
    let agent = spawn_run(options(&layout, lifecycle(), None));

    // Best-effort wait for the agent to start. The oneshot channel buffers
    // the signal, so the test stays correct even if startup takes longer.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !agent.handle.is_finished(),
        "run exited before the shutdown signal"
    );

    agent.stop().await.unwrap();
}
