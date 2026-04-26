// internal crates
use crate::authn::token_mngr::TokenFile;
use crate::crypt::jwt;
use crate::filesys::{self, cached_file::ConcurrentCachedFile, PathExt};
use crate::models::{self, device};
use crate::storage::{
    errors::{DeviceNotActivatedErr, ResolveDeviceIDErr, StorageErr},
    layout::Layout,
};
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

/// Recover the device id from on-disk state without requiring the device file
/// to be parseable. Falls back to extracting the device id from the JWT in
/// `auth/token.json` if `device.json` is missing or corrupt.
///
/// Returns `ResolveDeviceIDErr` when neither source yields a device id —
/// typically because the agent was never installed.
pub async fn resolve_device_id(layout: &Layout) -> Result<String, StorageErr> {
    // attempt to get the device id from the device file
    let device_file_err = match layout.device().read_json::<models::Device>().await {
        Ok(device) => return Ok(device.id),
        Err(e) => e,
    };

    // attempt to get the device id from the existing token on file
    let auth = layout.auth();
    let token_file_path = auth.token();
    let token_file =
        TokenFile::new_with_default(token_file_path, crate::authn::Token::default()).await?;
    let token = token_file.read().await;
    let jwt_err = match jwt::extract_device_id(&token.token) {
        Ok(device_id) => return Ok(device_id),
        Err(e) => e,
    };

    Err(StorageErr::ResolveDeviceIDErr(Box::new(
        ResolveDeviceIDErr {
            device_file_err: Box::new(device_file_err),
            jwt_err: Box::new(jwt_err),
            trace: trace!(),
        },
    )))
}
