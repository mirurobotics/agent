// internal crates
use crate::queue::QueueJob;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a [`crate::buffer::Buffer`] needs to know about the job it carries, beyond what the
/// queue needs.
pub trait BufferJob: QueueJob {
    /// When the item was accepted. Orders a queue rebuilt from the item files,
    /// since the original queue order is lost with `queue.json`.
    fn received_at(&self) -> DateTime<Utc>;
}

/// One buffered item, and the on-disk shape of its file. The job is stored
/// beside the content so the queue can be rebuilt from the item files alone if
/// `queue.json` is ever lost to corruption.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item<J, C> {
    pub job: J,
    pub content: C,
}
