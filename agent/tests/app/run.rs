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
use miru_agent::server::{tcp, Options};

// external crates
use serde_json::Value;
use serial_test::serial;
use tokio::time::Duration;

// Outer wall-clock net around run() in each test. Purely hang
// protection -- its value is NOT part of the verified behavior. It
// must absorb coverage-instrumented, loaded-machine runs, so keep it
// generous; on success it never elapses and costs nothing.
const HANG_GUARD: Duration = Duration::from_secs(60);

// Pins a competing lifecycle exit path so far away it cannot fire
// within HANG_GUARD, making each test's intended exit path
// unambiguous.
const NEVER: Duration = Duration::from_secs(3600);

// ShutdownManager::shutdown calls std::process::exit(1) if teardown
// exceeds max_shutdown_delay, which would kill the whole test binary.
// Keep it above HANG_GUARD so a hung shutdown fails only the
// offending test via the outer timeout instead.
const SHUTDOWN_WATCHDOG: Duration = Duration::from_secs(300);

fn no_proxy_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn write_stale_discovery_file(layout: &Layout) {
    files::write_string(&layout.device_api(), "stale", WriteOptions::default())
        .await
        .unwrap();
}

async fn prepare_valid_server_storage(dir: filesys::Dir) {
    let layout = Layout::new(dir);

    // create a private key file
    let private_key_file = layout.auth().private_key();
    files::write_string(&private_key_file, "test", WriteOptions::default())
        .await
        .unwrap();

    // create a public key file
    let public_key_file = layout.auth().public_key();
    files::write_string(&public_key_file, "test", WriteOptions::default())
        .await
        .unwrap();

    // create the device file
    let device_file = layout.device();
    let device = Device::default();
    files::write_json(&device_file, &device, WriteOptions::default())
        .await
        .unwrap();
}

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
    tokio::time::timeout(HANG_GUARD, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap_err();
    })
    .await
    .unwrap();

    // the stale file goes before the failing init step
    assert!(!layout.device_api().exists());
}

#[serial]
#[tokio::test]
async fn max_runtime_reached() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir.to_dir()),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            max_runtime: Duration::from_millis(100),
            idle_timeout: NEVER,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
            ..Default::default()
        },
        enable_tcp_server: true,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            tcp_port: 0,
        },
        ..Default::default()
    };

    // the run self-terminates via max_runtime (~100ms); the outer
    // timeout is only hang protection
    tokio::time::timeout(HANG_GUARD, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
}

#[serial]
#[tokio::test]
async fn tcp_port_in_use_does_not_abort_startup() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let layout = Layout::new(dir.to_dir());
    write_stale_discovery_file(&layout).await;
    let (_taken, port) = tcp::bind(0).await.unwrap();
    let options = AppOptions {
        storage: StorageOptions {
            layout: layout.clone(),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            max_runtime: Duration::from_millis(100),
            idle_timeout: NEVER,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
            ..Default::default()
        },
        enable_tcp_server: true,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            tcp_port: port,
        },
        ..Default::default()
    };

    // the bind fails, the agent keeps running, and max_runtime (~100ms)
    // ends the run cleanly
    tokio::time::timeout(HANG_GUARD, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();

    // no server wrote a fresh file, so the stale one stays removed
    assert!(!layout.device_api().exists());
}

#[serial]
#[tokio::test]
async fn discovery_write_failure_does_not_abort_startup() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let layout = Layout::new(dir.to_dir());
    // a file where the discovery directory should be makes the write fail
    let parent = layout.device_api().parent().unwrap();
    files::write_string(
        &filesys::File::new(parent.path().clone()),
        "x",
        WriteOptions::default(),
    )
    .await
    .unwrap();
    let options = AppOptions {
        storage: StorageOptions {
            layout,
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            max_runtime: Duration::from_millis(100),
            idle_timeout: NEVER,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
            ..Default::default()
        },
        enable_tcp_server: true,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            tcp_port: 0,
        },
        ..Default::default()
    };

    // serve fails, the agent keeps running, and max_runtime (~100ms) ends
    // the run cleanly
    tokio::time::timeout(HANG_GUARD, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
}

