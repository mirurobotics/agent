//! Unit-test fixture subset mounted by `agent/src/lib.rs` as `crate::test_utils`.
//!
//! Every file reached from here is also compiled inside the integration crate
//! (`agent/tests/mod.rs`) under a different parent module. Fixture sources must
//! therefore name the library as `miru_agent::` and reach sibling fixtures only
//! through `super::` within `test_utils/` — never `crate::mocks::…` or
//! `crate::sync::…`, which resolve differently in the two trees.

#[path = "../errors/harnesses.rs"]
pub mod error_harnesses;
#[path = "filesys/mod.rs"]
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
