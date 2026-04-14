// standard crates
use std::time::Duration;

// internal crates
use miru_agent::mqtt::options::{ConnectAddress, Credentials, Options, Protocol, Timeouts};

mod connect_address {
    use super::*;

    #[test]
    fn default() {
        let addr = ConnectAddress::default();
        assert!(matches!(addr.protocol, Protocol::SSL));
        assert_eq!(addr.broker, "mqtt.mirurobotics.com");
        assert_eq!(addr.port, 8883);
    }
}

mod credentials {
    use super::*;

    #[test]
    fn default() {
        let creds = Credentials::default();
        assert_eq!(creds.username, "miru-agent");
        assert_eq!(creds.password, "miru-agent-password");
    }
}

mod timeouts {
    use super::*;

    #[test]
    fn default() {
        let actual = Timeouts::default();
        let expected = Timeouts {
            publish: Duration::from_secs(3),
            subscribe: Duration::from_secs(3),
            unsubscribe: Duration::from_secs(3),
            disconnect: Duration::from_secs(3),
        };
        assert_eq!(actual, expected);
    }
}

mod opts {
    use super::*;

    #[test]
    fn new_defaults() {
        let creds = Credentials {
            username: "user".to_string(),
            password: "pass".to_string(),
        };
        let actual = Options::new(creds.clone());
        let expected = Options {
            credentials: creds,
            client_id: "user".to_string(),
            connect_address: ConnectAddress::default(),
            keep_alive: Duration::from_secs(20),
            timeouts: Timeouts::default(),
            capacity: 64,
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn default() {
        let actual = Options::default();
        let expected = Options {
            credentials: Credentials::default(),
            client_id: "miru-agent".to_string(),
            connect_address: ConnectAddress::default(),
            keep_alive: Duration::from_secs(20),
            timeouts: Timeouts::default(),
            capacity: 64,
        };
        assert!(matches!(actual.connect_address.protocol, Protocol::SSL));
        assert_eq!(actual, expected);
    }

    #[test]
    fn set_password() {
        let mut opts = Options::default();
        opts.set_password("new-password".to_string());
        assert_eq!(opts.credentials.password, "new-password");
    }

    #[test]
    fn with_connect_address() {
        let addr = ConnectAddress {
            protocol: Protocol::TCP,
            broker: "local".to_string(),
            port: 1883,
        };
        let opts = Options::default().with_connect_address(addr);
        assert!(matches!(opts.connect_address.protocol, Protocol::TCP));
        assert_eq!(opts.connect_address.broker, "local");
        assert_eq!(opts.connect_address.port, 1883);
    }

    #[test]
    fn with_credentials() {
        let new_creds = Credentials {
            username: "other".to_string(),
            password: "secret".to_string(),
        };
        let opts = Options::default().with_credentials(new_creds);
        assert_eq!(opts.credentials.username, "other");
        assert_eq!(opts.credentials.password, "secret");
    }

    #[test]
    fn with_client_id() {
        let opts = Options::default().with_client_id("custom-id".to_string());
        assert_eq!(opts.client_id, "custom-id");
    }

    #[test]
    fn with_timeouts() {
        let timeouts = Timeouts {
            publish: Duration::from_secs(10),
            subscribe: Duration::from_secs(10),
            unsubscribe: Duration::from_secs(10),
            disconnect: Duration::from_secs(10),
        };
        let opts = Options::default().with_timeouts(timeouts);
        assert_eq!(opts.timeouts.publish, Duration::from_secs(10));
    }
}
