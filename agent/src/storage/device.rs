// internal crates
use crate::filesys::{self, cached_file::ConcurrentCachedFile, PathExt};
use crate::models::{self, device};
use crate::storage::errors::{DeviceNotActivatedErr, StorageErr};
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
