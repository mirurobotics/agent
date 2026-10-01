// internal crates
use crate::disk::errors::DiskErr;
use crate::filesys::{self, files, PathExt, WriteOptions};
use crate::models::SystemMetadata;

/// Read the cached device system metadata, or `Ok(None)` when the cache file
/// does not yet exist. Mirrors [`crate::disk::agent_version::read`] but JSON-typed.
pub async fn read(file: &filesys::File) -> Result<Option<SystemMetadata>, DiskErr> {
    if !file.exists() {
        return Ok(None);
    }
    let meta = files::read_json::<SystemMetadata>(file).await?;
    Ok(Some(meta))
}

/// Atomically overwrite the cached device system metadata.
pub async fn write(file: &filesys::File, meta: &SystemMetadata) -> Result<(), DiskErr> {
    files::write_json(file, meta, WriteOptions::OVERWRITE_ATOMIC).await?;
    Ok(())
}
