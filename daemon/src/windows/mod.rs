//! Windows Service Control Manager integration.
//!
//! [`scm`] holds the control handler and status lifecycle and compiles only
//! on Windows. The binary owns `ServiceMain` (next to the agent body).
//! [`errors`] compiles everywhere so Linux can test `NotLaunchedByScm`.
//! Portable stop types live in [`crate::shutdown`].

pub mod errors;
#[cfg(windows)]
pub mod scm;
