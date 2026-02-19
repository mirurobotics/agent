// standard library
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::mqtt::errors::*;
use crate::mqtt::options::{Options, Protocol, Timeouts};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, QoS, Transport};

pub struct Publish<'a> {
    pub topic: &'a str,
    pub qos: QoS,
    pub retained: bool,
    pub payload: &'a [u8],
}

pub trait ClientI: Send + Sync {
    fn publish(&self, msg: Publish<'_>) -> impl Future<Output = Result<(), MQTTError>> + Send;

    fn subscribe(
        &self,
        topic: &str,
        qos: QoS,
    ) -> impl Future<Output = Result<(), MQTTError>> + Send;

    fn unsubscribe(&self, topic: &str) -> impl Future<Output = Result<(), MQTTError>> + Send;

    fn disconnect(&self) -> impl Future<Output = Result<(), MQTTError>> + Send;
}

pub struct MQTTClient {
    pub created_at: DateTime<Utc>,
    pub(crate) client: AsyncClient,
    pub(crate) timeouts: Timeouts,
}

impl MQTTClient {
    pub async fn new(options: &Options) -> (Self, EventLoop) {
        let mut mqtt_options = MqttOptions::new(
            &options.client_id,
            &options.connect_address.broker,
            options.connect_address.port,
        );

        mqtt_options.set_keep_alive(options.keep_alive);
        mqtt_options.set_credentials(&options.credentials.username, &options.credentials.password);

        match options.connect_address.protocol {
            Protocol::TCP => {
                mqtt_options.set_transport(Transport::Tcp);
            }
            Protocol::SSL => {
                mqtt_options.set_transport(Transport::Tls(Default::default()));
            }
        }

        let (client, eventloop) = AsyncClient::new(mqtt_options, options.capacity);

        (
            Self {
                created_at: Utc::now(),
                client,
                timeouts: options.timeouts,
            },
            eventloop,
        )
    }
}

impl ClientI for MQTTClient {
    async fn publish(&self, msg: Publish<'_>) -> Result<(), MQTTError> {
        with_timeout(
            self.timeouts.publish,
            self.client
                .publish(msg.topic, msg.qos, msg.retained, msg.payload),
            "Publish timeout",
            |e| {
                MQTTError::PublishErr(PublishErr {
                    source: e,
                    trace: trace!(),
                })
            },
        )
        .await
    }

    async fn subscribe(&self, topic: &str, qos: QoS) -> Result<(), MQTTError> {
        with_timeout(
            self.timeouts.subscribe,
            self.client.subscribe(topic, qos),
            "Subscribe timeout",
            |e| {
                MQTTError::SubscribeErr(SubscribeErr {
                    source: e,
                    trace: trace!(),
                })
            },
        )
        .await
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), MQTTError> {
        with_timeout(
            self.timeouts.unsubscribe,
            self.client.unsubscribe(topic),
            "Unsubscribe timeout",
            |e| {
                MQTTError::UnsubscribeErr(UnsubscribeErr {
                    source: e,
                    trace: trace!(),
                })
            },
        )
        .await
    }

    async fn disconnect(&self) -> Result<(), MQTTError> {
        with_timeout(
            self.timeouts.disconnect,
            self.client.disconnect(),
            "Disconnect timeout",
            |e| {
                MQTTError::DisconnectErr(DisconnectErr {
                    source: e,
                    trace: trace!(),
                })
            },
        )
        .await
    }
}

pub async fn poll(eventloop: &mut EventLoop) -> Result<Event, MQTTError> {
    eventloop.poll().await.map_err(|e| {
        match e {
            // poor network connection errors
            rumqttc::ConnectionError::NetworkTimeout
            | rumqttc::ConnectionError::Io(_)
            | rumqttc::ConnectionError::MqttState(rumqttc::StateError::AwaitPingResp)
            | rumqttc::ConnectionError::FlushTimeout
            | rumqttc::ConnectionError::NotConnAck(_) => {
                MQTTError::NetworkConnectionErr(NetworkConnectionErr {
                    source: e,
                    trace: trace!(),
                })
            }

            // mqtt broker rejected the authentication
            rumqttc::ConnectionError::ConnectionRefused(_) => {
                MQTTError::AuthenticationErr(AuthenticationErr {
                    source: e,
                    trace: trace!(),
                })
            }

            // all other errors
            _ => MQTTError::PollErr(PollErr {
                source: e,
                trace: trace!(),
            }),
        }
    })
}

async fn with_timeout<F>(
    duration: Duration,
    future: F,
    timeout_msg: &str,
    map_err: impl FnOnce(rumqttc::ClientError) -> MQTTError,
) -> Result<(), MQTTError>
where
    F: Future<Output = Result<(), rumqttc::ClientError>>,
{
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| {
            MQTTError::TimeoutErr(TimeoutErr {
                msg: timeout_msg.to_string(),
                trace: trace!(),
            })
        })?
        .map_err(map_err)
}
