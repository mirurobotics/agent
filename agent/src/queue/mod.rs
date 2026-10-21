//! Durable FIFO job queues.
//!
//! [`FileQueue`] keeps its entries in memory and, optionally, in one snapshot file
//! (`queue.json`). [`DirQueue`] adds a payload per entry for jobs too large to
//! rewrite on every change: the entries stay in a [`FileQueue`], and each payload
//! gets its own file in a content directory.

pub mod dir;
pub mod errors;
pub mod file;
pub mod item;
pub mod job;
pub mod snapshot;

pub use self::dir::{DirQueue, DirQueueOptions};
pub use self::errors::{DirQueueErr, EnqueueErr, ItemTooLargeErr, QueueFullErr, SerializeItemErr};
pub use self::file::{FileQueue, Persistence};
pub use self::item::Item;
pub use self::job::QueueJob;
pub use self::snapshot::{QueueEntry, QueueSnapshot, QueueSnapshotFile};
