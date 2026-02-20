// internal crates
use crate::models;
use crate::services::errors::*;
use crate::storage;

pub async fn get_device(device_stor: &storage::Device) -> Result<models::Device, ServiceErr> {
    let device = device_stor.read().await?;
    Ok((*device).clone())
}
