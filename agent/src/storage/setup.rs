// internal crates
use super::errors::*;
use super::layout::Layout;
use super::settings::Settings;
use crate::authn::token::Token;
use crate::filesys::file::File;
use crate::filesys::{Overwrite, WriteOptions};
use crate::models::device::Device;

pub async fn bootstrap(
    layout: &Layout,
    device: &Device,
    settings: &Settings,
    private_key_file: &File,
    public_key_file: &File,
) -> Result<(), StorageErr> {
    // overwrite the device file
    let device_file = layout.device();
    device_file
        .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
        .await?;

    // overwrite the settings file
    let settings_file = layout.settings();
    settings_file
        .write_json(&settings, WriteOptions::OVERWRITE_ATOMIC)
        .await?;

    // create the auth directory
    let auth_dir = layout.auth();
    auth_dir.root.create_if_absent().await?;

    // overwrite the auth file
    let token = Token::default();
    let auth_file = auth_dir.token();
    auth_file
        .write_json(&token, WriteOptions::OVERWRITE_ATOMIC)
        .await?;

    // move the private and public keys to the auth directory
    private_key_file
        .move_to(&auth_dir.private_key(), Overwrite::Allow)
        .await?;
    public_key_file
        .move_to(&auth_dir.public_key(), Overwrite::Allow)
        .await?;

    // wipe the customer configs directory
    let customer_configs_dir = layout.customer_configs();
    customer_configs_dir.delete().await?;
    customer_configs_dir.create_if_absent().await?;

    // wipe resources directory
    let resources_dir = layout.resources();
    resources_dir.delete().await?;

    Ok(())
}
