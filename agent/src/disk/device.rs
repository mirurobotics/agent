// internal crates
use crate::authn::token_mngr::TokenFile;
use crate::crypt::jwt;
use crate::disk::{
    errors::{DeviceNotActivatedErr, DiskErr, ResolveDeviceIDErr},
    layout::Layout,
};
use crate::filesys::{
    files,
    state_file::{ConcurrentStateFile, Options},
    PathExt,
};
use crate::models::{self, device};
use crate::trace;

pub type Device = ConcurrentStateFile<models::Device, device::Updates>;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Activation {
    Activated,
    NotActivated,
}

pub fn activation_state(layout: &Layout) -> Result<Activation, DiskErr> {
    let auth_dir = layout.auth();
    if !auth_dir.private_key().try_exists()? {
        return Ok(Activation::NotActivated);
    }
    if !auth_dir.public_key().try_exists()? {
        return Ok(Activation::NotActivated);
    }

    Ok(Activation::Activated)
}

pub fn assert_activated(layout: &Layout) -> Result<(), DiskErr> {
    match activation_state(layout)? {
        Activation::Activated => Ok(()),
        Activation::NotActivated => Err(DiskErr::DeviceNotActivatedErr(DeviceNotActivatedErr {
            msg: "device is not activated".to_string(),
            trace: trace!(),
        })),
    }
}

/// Resolve the device id from the on-disk state.
pub async fn resolve_device_id(layout: &Layout) -> Result<String, DiskErr> {
    // attempt to get the device id from the device file
    let device_file_err = match files::read_json::<models::Device>(&layout.device()).await {
        Ok(device) => return Ok(device.id),
        Err(e) => e,
    };

    // attempt to get the device id from the existing token on file (0o600: the
    // token is a live bearer credential and doubles as the MQTT password).
    let token_file = TokenFile::open(
        layout.auth().token(),
        Options {
            default: Some(crate::authn::Token::default()),
            mode: Some(0o600),
        },
    )
    .await?;
    let token = token_file.read();
    let jwt_err = match jwt::extract_device_id(&token.token) {
        Ok(device_id) => return Ok(device_id),
        Err(e) => e,
    };

    Err(DiskErr::ResolveDeviceIDErr(Box::new(ResolveDeviceIDErr {
        device_file_err: Box::new(device_file_err),
        jwt_err: Box::new(jwt_err),
        trace: trace!(),
    })))
}
