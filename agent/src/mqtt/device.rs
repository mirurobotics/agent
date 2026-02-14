// internal crates
use crate::mqtt::client::MQTTClient;
// use crate::mqtt::device::
use crate::mqtt::{
    errors::*,
    topics::{device_ping, device_pong, device_sync},
};
use crate::trace;

// external crates
use chrono::Utc;
use rumqttc::QoS;

pub type SyncDevice = openapi_client::models::SyncDevice;
pub type Ping = openapi_client::models::Ping;
pub type Pong = openapi_client::models::Pong;

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
pub trait DeviceExt {
    async fn subscribe_device_sync(&self, device_id: &str) -> Result<(), MQTTError>;
    async fn publish_device_sync(&self, device_id: &str) -> Result<(), MQTTError>;
    async fn subscribe_device_ping(&self, device_id: &str) -> Result<(), MQTTError>;
    async fn publish_device_pong(
        &self,
        device_id: &str,
        message_id: String,
    ) -> Result<(), MQTTError>;
}

impl DeviceExt for MQTTClient {
    async fn subscribe_device_sync(&self, device_id: &str) -> Result<(), MQTTError> {
        let topic = device_sync(device_id);
        self.subscribe(&topic, QoS::AtLeastOnce).await
    }

    async fn publish_device_sync(&self, device_id: &str) -> Result<(), MQTTError> {
        let topic = device_sync(device_id);
        let payload = SyncDevice { is_synced: true };
        let payload_bytes = serde_json::to_vec(&payload).map_err(|e| {
            MQTTError::SerdeErr(SerdeErr {
                source: e,
                trace: trace!(),
            })
        })?;
        self.publish(&topic, QoS::AtLeastOnce, true, &payload_bytes)
            .await
    }

    async fn subscribe_device_ping(&self, device_id: &str) -> Result<(), MQTTError> {
        let topic = device_ping(device_id);
        self.subscribe(&topic, QoS::AtLeastOnce).await
    }

    async fn publish_device_pong(
        &self,
        device_id: &str,
        ping_message_id: String,
    ) -> Result<(), MQTTError> {
        let topic = device_pong(device_id);
        let payload = Pong {
            message_id: ping_message_id,
            timestamp: Utc::now().to_rfc3339(),
        };
        let payload_bytes = serde_json::to_vec(&payload).map_err(|e| {
            MQTTError::SerdeErr(SerdeErr {
                source: e,
                trace: trace!(),
            })
        })?;
        self.publish(&topic, QoS::AtLeastOnce, false, &payload_bytes)
            .await
    }
}
