//! Marker file written to `/var/lib/miru/agent_version` recording the agent
//! version of the binary that last successfully bootstrapped the on-disk
//! state. Read once at boot and written once on rebootstrap.
//!
//! Plain UTF-8 text — one line, the version string. Plain text rather than
//! JSON because the marker carries a single field and the wipe-and-rebootstrap
//! flow exists precisely to tolerate schema churn elsewhere; this is the one
//! file where having no schema is the point.

// internal crates
use crate::filesys::{self, PathExt, WriteOptions};
use crate::storage::errors::StorageErr;

pub async fn read(file: &filesys::File) -> Result<Option<String>, StorageErr> {
    if !file.exists() {
        return Ok(None);
    }
    let raw = file.read_string().await?;
    Ok(Some(raw.trim().to_string()))
}

pub async fn write(file: &filesys::File, version: &str) -> Result<(), StorageErr> {
    let body = format!("{}\n", version.trim());
    file.write_string(&body, WriteOptions::OVERWRITE_ATOMIC)
        .await?;
    Ok(())
}
