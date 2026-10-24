// external crates
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};

/// Everything a [`FileQueue`](super::FileQueue) needs to know about the job it carries.
pub trait QueueJob:
    Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Log prefix, e.g. `"upload"` or `"delete"`.
    const LABEL: &'static str;

    /// Earliest instant this job may be actioned. Jobs actionable on arrival
    /// return `DateTime::<Utc>::MIN_UTC`.
    fn ready_at(&self) -> DateTime<Utc>;

    /// How this job identifies itself in logs and the capacity error.
    fn name(&self) -> String;
}
