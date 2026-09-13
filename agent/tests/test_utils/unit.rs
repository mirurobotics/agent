//! Shared fixtures needed by owner-local unit tests.

#[path = "../errors/harnesses.rs"]
pub mod error_harnesses;
#[path = "filesys.rs"]
pub mod filesys;
#[path = "../mocks/http_client.rs"]
pub mod http_client;
#[path = "sync.rs"]
pub mod sync;
#[path = "../sync/helpers.rs"]
pub mod sync_helpers;
#[path = "../mocks/token_manager.rs"]
pub mod token_manager;
#[path = "upload.rs"]
pub mod upload;
