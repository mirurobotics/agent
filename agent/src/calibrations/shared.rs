// standard crates
use std::fmt;
use std::sync::Arc;

// internal crates
use crate::buffer::BufferOptions;
use crate::calibrations::{
    buffer::{self, Buffer, Report},
    errors::CalibrationsErr,
};
use crate::disk::layout::CalibrationsLayout;
use crate::filesys::Dir;

// external crates
use tokio::sync::{Mutex, MutexGuard, Notify};
use uuid::Uuid;

/// The calibration buffer shared by intake and the upload worker, plus the
/// wake-up intake sends the worker after each accepted report.
///
/// Callers never hold [`Self::lock`] across a network call, so intake never
/// waits on Miru.
#[derive(Clone)]
pub struct SharedBuffer {
    buffer: Arc<Mutex<Buffer>>,
    failed: Dir,
    wake: Arc<Notify>,
}

impl SharedBuffer {
    /// Open the calibration buffer at `layout`, parking rejected reports in
    /// [`CalibrationsLayout::failed`].
    pub async fn open(
        layout: &CalibrationsLayout,
        options: BufferOptions,
    ) -> Result<Self, CalibrationsErr> {
        let buffer = buffer::open(layout, options).await?;
        Ok(Self::new(buffer, layout.failed()))
    }

    pub fn new(buffer: Buffer, failed: Dir) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(buffer)),
            failed,
            wake: Arc::new(Notify::new()),
        }
    }

    /// Buffer `report` durably and wake the worker. A full buffer is the 507
    /// [`CalibrationsErr::BufferFullErr`] and wakes nobody.
    pub async fn enqueue(&self, report: Report) -> Result<Uuid, CalibrationsErr> {
        let id = self.buffer.lock().await.enqueue(report).await?;
        self.wake.notify_one();
        Ok(id)
    }

    pub async fn lock(&self) -> MutexGuard<'_, Buffer> {
        self.buffer.lock().await
    }

    /// Where reports Miru permanently rejected are parked.
    pub fn failed_dir(&self) -> &Dir {
        &self.failed
    }

    /// Resolves once a report has been enqueued since the last time this
    /// resolved. A wake sent while nobody waits is kept, so a report enqueued
    /// while the worker is busy is never missed.
    pub async fn woken(&self) {
        self.wake.notified().await;
    }
}

/// [`Buffer`] has no `Debug`, so only the parked-reports directory is shown.
impl fmt::Debug for SharedBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedBuffer")
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}
