//! The buffer intake and the upload worker share, and its wake-up.

// standard crates
use std::time::Duration;

// internal crates
use crate::test_utils::calibrations::{layout, report};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::buffer::BufferOptions;
use miru_agent::calibrations::{self, CalibrationsErr, SharedBuffer};
use miru_agent::filesys::PathExt;

// external crates
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);

async fn open(dir: &test_dirs::TempDir, options: BufferOptions) -> SharedBuffer {
    SharedBuffer::open(&layout(dir), options).await.unwrap()
}

#[tokio::test]
async fn open_uses_the_calibrations_layout() {
    let dir = test_dirs::temp("calibration_shared_test").unwrap();

    let buffer = open(&dir, calibrations::DEFAULT_OPTIONS).await;

    assert!(layout(&dir).content().exists());
    assert_eq!(
        buffer.failed_dir().to_string(),
        layout(&dir).failed().to_string()
    );
}

#[tokio::test]
async fn open_failure_is_a_buffer_error() {
    let dir = test_dirs::temp("calibration_shared_test").unwrap();
    // a file where the buffer root should be
    std::fs::write(layout(&dir).root.path(), "x").unwrap();

    let err = SharedBuffer::open(&layout(&dir), calibrations::DEFAULT_OPTIONS)
        .await
        .unwrap_err();

    assert!(matches!(err, CalibrationsErr::BufferErr(_)), "{err:?}");
}

#[tokio::test]
async fn enqueue_buffers_and_wakes() {
    let dir = test_dirs::temp("calibration_shared_test").unwrap();
    let buffer = open(&dir, calibrations::DEFAULT_OPTIONS).await;
    let waiter = {
        let buffer = buffer.clone();
        tokio::spawn(async move { buffer.woken().await })
    };

    let id = buffer.enqueue(report("arm")).await.unwrap();

    timeout(WAIT, waiter).await.unwrap().unwrap();
    let guard = buffer.lock().await;
    assert_eq!(guard.len(), 1);
    assert_eq!(guard.read(id).await.unwrap(), report("arm"));
}

#[tokio::test]
async fn a_wake_before_waiting_is_kept() {
    let dir = test_dirs::temp("calibration_shared_test").unwrap();
    let buffer = open(&dir, calibrations::DEFAULT_OPTIONS).await;

    buffer.enqueue(report("arm")).await.unwrap();

    timeout(WAIT, buffer.woken()).await.unwrap();
}

#[tokio::test]
async fn a_full_buffer_is_507_and_does_not_wake() {
    let dir = test_dirs::temp("calibration_shared_test").unwrap();
    let options = BufferOptions {
        max_items: 1,
        ..calibrations::DEFAULT_OPTIONS
    };
    let buffer = open(&dir, options).await;
    buffer.enqueue(report("arm")).await.unwrap();
    // drain the first enqueue's wake
    timeout(WAIT, buffer.woken()).await.unwrap();

    let err = buffer.enqueue(report("leg")).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::BufferFullErr(_)), "{err:?}");
    let woken = timeout(Duration::from_millis(50), buffer.woken()).await;
    assert!(woken.is_err(), "a rejected report woke the worker");
    assert_eq!(buffer.lock().await.len(), 1);
}

#[tokio::test]
async fn debug_names_the_type() {
    let dir = test_dirs::temp("calibration_shared_test").unwrap();
    let buffer = open(&dir, calibrations::DEFAULT_OPTIONS).await;

    let debug = format!("{buffer:?}");

    assert!(debug.starts_with("SharedBuffer { failed: "), "{debug}");
    assert!(debug.ends_with(", .. }"), "{debug}");
}
