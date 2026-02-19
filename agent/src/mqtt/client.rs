// internal crates
use crate::mqtt::errors::*;
use crate::mqtt::options::{Options, Protocol, Timeouts};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, QoS, Transport};
use tokio::time::timeout;

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

    pub async fn publish(
        &self,
        topic: &str,
        qos: QoS,
        retained: bool,
        payload: &[u8],
    ) -> Result<(), MQTTError> {
        timeout(
            self.timeouts.publish,
            self.client.publish(topic, qos, retained, payload),
        )
        .await
        .map_err(|_| {
            MQTTError::TimeoutErr(TimeoutErr {
                msg: "Publish timeout".to_string(),
                trace: trace!(),
            })
        })?
        .map_err(|e| {
            MQTTError::PublishErr(PublishErr {
                source: e,
                trace: trace!(),
            })
        })?;

        Ok(())
    }

    pub async fn subscribe(&self, topic: &str, qos: QoS) -> Result<(), MQTTError> {
        timeout(self.timeouts.subscribe, self.client.subscribe(topic, qos))
            .await
            .map_err(|_| {
                MQTTError::TimeoutErr(TimeoutErr {
                    msg: "Subscribe timeout".to_string(),
                    trace: trace!(),
                })
            })?
            .map_err(|e| {
                MQTTError::PublishErr(PublishErr {
                    source: e,
                    trace: trace!(),
                })
            })?;

        Ok(())
    }

    pub async fn unsubscribe(&self, topic: &str) -> Result<(), MQTTError> {
        timeout(self.timeouts.unsubscribe, self.client.unsubscribe(topic))
            .await
            .map_err(|_| {
                MQTTError::TimeoutErr(TimeoutErr {
                    msg: "Unsubscribe timeout".to_string(),
                    trace: trace!(),
                })
            })?
            .map_err(|e| {
                MQTTError::PublishErr(PublishErr {
                    source: e,
                    trace: trace!(),
                })
            })?;

        Ok(())
    }

    pub async fn disconnect(&self) -> Result<(), MQTTError> {
        timeout(self.timeouts.disconnect, self.client.disconnect())
            .await
            .map_err(|_| {
                MQTTError::TimeoutErr(TimeoutErr {
                    msg: "Disconnect timeout".to_string(),
                    trace: trace!(),
                })
            })?
            .map_err(|e| {
                MQTTError::PublishErr(PublishErr {
                    source: e,
                    trace: trace!(),
                })
            })?;

        Ok(())
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
