// standard crates
use std::cmp::Eq;
use std::fmt::Debug;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Ord, PartialOrd)]
pub struct CacheEntry<K, V>
where
    K: ToString + Serialize,
    V: Clone + Serialize,
{
    pub key: K,
    pub value: V,
    pub is_dirty: bool,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}
