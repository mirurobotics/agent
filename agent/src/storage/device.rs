// internal crates
use crate::authn::token_mngr::TokenFile;
use crate::crypt::jwt;
use crate::filesys::{cached_file::ConcurrentCachedFile, PathExt};
use crate::models::{self, device};
use crate::storage::{
    errors::{DeviceNotActivatedErr, ResolveDeviceIDErr, StorageErr},
    layout::Layout,
};
use crate::trace;

pub type Device = ConcurrentCachedFile<models::Device, device::Updates>;

pub async fn assert_activated(layout: &Layout) -> Result<(), StorageErr> {
    let auth_dir = layout.auth();
    if !auth_dir.private_key().exists() {
        return Err(StorageErr::DeviceNotActivatedErr(DeviceNotActivatedErr {
            msg: "device is not activated".to_string(),
            trace: trace!(),
        }));
    }
    if !auth_dir.public_key().exists() {
        return Err(StorageErr::DeviceNotActivatedErr(DeviceNotActivatedErr {
            msg: "device is not activated".to_string(),
            trace: trace!(),
        }));
    }

    Ok(())
}

/// Resolve the device id from the on-disk state.
pub async fn resolve_device_id(layout: &Layout) -> Result<String, StorageErr> {
    // attempt to get the device id from the device file
    let device_file_err = match layout.device().read_json::<models::Device>().await {
        Ok(device) => return Ok(device.id),
        Err(e) => e,
    };

    // attempt to get the device id from the existing token on file
    let token_file =
        TokenFile::new_with_default(layout.auth().token(), crate::authn::Token::default()).await?;
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
