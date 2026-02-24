use crate::http;
use crate::models;
use crate::storage;
use crate::sync::errors::*;

use tracing::info;

pub async fn push<HTTPClientT: http::ClientI>(
    device_stor: &storage::Device,
    http_client: &HTTPClientT,
    token: &str,
    agent_version: String,
) -> Result<(), SyncErr> {
    let device = device_stor.read().await?;
    if device.agent_version == agent_version {
        return Ok(());
    }

    info!(
        "detected new agent version: {} -> {}",
        device.agent_version, agent_version
    );

    // update the backend with the new agent version first so that if it fails, we don't
    // patch the device file and try again later
    http::devices::update(
        http_client,
        http::devices::UpdateParams {
            id: &device.id,
            payload: &openapi_client::models::UpdateDeviceFromAgentRequest {
                agent_version: Some(agent_version.clone()),
            },
            token,
        },
    )
    .await?;

    // update the device file
    let updates = models::device::Updates {
        agent_version: Some(agent_version),
        ..models::device::Updates::empty()
    };
    device_stor.patch(updates).await?;

    Ok(())
}
