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
