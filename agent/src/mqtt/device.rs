// internal crates
use crate::mqtt::client::{ClientI, Publish};
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

pub async fn subscribe_sync(client: &impl ClientI, device_id: &str) -> Result<(), MQTTError> {
    let topic = device_sync(device_id);
    client.subscribe(&topic, QoS::AtLeastOnce).await
}

pub async fn publish_sync(client: &impl ClientI, device_id: &str) -> Result<(), MQTTError> {
    let topic = device_sync(device_id);
    let payload = SyncDevice { is_synced: true };
    let payload_bytes = serde_json::to_vec(&payload).map_err(|e| {
        MQTTError::SerdeErr(SerdeErr {
            source: e,
            trace: trace!(),
        })
    })?;
    client
        .publish(Publish {
            topic: &topic,
            qos: QoS::AtLeastOnce,
            retained: true,
            payload: &payload_bytes,
        })
        .await
}

pub async fn subscribe_ping(client: &impl ClientI, device_id: &str) -> Result<(), MQTTError> {
    let topic = device_ping(device_id);
    client.subscribe(&topic, QoS::AtLeastOnce).await
}

pub async fn publish_pong(
    client: &impl ClientI,
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
    client
        .publish(Publish {
            topic: &topic,
            qos: QoS::AtLeastOnce,
            retained: false,
            payload: &payload_bytes,
        })
        .await
}
