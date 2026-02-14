// standard crates
use std::time::Duration;

// internal crates
use crate::mqtt::errors::*;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, QoS, Transport};
use tokio::time::timeout;

// ================================== OPTIONS ====================================== //
#[derive(Debug, Clone)]
pub enum Protocol {
    TCP,
    SSL,
}

#[derive(Debug, Clone)]
pub struct ConnectAddress {
    pub protocol: Protocol,
    pub broker: String,
    pub port: u16,
}

impl ConnectAddress {
    pub fn new(protocol: Protocol, broker: String, port: u16) -> Self {
        Self {
            protocol,
            broker,
            port,
        }
    }
}

impl Default for ConnectAddress {
    fn default() -> Self {
        Self {
            protocol: Protocol::SSL,
            broker: "mqtt.mirurobotics.com".to_string(),
            port: 8883,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl Credentials {
    pub fn new(username: String, password: String) -> Self {
        Self { username, password }
    }
}

impl Default for Credentials {
    fn default() -> Self {
        Self {
            username: "miru-agent".to_string(),
            password: "miru-agent-password".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub publish: Duration,
    pub subscribe: Duration,
    pub unsubscribe: Duration,
    pub disconnect: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            publish: Duration::from_secs(3),
            subscribe: Duration::from_secs(3),
            unsubscribe: Duration::from_secs(3),
            disconnect: Duration::from_secs(3),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub connect_address: ConnectAddress,
    pub credentials: Credentials,
    pub client_id: String,
    pub keep_alive: Duration,
    pub timeouts: Timeouts,
    pub capacity: usize,
}

impl Options {
    pub fn new(
        connect_address: ConnectAddress,
        credentials: Credentials,
        client_id: String,
        keep_alive: Duration,
        timeouts: Timeouts,
        capacity: usize,
    ) -> Self {
        Self {
            connect_address,
            credentials,
            client_id,
            keep_alive,
            timeouts,
            capacity,
        }
    }

    pub fn set_password(&mut self, password: String) {
        self.credentials.password = password;
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new(
            ConnectAddress::default(),
            Credentials::default(),
            "miru-agent".to_string(),
            Duration::from_secs(60),
            Timeouts::default(),
            64,
        )
    }
}

pub struct OptionsBuilder {
    options: Options,
}

impl OptionsBuilder {
    pub fn new(credentials: Credentials) -> Self {
        Self {
            options: Options {
                connect_address: ConnectAddress::default(),
                client_id: credentials.username.clone(),
                credentials,
                keep_alive: Duration::from_secs(20),
                timeouts: Timeouts::default(),
                capacity: 64,
            },
        }
    }

    pub fn with_connect_address(mut self, connect_address: ConnectAddress) -> Self {
        self.options.connect_address = connect_address;
        self
    }

    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.options.credentials = credentials;
        self
    }

    pub fn with_client_id(mut self, client_id: String) -> Self {
        self.options.client_id = client_id;
        self
    }

    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.options.timeouts = timeouts;
        self
    }

    pub fn build(self) -> Options {
        self.options
    }
}

// =================================== CLIENT ======================================= //
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
