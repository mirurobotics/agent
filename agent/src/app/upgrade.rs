// standard crates
use std::time::Duration;

// internal crates
use crate::app::errors::UpgradeErr;
use crate::authn;
use crate::cooldown;
use crate::filesys::PathExt;
use crate::http::{self, ClientI};
use crate::models;
use crate::storage::{self, Layout, Settings};

// external crates
use tracing::{info, warn};

/// Reconcile on-disk state with the running version. No-op if the marker
/// matches; otherwise wipes per-version state and rebootstraps from the
/// backend. Blocks indefinitely on network failure to avoid leaving a
/// half-wiped device.
pub async fn reconcile<HTTPClientT: ClientI>(
    layout: &Layout,
    http_client: &HTTPClientT,
    version: &str,
) -> Result<(), UpgradeErr> {
    // fast path: marker exists and matches running version
    let marker_file = layout.agent_version();
    if let Some(marker) = storage::agent_version::read(&marker_file).await? {
        if marker == version {
            return Ok(());
        }
        info!(
            "upgrade: on-disk marker version '{}' differs from running version '{}'; rebootstrapping",
            marker, version
        );
    } else {
        info!(
            "upgrade: no on-disk version marker; rebootstrapping at version '{}'",
            version
        );
    }

    // device id (with fallback to the on-disk JWT — see resolve_device_id)
    let device_id = storage::resolve_device_id(layout).await?;

    let auth_dir = layout.auth();
    let private_key_file = auth_dir.private_key();
    private_key_file.assert_exists()?;
    let public_key_file = auth_dir.public_key();
    public_key_file.assert_exists()?;

    let backoff = cooldown::Backoff {
        base_secs: 1,
        growth_factor: 2,
        max_secs: 12 * 60 * 60, // 12 hours
    };

    // fetch the current Device record from the backend (forever-retry on
    // network errors)
    let device = retry_forever(&backoff, "GET /device", || async {
        let token = authn::issue_token(http_client, &private_key_file, &public_key_file).await?;
        let device = http::devices::get(http_client, &token.token).await?;
        Ok::<backend_api::models::Device, UpgradeErr>(device)
    })
    .await?;

    // build the on-disk Device with the running agent version stamped in
    let device_model: models::Device = (&device).into();

    // wipe + rewrite all stateful files (also writes the marker)
    let settings = Settings::default();
    storage::setup::reset(layout, &device_model, &settings, version).await?;

    // PATCH after the marker is on disk: a crash here re-enters next boot,
    // sees the matching marker, and skips the rebootstrap — so the PATCH must
    // succeed within this call or the backend never learns the new version.
    retry_forever(&backoff, "PATCH /devices/{id}", || async {
        let token = authn::issue_token(http_client, &private_key_file, &public_key_file).await?;
        http::devices::update(
            http_client,
            http::devices::UpdateParams {
                id: &device_id,
                payload: &backend_api::models::UpdateDeviceFromAgentRequest {
                    agent_version: Some(version.to_string()),
                },
                token: &token.token,
            },
        )
        .await?;
        Ok::<(), UpgradeErr>(())
    })
    .await?;

    info!("upgrade: rebootstrap complete at version '{}'", version);
    Ok(())
}

/// Run `op` repeatedly with exponential backoff. Network/HTTP/auth errors
/// are retried indefinitely. Errors that are local to the agent (storage,
/// filesystem, error-resolving the device id) propagate out.
async fn retry_forever<F, Fut, T>(
    backoff: &cooldown::Backoff,
    label: &str,
    mut op: F,
) -> Result<T, UpgradeErr>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, UpgradeErr>>,
{
    let mut err_streak: u32 = 0;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if !is_retryable(&e) {
                    return Err(e);
                }
                warn!("upgrade: {label} failed; will retry: {e}");
                let wait = cooldown::calc(backoff, err_streak);
                tokio::time::sleep(Duration::from_secs(wait as u64)).await;
                err_streak = err_streak.saturating_add(1);
            }
        }
    }
}

fn is_retryable(err: &UpgradeErr) -> bool {
    // Retry on transport-level errors. Anything else (including local
    // storage or filesystem errors) is treated as a hard failure that
    // should surface to the operator.
    matches!(err, UpgradeErr::HTTPErr(_) | UpgradeErr::AuthnErr(_))
}
