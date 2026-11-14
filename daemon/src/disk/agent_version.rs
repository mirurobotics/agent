// internal crates
use crate::disk::errors::DiskErr;
use crate::filesys::{self, files, PathExt, WriteOptions};

pub async fn read(file: &filesys::File) -> Result<Option<String>, DiskErr> {
    if !file.exists() {
        return Ok(None);
    }
    let raw = files::read_string(file).await?;
    Ok(Some(raw.trim().to_string()))
}

pub async fn write(file: &filesys::File, version: &str) -> Result<(), DiskErr> {
    let body = format!("{}\n", version.trim());
    files::write_string(file, &body, WriteOptions::OVERWRITE_ATOMIC).await?;
    Ok(())
}
