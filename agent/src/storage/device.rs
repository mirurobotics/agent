// internal crates
use super::errors::{DeviceNotActivatedErr, StorageErr};
use crate::filesys;
use crate::filesys::cached_file::ConcurrentCachedFile;
use crate::filesys::PathExt;
use crate::models::{self, device};
use crate::trace;

pub type Device = ConcurrentCachedFile<models::Device, device::Updates>;

pub async fn assert_activated(device_file: &filesys::File) -> Result<(), StorageErr> {
    // check the agent file exists
    device_file.assert_exists()?;

    // attempt to read it
    let device = device_file.read_json::<models::Device>().await?;

    // check the agent is activated
    if !device.activated {
        return Err(StorageErr::DeviceNotActivatedErr(DeviceNotActivatedErr {
            msg: "device is not activated".to_string(),
            trace: trace!(),
        }));
    }

    Ok(())
}
