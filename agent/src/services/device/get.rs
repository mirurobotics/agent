// internal crates
use crate::models::device::Device;
use crate::services::errors::*;
use crate::storage::device::DeviceFile;

pub async fn get_device(device_file: &DeviceFile) -> Result<Device, ServiceErr> {
    let device = device_file.read().await?;
    Ok((*device).clone())
}
