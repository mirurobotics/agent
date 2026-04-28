// standard crates
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::app::errors::UpgradeErr;
use crate::authn::{self, token::Token};
use crate::cooldown;
use crate::filesys::PathExt;
use crate::http::{self, ClientI};
use crate::models;
use crate::storage::{self, Layout, Settings};

// external crates
use tracing::{error, info, warn};

/// Reconcile on-disk state with the running version. No-op if the marker matches;
/// otherwise wipes per-version state and rebootstraps from the backend. Blocks
/// indefinitely on network failure to avoid leaving a half-wiped device.
pub async fn reconcile<F, Fut, HTTPClientT: ClientI>(
    layout: &Layout,
    http_client: &HTTPClientT,
    version: &str,
    sleep_fn: F,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    let backoff = cooldown::Backoff {
        base_secs: 1,
        growth_factor: 2,
        max_secs: 60,
    };
    let mut err_streak: u32 = 0;

    loop {
        if !needs_upgrade(layout, version).await {
            return;
        }
        info!("resetting miru agent state to use version '{}'", version);

        match reconcile_impl(http_client, layout, version).await {
            Ok(_) => break,
            Err(e) => {
                warn!("updating agent version storage failed: {e}");
                err_streak = err_streak.saturating_add(1);
                let wait = cooldown::calc(&backoff, err_streak);
                warn!("retrying in {wait} seconds (error streak: {err_streak})");
                sleep_fn(Duration::from_secs(wait as u64)).await;
            }
        }
    }

    info!(
        "upgrade: resetting storage state for version '{}' complete",
        version
    );
}

pub async fn needs_upgrade(layout: &Layout, cur_version: &str) -> bool {
    let marker_file = layout.agent_version();
    match storage::agent_version::read(&marker_file).await {
        Ok(None) => {
            info!("unable to determine previous miru agent package version");
            true
        }
        Ok(Some(prev_version)) => {
            if prev_version == cur_version {
                return false;
            }
            info!(
                "detected miru agent package updated from version '{}' to version '{}'",
                prev_version, cur_version
            );
            true
        }
        Err(e) => {
            error!("unable to read agent version marker file: {e}");
            info!("unable to determine previous miru agent package version");
            true
        }
    }
}

pub async fn reconcile_impl<HTTPClientT: ClientI>(
    http_client: &HTTPClientT,
    layout: &Layout,
    version: &str,
) -> Result<(), UpgradeErr> {
    let token = issue_token(http_client, layout).await?;
    let device = fetch_device(http_client, &token).await?;
    storage::setup::reset(layout, &device, &Settings::default(), version).await?;
    update_device(http_client, &device, version, &token).await?;
    Ok(())
}

async fn issue_token<HTTPClientT: ClientI>(
    http_client: &HTTPClientT,
    layout: &Layout,
) -> Result<Token, UpgradeErr> {
    let auth_dir = layout.auth();
    let private_key_file = auth_dir.private_key();
    private_key_file.assert_exists()?;
    let public_key_file = auth_dir.public_key();
    public_key_file.assert_exists()?;
    let token = authn::issue_token(http_client, &private_key_file, &public_key_file).await?;
    Ok(token)
}

async fn fetch_device<HTTPClientT: ClientI>(
    http_client: &HTTPClientT,
    token: &Token,
) -> Result<models::Device, UpgradeErr> {
    let api_device = http::devices::get(http_client, &token.token).await?;
    Ok((&api_device).into())
}

async fn update_device<HTTPClientT: ClientI>(
    http_client: &HTTPClientT,
    device: &models::Device,
    version: &str,
    token: &Token,
) -> Result<(), UpgradeErr> {
    http::devices::update(
        http_client,
        http::devices::UpdateParams {
            id: &device.id,
            payload: &backend_api::models::UpdateDeviceFromAgentRequest {
                agent_version: Some(version.to_string()),
            },
            token: &token.token,
        },
    )
    .await?;
    Ok(())
}
