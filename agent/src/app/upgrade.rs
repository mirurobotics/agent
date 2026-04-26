// standard crates
use std::time::Duration;

// internal crates
use crate::authn;
use crate::cooldown;
use crate::filesys::PathExt;
use crate::http::{self, ClientI};
use crate::models;
use crate::storage::{self, Layout, Settings};

// external crates
use tracing::{info, warn};

#[derive(Debug, thiserror::Error)]
pub enum UpgradeErr {
    #[error(transparent)]
    StorageErr(#[from] storage::StorageErr),
    #[error(transparent)]
    HTTPErr(#[from] http::HTTPErr),
    #[error(transparent)]
    AuthnErr(#[from] authn::AuthnErr),
    #[error(transparent)]
    FileSysErr(#[from] crate::filesys::FileSysErr),
}

/// Idempotent upgrade gate. Called at boot before `assert_activated` to
/// reconcile the on-disk schema with the running binary's compile-time
/// `version::VERSION`. If the marker file at `Layout::agent_version()`
/// matches `version`, this is a no-op. Otherwise this rebootstraps the
/// agent's persistent state in place: it re-fetches the `Device` record
/// from the backend (using a JWT minted from the on-disk private key),
/// rewrites `device.json`, `settings.json`, and `auth/token.json` from
/// compile-time defaults plus the backend response, wipes
/// `resources/` and `events/`, PATCHes the backend with the new agent
/// version, and finally writes the marker.
///
/// The keypair under `auth/` is never touched — it is the device's
/// identity at the backend and losing it would orphan the device.
///
/// Network failures retry forever using exponential backoff. A
/// rebootstrap that fails fast would leave the agent in a half-wiped
/// state on the next boot; blocking until the backend is reachable is
/// the safer choice.
pub async fn ensure<HTTPClientT: ClientI>(
    layout: &Layout,
    http_client: &HTTPClientT,
    version: &str,
) -> Result<(), UpgradeErr> {
    // fast path: marker exists and matches running version
    let marker_file = layout.agent_version();
    if let Some(marker) = storage::agent_version::read(&marker_file).await? {
        if marker.version == version {
            return Ok(());
        }
        info!(
            "upgrade: on-disk marker version '{}' differs from running version '{}'; rebootstrapping",
            marker.version, version
        );
    } else {
        info!(
            "upgrade: no on-disk version marker; rebootstrapping at version '{}'",
            version
        );
    }

    // resolve the device id from the on-disk state. If the device file is
    // missing or corrupt, fall back to the JWT in the on-disk token. If
    // both fail the agent has never been installed and cannot recover —
    // surface that error so the operator can run `miru-agent install`.
    let device_id = storage::resolve_device_id(layout).await?;

    let auth_dir = layout.auth();
    let private_key_file = auth_dir.private_key();
    private_key_file.assert_exists()?;

    let backoff = cooldown::Backoff {
        base_secs: 1,
        growth_factor: 2,
        max_secs: 12 * 60 * 60, // 12 hours
    };

    // fetch the current Device record from the backend (forever-retry on
    // network errors)
    let device = retry_forever(&backoff, "GET /device", || async {
        let token = authn::issue_token(http_client, &private_key_file, &device_id).await?;
        let device = http::devices::get(http_client, &token.token).await?;
        Ok::<backend_api::models::Device, UpgradeErr>(device)
    })
    .await?;

    // build the on-disk Device with the running agent version stamped in
    let mut device_model: models::Device = (&device).into();
    device_model.agent_version = version.to_string();

    // wipe + rewrite all stateful files (also writes the marker)
    let settings = Settings::default();
    storage::setup::reset(layout, &device_model, &settings, version).await?;

    // tell the backend about the new agent version (forever-retry on
    // network errors). The marker is already on disk, so a crash here will
    // re-enter `ensure` next boot — the marker check at the top of this
    // function will pass and we'll skip the rebootstrap. The PATCH must
    // therefore retry inside `ensure` so the backend always learns the new
    // version before we let the agent boot fully.
    retry_forever(&backoff, "PATCH /devices/{id}", || async {
        let token = authn::issue_token(http_client, &private_key_file, &device_id).await?;
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
