// standard crates
use std::time::Duration;

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
