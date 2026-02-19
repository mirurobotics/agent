// internal crates
use miru_agent::errors::Error;
use miru_agent::mqtt::errors::*;

// external crates
use rumqttc::{ConnectReturnCode, ConnectionError, Disconnect, Request};

fn trace() -> Box<miru_agent::errors::Trace> {
    miru_agent::trace!()
}

fn client_error() -> rumqttc::ClientError {
    rumqttc::ClientError::Request(Request::Disconnect(Disconnect))
}

fn connection_error_network() -> ConnectionError {
    ConnectionError::NetworkTimeout
}

fn connection_error_auth() -> ConnectionError {
    ConnectionError::ConnectionRefused(ConnectReturnCode::BadUserNamePassword)
}

mod is_network_connection_error {
    use super::*;

    #[test]
    fn authentication_err_returns_false() {
        let err = MQTTError::AuthenticationErr(AuthenticationErr {
            source: connection_error_auth(),
            trace: trace(),
        });
        assert!(!err.is_network_connection_error());
    }

    #[test]
    fn network_connection_err_returns_true() {
        let err = MQTTError::NetworkConnectionErr(NetworkConnectionErr {
            source: connection_error_network(),
            trace: trace(),
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn timeout_err_returns_true() {
        let err = MQTTError::TimeoutErr(TimeoutErr {
            msg: "timed out".to_string(),
            trace: trace(),
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn poll_err_returns_false() {
        let err = MQTTError::PollErr(PollErr {
            source: ConnectionError::RequestsDone,
            trace: trace(),
        });
        assert!(!err.is_network_connection_error());
    }

    #[test]
    fn publish_err_returns_true() {
        let err = MQTTError::PublishErr(PublishErr {
            source: client_error(),
            trace: trace(),
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn subscribe_err_returns_true() {
        let err = MQTTError::SubscribeErr(SubscribeErr {
            source: client_error(),
            trace: trace(),
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn unsubscribe_err_returns_true() {
        let err = MQTTError::UnsubscribeErr(UnsubscribeErr {
            source: client_error(),
            trace: trace(),
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn disconnect_err_returns_true() {
        let err = MQTTError::DisconnectErr(DisconnectErr {
            source: client_error(),
            trace: trace(),
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn serde_err_returns_false() {
        let source: serde_json::Error = serde_json::from_str::<bool>("invalid").unwrap_err();
        let err = MQTTError::SerdeErr(SerdeErr {
            source,
            trace: trace(),
        });
        assert!(!err.is_network_connection_error());
    }

    #[test]
    fn mock_err_delegates_true() {
        let err = MQTTError::MockErr(MockErr {
            is_authentication_error: false,
            is_network_connection_error: true,
        });
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn mock_err_delegates_false() {
        let err = MQTTError::MockErr(MockErr {
            is_authentication_error: false,
            is_network_connection_error: false,
        });
        assert!(!err.is_network_connection_error());
    }
}

mod is_authentication_error {
    use super::*;

    #[test]
    fn authentication_err_returns_true() {
        let err = MQTTError::AuthenticationErr(AuthenticationErr {
            source: connection_error_auth(),
            trace: trace(),
        });
        assert!(err.is_authentication_error());
    }

    #[test]
    fn network_connection_err_returns_false() {
        let err = MQTTError::NetworkConnectionErr(NetworkConnectionErr {
            source: connection_error_network(),
            trace: trace(),
        });
        assert!(!err.is_authentication_error());
    }

    #[test]
    fn timeout_err_returns_false() {
        let err = MQTTError::TimeoutErr(TimeoutErr {
            msg: "timed out".to_string(),
            trace: trace(),
        });
        assert!(!err.is_authentication_error());
    }

    #[test]
    fn publish_err_returns_false() {
        let err = MQTTError::PublishErr(PublishErr {
            source: client_error(),
            trace: trace(),
        });
        assert!(!err.is_authentication_error());
    }

    #[test]
    fn mock_err_delegates_true() {
        let err = MQTTError::MockErr(MockErr {
            is_authentication_error: true,
            is_network_connection_error: false,
        });
        assert!(err.is_authentication_error());
    }

    #[test]
    fn mock_err_delegates_false() {
        let err = MQTTError::MockErr(MockErr {
            is_authentication_error: false,
            is_network_connection_error: false,
        });
        assert!(!err.is_authentication_error());
    }
}

mod display {
    use super::*;

    #[test]
    fn authentication_err() {
        let err = AuthenticationErr {
            source: connection_error_auth(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("authenticate"));
    }

    #[test]
    fn network_connection_err() {
        let err = NetworkConnectionErr {
            source: connection_error_network(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("connect"));
    }

    #[test]
    fn poll_err() {
        let err = PollErr {
            source: ConnectionError::RequestsDone,
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("poll"));
    }

    #[test]
    fn timeout_err() {
        let err = TimeoutErr {
            msg: "Subscribe timeout".to_string(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("Subscribe timeout"));
    }

    #[test]
    fn publish_err() {
        let err = PublishErr {
            source: client_error(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("publish"));
    }

    #[test]
    fn subscribe_err() {
        let err = SubscribeErr {
            source: client_error(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("subscribe"));
    }

    #[test]
    fn unsubscribe_err() {
        let err = UnsubscribeErr {
            source: client_error(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("unsubscribe"));
    }

    #[test]
    fn disconnect_err() {
        let err = DisconnectErr {
            source: client_error(),
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("disconnect"));
    }

    #[test]
    fn serde_err() {
        let source: serde_json::Error = serde_json::from_str::<bool>("invalid").unwrap_err();
        let err = SerdeErr {
            source,
            trace: trace(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("erialization"));
    }

    #[test]
    fn mock_err() {
        let err = MockErr {
            is_authentication_error: true,
            is_network_connection_error: false,
        };
        let msg = format!("{err}");
        assert!(msg.contains("true"));
        assert!(msg.contains("false"));
    }

    #[test]
    fn mqtt_error_is_transparent() {
        let inner = TimeoutErr {
            msg: "Publish timeout".to_string(),
            trace: trace(),
        };
        let inner_msg = format!("{inner}");
        let err = MQTTError::TimeoutErr(inner);
        let outer_msg = format!("{err}");
        assert_eq!(inner_msg, outer_msg);
    }
}

mod poll_error_classification {
    use super::*;
    use miru_agent::mqtt::client::poll;
    use miru_agent::mqtt::options::{ConnectAddress, Credentials, Options, Protocol};

    // poll() classifies ConnectionError variants into MQTTError types.
    // We can't inject errors into an EventLoop directly, but we can
    // verify the classification indirectly through real connection attempts.

    #[tokio::test]
    async fn unreachable_host_is_network_connection_error() {
        // Connect to a port that refuses connections
        let opts = Options::new(Credentials {
            username: "test".to_string(),
            password: "test".to_string(),
        })
        .with_connect_address(ConnectAddress {
            protocol: Protocol::TCP,
            broker: "127.0.0.1".to_string(),
            port: 1, // unlikely to have anything listening
        });

        let (_, mut eventloop) = miru_agent::mqtt::client::MQTTClient::new(&opts).await;

        let err = poll(&mut eventloop).await.unwrap_err();
        assert!(err.is_network_connection_error());
    }
}

mod mqtt_client_new {
    use miru_agent::mqtt::client::MQTTClient;
    use miru_agent::mqtt::options::{Credentials, Options};

    #[tokio::test]
    async fn created_at_is_recent() {
        let before = chrono::Utc::now();
        let opts = Options::new(Credentials {
            username: "test".to_string(),
            password: "test".to_string(),
        });
        let (client, _eventloop) = MQTTClient::new(&opts).await;
        let after = chrono::Utc::now();

        assert!(client.created_at >= before);
        assert!(client.created_at <= after);
    }
}
