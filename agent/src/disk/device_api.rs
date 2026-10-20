//! Discovery file for TCP clients of the local device API.
//!
//! The app writes the file once the TCP listener is bound and removes it after
//! the listener stops, so the file exists only while TCP is being served. It
//! holds that listener's port and the bearer token for this agent run.
//! Clients:
//! - read the file, then close it right away;
//! - treat a missing file or a refused connection as "not serving" and retry
//!   later by re-reading the file;
//! - re-read the file on a 401, because the token changes on every start.

// standard crates
use std::time::Duration;

// internal crates
use crate::cooldown::retry;
use crate::disk::errors::DiskErr;
use crate::filesys::{self, files, Atomic, FileSysErr, Overwrite, WriteOptions};

// external crates
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

/// Windows refuses to replace or delete a file while another process has it
/// open without delete sharing, which Python's `open` does. Readers close the
/// file quickly, so a few short retries ride out the overlap. Unix has no such
/// conflict, so it tries once.
const RETRY_DELAYS: &[Duration] = if cfg!(windows) {
    &[Duration::from_millis(50); 9]
} else {
    &[]
};

/// Discovery file contents. Not `Debug`, so the token cannot reach logs.
#[derive(Serialize)]
struct Discovery<'a> {
    port: u16,
    token: &'a str,
}

/// Atomically write the TCP port and bearer token for local device API
/// clients. On Unix the file mode is 0640 (owner read-write, group read); on
/// Windows the file inherits the `device-api` directory ACL.
pub async fn write(file: &filesys::File, port: u16, token: &SecretString) -> Result<(), DiskErr> {
    let discovery = Discovery {
        port,
        token: token.expose_secret(),
    };
    let opts = WriteOptions {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
        mode: Some(0o640),
    };
    retry(
        |n| RETRY_DELAYS.get(n as usize).copied(),
        |_: &FileSysErr| true,
        tokio::time::sleep,
        || files::write_json(file, &discovery, opts),
    )
    .await?;
    Ok(())
}

/// Remove the discovery file; a missing file is not an error.
pub async fn remove(file: &filesys::File) -> Result<(), DiskErr> {
    retry(
        |n| RETRY_DELAYS.get(n as usize).copied(),
        |_: &FileSysErr| true,
        tokio::time::sleep,
        || files::delete(file),
    )
    .await?;
    Ok(())
}
