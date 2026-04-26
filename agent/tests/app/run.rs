// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::app::options::{AppOptions, LifecycleOptions, StorageOptions};
use miru_agent::app::run::run;
use miru_agent::filesys::{self, WriteOptions};
use miru_agent::models::Device;
use miru_agent::server::Options;
use miru_agent::storage::Layout;

// external crates
use serial_test::serial;
use tokio::time::Duration;

async fn prepare_valid_server_storage(dir: filesys::Dir) {
    let layout = Layout::new(dir);

    // create a private key file
    let private_key_file = layout.auth().private_key();
    private_key_file
        .write_string("test", WriteOptions::default())
        .await
        .unwrap();

    // create the device file
    let device_file = layout.device();
    let device = Device::default();
    device_file
        .write_json(&device, WriteOptions::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn invalid_app_state_initialization() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir),
            ..Default::default()
        },
        ..Default::default()
    };
    tokio::time::timeout(Duration::from_secs(5), async move {
        run(options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap_err();
    })
    .await
    .unwrap();
}

#[serial]
#[tokio::test]
async fn max_runtime_reached() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            max_runtime: Duration::from_millis(100),
            ..Default::default()
        },
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

    // should safely run and shutdown in about 100ms
    tokio::time::timeout(Duration::from_secs(5), async move {
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
async fn is_persistent() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let max_runtime = Duration::from_millis(100);
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: true,
            max_runtime,
            ..Default::default()
        },
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

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
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: Layout::new(dir),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            idle_timeout: Duration::from_millis(100),
            idle_timeout_poll_interval: Duration::from_millis(10),
            max_shutdown_delay: Duration::from_secs(5),
            ..Default::default()
        },
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

    // idle timeout triggers after ~100ms; shutdown may take up to max_shutdown_delay (5s)
    tokio::time::timeout(Duration::from_secs(15), async move {
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
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        lifecycle: LifecycleOptions {
            is_persistent: true,
            ..Default::default()
        },
        storage: StorageOptions {
            layout: Layout::new(dir),
            ..Default::default()
        },
        server: Options {
            socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock")),
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

    // Small delay to ensure server is running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send shutdown signal
    tx.send(()).unwrap();

    // Wait for server to shutdown with timeout
    tokio::time::timeout(Duration::from_secs(5), server_handle)
        .await
        .unwrap()
        .unwrap();
}
