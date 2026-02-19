use crate::errors::Trace;

#[derive(Debug, thiserror::Error)]
#[error("Failed to authenticate to MQTT broker: {source}")]
pub struct AuthenticationErr {
    pub source: rumqttc::ConnectionError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for AuthenticationErr {}

#[derive(Debug, thiserror::Error)]
#[error("Failed to connect to MQTT broker: {source}")]
pub struct NetworkConnectionErr {
    pub source: rumqttc::ConnectionError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for NetworkConnectionErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to poll event loop: {source}")]
pub struct PollErr {
    pub source: rumqttc::ConnectionError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for PollErr {}

#[derive(Debug, thiserror::Error)]
#[error("Request timed out: {msg}")]
pub struct TimeoutErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for TimeoutErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to publish message: {source}")]
pub struct PublishErr {
    pub source: rumqttc::ClientError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for PublishErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to subscribe: {source}")]
pub struct SubscribeErr {
    pub source: rumqttc::ClientError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SubscribeErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to unsubscribe: {source}")]
pub struct UnsubscribeErr {
    pub source: rumqttc::ClientError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for UnsubscribeErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to disconnect: {source}")]
pub struct DisconnectErr {
    pub source: rumqttc::ClientError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DisconnectErr {
    fn is_network_connection_error(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Serialization error: {source}")]
pub struct SerdeErr {
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SerdeErr {}

#[derive(Debug, thiserror::Error)]
#[error("Mock MQTT error (is authentication error: {is_authentication_error}, is network connection error: {is_network_connection_error})")]
pub struct MockErr {
    pub is_authentication_error: bool,
    pub is_network_connection_error: bool,
}

impl crate::errors::Error for MockErr {
    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MQTTError {
    #[error(transparent)]
    AuthenticationErr(AuthenticationErr),
    #[error(transparent)]
    NetworkConnectionErr(NetworkConnectionErr),
    #[error(transparent)]
    TimeoutErr(TimeoutErr),
    #[error(transparent)]
    PollErr(PollErr),
    #[error(transparent)]
    PublishErr(PublishErr),
    #[error(transparent)]
    SubscribeErr(SubscribeErr),
    #[error(transparent)]
    UnsubscribeErr(UnsubscribeErr),
    #[error(transparent)]
    DisconnectErr(DisconnectErr),
    #[error(transparent)]
    SerdeErr(SerdeErr),
    #[error(transparent)]
    MockErr(MockErr),
}

crate::impl_error!(MQTTError {
    AuthenticationErr,
    NetworkConnectionErr,
    TimeoutErr,
    PollErr,
    PublishErr,
    SubscribeErr,
    UnsubscribeErr,
    DisconnectErr,
    SerdeErr,
    MockErr,
});

impl MQTTError {
    pub fn is_authentication_error(&self) -> bool {
        match self {
            MQTTError::AuthenticationErr(_) => true,
            MQTTError::MockErr(e) => e.is_authentication_error,
            _ => false,
        }
    }
}
