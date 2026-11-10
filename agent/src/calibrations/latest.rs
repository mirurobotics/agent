// internal crates
pub use crate::calibrations::latest_impl::{Filter, LatestStore};

/// The most keys the store holds in the agent.
pub const DEFAULT_CAPACITY: usize = 1000;
