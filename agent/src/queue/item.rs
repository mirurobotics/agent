// external crates
use serde::{Deserialize, Serialize};

/// One [`DirQueue`](super::DirQueue) item, and the on-disk shape of its file. The job is stored
/// beside the content so the queue can be rebuilt from the item files alone if
/// `queue.json` is ever lost to corruption.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item<J, C> {
    pub job: J,
    pub content: C,
}
