//! Windows Service Control Manager integration.
//!
//! [`scm`] holds the SCM entry point, control handler, and status lifecycle
//! and compiles only on Windows. [`errors`] compiles everywhere so Linux can
//! test `NotLaunchedByScm`. Portable stop types live in [`crate::shutdown`].

pub mod errors;
#[cfg(windows)]
pub mod scm;
