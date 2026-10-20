// external crates
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};

/// Everything a [`Queue`](super::Queue) needs to know about the job it carries.
pub trait QueueJob:
    Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Log prefix, e.g. `"upload"` or `"delete"`.
    const LABEL: &'static str;

    /// Earliest instant this job may be actioned. Jobs actionable on arrival
    /// return `DateTime::<Utc>::MIN_UTC`.
    fn due_at(&self) -> DateTime<Utc>;

    /// How this job identifies itself in logs and the capacity error.
    fn name(&self) -> String;
}

/// What a [`DirQueue`](super::DirQueue) needs to know about the job it
/// carries, beyond what [`QueueJob`] covers.
pub trait DirQueueJob: QueueJob {
    /// When the item was accepted. Orders a queue rebuilt from the item files,
    /// since the original queue order is lost with `queue.json`.
    fn received_at(&self) -> DateTime<Utc>;
}
