const VERSION: &str = "v1";

// device sync was the first topic we supported and we didn't use the /v1 prefix :(
// we're just going to keep using it for now
pub fn device_sync(device_id: &str) -> String {
    format!("cmd/devices/{device_id}/sync")
}
pub fn device_ping(device_id: &str) -> String {
    format!("{VERSION}/cmd/devices/{device_id}/ping")
}
pub fn device_pong(device_id: &str) -> String {
    format!("{VERSION}/resp/devices/{device_id}/pong")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionTopics {
    Sync,
    Ping,
    Unknown,
}

pub fn parse_subscription(device_id: &str, topic: &str) -> SubscriptionTopics {
    if topic == device_sync(device_id) {
        SubscriptionTopics::Sync
    } else if topic == device_ping(device_id) {
        SubscriptionTopics::Ping
    } else {
        SubscriptionTopics::Unknown
    }
}
