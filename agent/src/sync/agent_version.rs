use crate::http;
use crate::http::devices;
use crate::models::device;
use crate::storage::device::DeviceFile;
use crate::sync::errors::*;

use tracing::info;

pub async fn push<HTTPClientT: http::ClientI>(
    device_file: &DeviceFile,
    http_client: &HTTPClientT,
    token: &str,
    agent_version: String,
) -> Result<(), SyncErr> {
    let device = device_file.read().await?;
    if device.agent_version == agent_version {
        return Ok(());
    }

    info!(
        "Detected new agent version: {} -> {}",
        device.agent_version, agent_version
    );

    // update the device file
    let updates = device::Updates {
        agent_version: Some(agent_version.clone()),
        ..device::Updates::empty()
    };

    device_file.patch(updates).await?;

    // update the backend
    devices::update(
        http_client,
        devices::UpdateParams {
            device_id: &device.id,
            payload: &openapi_client::models::UpdateDeviceFromAgentRequest {
                agent_version: Some(agent_version),
            },
            token,
        },
    )
    .await?;

    Ok(())
}
