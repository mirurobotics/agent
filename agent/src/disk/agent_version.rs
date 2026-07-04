// internal crates
use crate::disk::errors::DiskErr;
use crate::filesys::{self, PathExt, WriteOptions};

pub async fn read(file: &filesys::File) -> Result<Option<String>, DiskErr> {
    if !file.exists() {
        return Ok(None);
    }
    let raw = file.read_string().await?;
    Ok(Some(raw.trim().to_string()))
}

pub async fn write(file: &filesys::File, version: &str) -> Result<(), DiskErr> {
    let body = format!("{}\n", version.trim());
    file.write_string(&body, WriteOptions::OVERWRITE_ATOMIC)
        .await?;
    Ok(())
}
