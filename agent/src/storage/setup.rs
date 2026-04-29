// internal crates
use crate::authn;
use crate::filesys::{self, Overwrite, WriteOptions};
use crate::models;
use crate::storage::{self, errors::*, layout::Layout, settings::Settings};

/// Wipe all stateful files and rewrite them from the supplied inputs. The
/// device's RSA keypair under `auth/` is intentionally NOT touched here —
/// the keypair is the device's identity at the backend, so losing it would
/// orphan the device.
pub async fn reset(
    layout: &Layout,
    device: &models::Device,
    settings: &Settings,
    agent_version: &str,
) -> Result<(), StorageErr> {
    // ensure auth dir exists (token.json lives there)
    let auth_dir = layout.auth();
    auth_dir.root.create_if_absent().await?;

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

    // blank token.json
    let token = authn::Token::default();
    auth_dir
        .token()
        .write_json(&token, WriteOptions::OVERWRITE_ATOMIC)
        .await?;

    // wipe resources directory (also wipes config_instances/, deployments,
    // releases, git_commits — everything cached locally)
    layout.resources().delete().await?;

    // wipe events directory and recreate it
    let events_dir = layout.events_dir();
    events_dir.delete().await?;
    events_dir.create_if_absent().await?;

    // write the new marker last so that when resetting storage for the agent upgrade,
    // the marker represents a complete bootstrap.
    storage::agent_version::write(&layout.agent_version(), agent_version).await?;

    Ok(())
}

/// Initialized the file system state with the given parameters. The storage state is
/// reset if already has existing state.
pub async fn bootstrap(
    layout: &Layout,
    device: &models::Device,
    settings: &Settings,
    private_key_file: &filesys::File,
    public_key_file: &filesys::File,
    version: &str,
) -> Result<(), StorageErr> {
    // create the auth directory
    let auth_dir = layout.auth();
    auth_dir.root.create_if_absent().await?;

    // move the private and public keys to the auth directory
    private_key_file
        .move_to(&auth_dir.private_key(), Overwrite::Allow)
        .await?;
    public_key_file
        .move_to(&auth_dir.public_key(), Overwrite::Allow)
        .await?;

    // delegate to the shared wipe-and-write path
    reset(layout, device, settings, version).await
}
