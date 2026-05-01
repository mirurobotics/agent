// standard crates
use std::time::Duration;

// internal crates
use crate::mqtt::errors::InvalidConnectAddressErr;
use crate::network::{is_loopback_host, MqttHost};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Protocol {
    TCP,
    SSL,
}

/// An MQTT connect address.
///
/// Host validity is enforced by the [`MqttHost`] type: a `ConnectAddress`
/// always carries a loopback or allowed-domain broker host. The
/// SSL-unless-loopback rule (non-loopback brokers must use `Protocol::SSL`)
/// is enforced by [`ConnectAddress::new`]. The fields remain `pub` for
/// ergonomic construction in tests; the SSL rule is therefore only enforced
/// at the constructor, which is acceptable because the rule is a soft
/// preference for the production environment and tests routinely use
/// `Protocol::TCP` with loopback brokers.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ConnectAddress {
    pub protocol: Protocol,
    pub broker: MqttHost,
    pub port: u16,
}

impl Default for ConnectAddress {
    fn default() -> Self {
        Self {
            protocol: Protocol::SSL,
            broker: MqttHost::default(),
            port: 8883,
        }
    }
}

impl ConnectAddress {
    /// Constructs a `ConnectAddress`, enforcing the SSL-unless-loopback rule.
    /// Host validity is already guaranteed by the `MqttHost` type.
    pub fn new(
        broker: MqttHost,
        protocol: Protocol,
        port: u16,
    ) -> Result<Self, InvalidConnectAddressErr> {
        if !is_loopback_host(broker.as_str()) && !matches!(protocol, Protocol::SSL) {
            return Err(InvalidConnectAddressErr {
                msg: format!(
                    "non-loopback broker `{}` requires Protocol::SSL",
                    broker.as_str()
                ),
                trace: crate::trace!(),
            });
        }
        Ok(Self {
            protocol,
            broker,
            port,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl Default for Credentials {
    fn default() -> Self {
        Self {
            username: "miru-agent".to_string(),
            password: "miru-agent-password".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Options {
    pub connect_address: ConnectAddress,
    pub credentials: Credentials,
    pub client_id: String,
    pub keep_alive: Duration,
    pub timeouts: Timeouts,
    pub capacity: usize,
}

impl Options {
    pub fn new(credentials: Credentials) -> Self {
        Self {
            client_id: credentials.username.clone(),
            credentials,
            connect_address: ConnectAddress::default(),
            keep_alive: Duration::from_secs(20),
            timeouts: Timeouts::default(),
            capacity: 64,
        }
    }

    pub fn with_connect_address(mut self, connect_address: ConnectAddress) -> Self {
        self.connect_address = connect_address;
        self
    }

    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = credentials;
        self
    }

    pub fn with_client_id(mut self, client_id: String) -> Self {
        self.client_id = client_id;
        self
    }

    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub fn set_password(&mut self, password: String) {
        self.credentials.password = password;
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new(Credentials::default())
    }
}
