// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::app::options::{AppOptions, LifecycleOptions, StorageOptions};
use miru_agent::app::run::run;
use miru_agent::filesys::{dir::Dir, file::File, WriteOptions};
use miru_agent::models::device::Device;
use miru_agent::server::serve::ServerOptions;
use miru_agent::storage::layout::StorageLayout;

// external crates
use tokio::time::Duration;

async fn prepare_valid_server_storage(dir: Dir) {
    let layout = StorageLayout::new(dir);

    // create a private key file
    let private_key_file = layout.auth_dir().private_key_file();
    private_key_file
        .write_string("test", WriteOptions::default())
        .await
        .unwrap();

    // create the device file
    let device_file = layout.device_file();
    let device = Device::default();
    device_file
        .write_json(&device, WriteOptions::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn invalid_app_state_initialization() {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    let options = AppOptions {
        storage: StorageOptions {
            layout: StorageLayout::new(dir),
            ..Default::default()
        },
        ..Default::default()
    };
    tokio::time::timeout(Duration::from_secs(5), async move {
        run(Device::default().agent_version, options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap_err();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn max_runtime_reached() {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: StorageLayout::new(dir),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            max_runtime: Duration::from_millis(100),
            ..Default::default()
        },
        server: ServerOptions {
            socket_file: File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

    // should safely run and shutdown in about 100ms
    tokio::time::timeout(Duration::from_secs(5), async move {
        run(Device::default().agent_version, options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn is_persistent() {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    let max_runtime = Duration::from_millis(100);
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: StorageLayout::new(dir),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: true,
            max_runtime,
            ..Default::default()
        },
        server: ServerOptions {
            socket_file: File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

    tokio::time::timeout(2 * max_runtime, async move {
        run(Device::default().agent_version, options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    // unwrap err because the test should timeout
    .unwrap_err();
}

#[tokio::test]
async fn idle_timeout_reached() {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        storage: StorageOptions {
            layout: StorageLayout::new(dir),
            ..Default::default()
        },
        lifecycle: LifecycleOptions {
            is_persistent: false,
            idle_timeout: Duration::from_millis(100),
            idle_timeout_poll_interval: Duration::from_millis(10),
            ..Default::default()
        },
        server: ServerOptions {
            socket_file: File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

    // should safely run and shutdown in about 100ms
    tokio::time::timeout(Duration::from_secs(5), async move {
        run(Device::default().agent_version, options, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shutdown_signal_received() {
    let dir = Dir::create_temp_dir("testing").await.unwrap();
    prepare_valid_server_storage(dir.clone()).await;
    let options = AppOptions {
        lifecycle: LifecycleOptions {
            is_persistent: true,
            ..Default::default()
        },
        storage: StorageOptions {
            layout: StorageLayout::new(dir),
            ..Default::default()
        },
        server: ServerOptions {
            socket_file: File::new(PathBuf::from("/tmp").join("miru.sock")),
        },
        ..Default::default()
    };

    // Create a channel for manual shutdown
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Spawn the server in a task
    let server_handle = tokio::spawn(async move {
        run(Device::default().agent_version, options, async {
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
