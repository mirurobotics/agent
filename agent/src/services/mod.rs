pub mod backend;
pub mod deployment;
pub mod device;
pub mod errors;
pub mod events;
pub mod git_commit;
pub mod release;

pub use self::backend::{BackendFetcher, HttpBackend};
pub use self::errors::ServiceErr;
