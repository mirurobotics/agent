pub mod client;
pub mod config_instances;
pub mod deployments;
pub mod devices;
pub mod errors;
pub mod git_commits;
pub mod query;
pub mod releases;
pub mod request;
pub mod response;
pub mod retry;

pub use self::errors::HTTPErr;
pub use self::query::QueryParams;
pub use client::{Client, ClientI};
pub use retry::with_retry;
