// internal crates
use crate::authn;
use crate::disk::{self, errors::*, layout::Layout, settings::Settings};
use crate::filesys::{self, dirs, files, Overwrite, WriteOptions};
use crate::models;

/// Wipe all stateful files and rewrite them from the supplied inputs. The
/// device's RSA keypair under `auth/` is intentionally NOT touched here —
/// the keypair is the device's identity at the backend, so losing it would
/// orphan the device.
pub async fn reset(
    layout: &Layout,
    device: &models::Device,
    settings: &Settings,
    agent_version: &str,
) -> Result<(), DiskErr> {
    // ensure auth dir exists (token.json lives there)
    let auth_dir = layout.auth();
    dirs::create_if_absent(&auth_dir.root).await?;

    // overwrite the device file
    let device_file = layout.device();
    files::write_json(&device_file, &device, WriteOptions::OVERWRITE_ATOMIC).await?;

    // overwrite the settings file
    let settings_file = layout.settings();
    files::write_json(&settings_file, &settings, WriteOptions::OVERWRITE_ATOMIC).await?;

    // blank token.json — 0o600 since the token is a live bearer credential
    // (and the MQTT password); the update path in the token manager matches.
    let token = authn::Token::default();
    files::write_json(
        &auth_dir.token(),
        &token,
        WriteOptions::OVERWRITE_ATOMIC_0600,
    )
    .await?;

    // wipe resources directory (also wipes config_instances/, deployments,
    // releases, git_commits — everything cached locally)
    dirs::delete(&layout.resources()).await?;

    // wipe events directory and recreate it
    let events_dir = layout.events_dir();
    dirs::delete(&events_dir).await?;
    dirs::create_if_absent(&events_dir).await?;

    // write the new marker last so that when resetting storage for the agent upgrade,
    // the marker represents a complete bootstrap.
    disk::agent_version::write(&layout.agent_version(), agent_version).await?;

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
) -> Result<(), DiskErr> {
    // create the auth directory
    let auth_dir = layout.auth();
    dirs::create_if_absent(&auth_dir.root).await?;

    // move the private and public keys to the auth directory
    files::move_to(private_key_file, &auth_dir.private_key(), Overwrite::Allow).await?;
    files::move_to(public_key_file, &auth_dir.public_key(), Overwrite::Allow).await?;

    // delegate to the shared wipe-and-write path
    reset(layout, device, settings, version).await
}
