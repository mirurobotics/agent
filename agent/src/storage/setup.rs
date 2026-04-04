// internal crates
use crate::authn;
use crate::filesys::{self, Overwrite, WriteOptions};
use crate::models;
use crate::storage::{errors::*, layout::Layout, settings::Settings};

pub async fn bootstrap(
    layout: &Layout,
    device: &models::Device,
    settings: &Settings,
    private_key_file: &filesys::File,
    public_key_file: &filesys::File,
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
    let token = authn::Token::default();
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

    // wipe events directory
    let events_dir = layout.events_dir();
    events_dir.delete().await?;
    events_dir.create_if_absent().await?;

    Ok(())
}