#[serial]
#[tokio::test]
async fn tcp_requires_bearer_and_cleans_up_discovery_file() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let layout = Layout::new(dir.to_dir());
    let discovery_file = layout.device_api();
    let options = AppOptions {
        storage: StorageOptions {
            layout,
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: true,
            idle_timeout: NEVER,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
            ..Default::default()
        },
        enable_tcp_server: true,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            tcp_port: 0,
        },
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        run(options, async {
            let _ = rx.await;
        })
        .await
    });

    // the discovery file appears once the tcp server is serving
    let mut waited = Duration::ZERO;
    while !discovery_file.exists() {
        assert!(
            waited < Duration::from_secs(10),
            "discovery file not written within 10s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += Duration::from_millis(20);
    }
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

    tx.send(()).unwrap();
    tokio::time::timeout(HANG_GUARD, server_handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!discovery_file.exists());
}

#[serial]
#[tokio::test]
async fn stale_discovery_file_removed_when_tcp_disabled() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let layout = Layout::new(dir.to_dir());
    write_stale_discovery_file(&layout).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: layout.clone(),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            max_runtime: Duration::from_millis(100),
            idle_timeout: NEVER,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
            ..Default::default()
        },
        enable_tcp_server: false,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            ..Default::default()
        },
        ..Default::default()
    };

    tokio::time::timeout(HANG_GUARD, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();

    assert!(!layout.device_api().exists());
}

#[serial]
#[tokio::test]
async fn is_persistent() {
    let dir = test_dirs::temp("testing").unwrap();
    let max_runtime = Duration::from_millis(100);
    prepare_valid_server_storage(dir.to_dir()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir.to_dir()),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: true,
            max_runtime,
            ..Default::default()
        },
        enable_tcp_server: false,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            ..Default::default()
        },
        ..Default::default()
    };

    // negative assertion: the timeout MUST elapse because persistent
    // mode ignores max_runtime, so machine slowdown can only reinforce
    // the expected outcome -- the short window is intentional
    tokio::time::timeout(2 * max_runtime, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    // unwrap err because the test should timeout
    .unwrap_err();
}

#[serial]
#[tokio::test]
async fn idle_timeout_reached() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir.to_dir()),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            idle_timeout: Duration::from_millis(100),
            idle_timeout_poll_interval: Duration::from_millis(10),
            max_runtime: NEVER,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
        },
        enable_tcp_server: false,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            ..Default::default()
        },
        ..Default::default()
    };

    // the run self-terminates via idle_timeout (~100ms); the outer
    // timeout is only hang protection
    tokio::time::timeout(HANG_GUARD, async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
}

#[serial]
#[tokio::test]
async fn shutdown_signal_received() {
    let dir = test_dirs::temp("testing").unwrap();
    prepare_valid_server_storage(dir.to_dir()).await;
    let options = AppOptions {
        lifecycle: LifecycleOptions {
            is_persistent: true,
            max_shutdown_delay: SHUTDOWN_WATCHDOG,
            ..Default::default()
        },
        storage: StorageOptions {
            layout: Layout::new(dir.to_dir()),
            ..Default::default()
        },
        enable_tcp_server: false,
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
            ..Default::default()
        },
        ..Default::default()
    };

    // Create a channel for manual shutdown
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Spawn the server in a task
    let server_handle = tokio::spawn(async move {
        run(options, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    // Small delay to ensure server is running. Best-effort only: the
    // oneshot channel buffers the signal, so the test stays correct
    // even if startup takes longer than 100ms.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send shutdown signal
    tx.send(()).unwrap();

    // Wait for server to shutdown with timeout
    tokio::time::timeout(HANG_GUARD, server_handle)
        .await
        .unwrap()
        .unwrap();
}
