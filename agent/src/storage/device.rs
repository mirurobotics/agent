// internal crates
use crate::filesys::{cached_file::ConcurrentCachedFile, file::File, path::PathExt};
use crate::models::{device, device::Device};
use crate::storage::errors::{DeviceNotActivatedErr, StorageErr};
use crate::trace;

pub type DeviceFile = ConcurrentCachedFile<Device, device::Updates>;

pub async fn assert_activated(device_file: &File) -> Result<(), StorageErr> {
    // check the agent file exists
    device_file.assert_exists()?;

    // attempt to read it
    let device = device_file.read_json::<Device>().await?;

    // check the agent is activated
    if !device.activated {
        return Err(StorageErr::DeviceNotActivatedErr(DeviceNotActivatedErr {
            msg: "device is not activated".to_string(),
            trace: trace!(),
        }));
    }

    Ok(())
}
