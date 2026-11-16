// internal crates
use crate::disk;
use crate::models;
use crate::services::errors::*;

pub async fn get(device_stor: &disk::Device) -> Result<models::Device, ServiceErr> {
    let device = device_stor.read().await?;
    Ok((*device).clone())
}
