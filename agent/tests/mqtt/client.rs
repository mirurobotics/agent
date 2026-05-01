// standard crates
use std::collections::HashMap;
use std::time::Duration;

// internal crates
use crate::mocks::mqtt_client as mock;
use miru_agent::errors::Error;
use miru_agent::mqtt::client::{poll, Publish};
use miru_agent::mqtt::options::{ConnectAddress, Credentials, Options, Protocol, Timeouts};
use miru_agent::mqtt::{Client, ClientI, MQTTError};
use miru_agent::storage::validation::MqttHost;

// external crates
use rumqttc::QoS;

#[tokio::test]
async fn test_mqtt_client() {
    let mut auth = HashMap::new();
    auth.insert("test_user".to_string(), "test_pass".to_string());
    let _broker = mock::run_broker(18831, Some(auth));

    let options = Options::new(Credentials {
        username: "test_user".to_string(),
        password: "test_pass".to_string(),
    })
    .with_connect_address(ConnectAddress {
        protocol: Protocol::TCP,
        broker: MqttHost::new("127.0.0.1").unwrap(),
        port: 18831,
    });

    // create the client and subscribe to the device sync topic
    let (client, mut eventloop) = Client::new(&options).await;

    let topic = "a/unique/topic/string/for/miru";

    client.subscribe(topic, QoS::AtLeastOnce).await.unwrap();

    let payload = "test";
    client
        .publish(Publish {
            topic,
            qos: QoS::AtLeastOnce,
            retained: false,
            payload: payload.as_bytes(),
        })
        .await
        .unwrap();

    // wait for an event
    poll(&mut eventloop).await.unwrap();

    client.unsubscribe(topic).await.unwrap();

    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn invalid_broker_url() {
    let options = Options::new(Credentials {
        username: "test".to_string(),
        password: "test".to_string(),
    })
    .with_connect_address(ConnectAddress {
        protocol: Protocol::TCP,
        // Loopback on a port nothing's listening on — exercises the network
        // connection error path without needing an external unreachable IP
        // (which the `MqttHost` newtype would reject anyway).
        broker: MqttHost::new("127.0.0.1").unwrap(),
        port: 1,
    });

    // create the client and subscribe to the device sync topic
    let (_, mut eventloop) = Client::new(&options).await;

    let err = poll(&mut eventloop).await.unwrap_err();
    assert!(matches!(err, MQTTError::NetworkConnectionErr(_)));
    assert!(err.is_network_conn_err());
}

#[tokio::test]
async fn invalid_username_or_password() {
    // rumqttd has a protocol violation: it drops the TCP connection on auth failure
    // instead of sending a CONNACK refusal. We use a minimal TCP mock that sends
    // the correct CONNACK with BadUserNamePassword (0x04) to test this path locally.
    let _handle = mock::run_rejecting_broker(18832, 0x04).await;

    let options = Options::new(Credentials {
        username: "wrong_user".to_string(),
        password: "wrong_pass".to_string(),
    })
    .with_connect_address(ConnectAddress {
        protocol: Protocol::TCP,
        broker: MqttHost::new("127.0.0.1").unwrap(),
        port: 18832,
    });

    let (_, mut eventloop) = Client::new(&options).await;

    let err = poll(&mut eventloop).await.unwrap_err();
    assert!(matches!(err, MQTTError::AuthenticationErr(_)));
    assert!(err.is_authentication_error());
}

// No broker needed for the tests below — they exercise client-side error paths only.
fn mqtt_options() -> Options {
    Options::new(Credentials {
        username: "test".to_string(),
        password: "test".to_string(),
    })
    .with_connect_address(ConnectAddress {
        protocol: Protocol::TCP,
        broker: MqttHost::new("127.0.0.1").unwrap(),
        port: 1,
    })
}

#[tokio::test]
async fn publish_err() {
    let (client, eventloop) = Client::new(&mqtt_options()).await;
    drop(eventloop);

    let err = client
        .publish(Publish {
            topic: "test",
            qos: QoS::AtLeastOnce,
            retained: false,
            payload: b"test",
        })
        .await
        .unwrap_err();
    assert!(matches!(err, MQTTError::PublishErr(_)));
    assert!(err.is_network_conn_err());
}

#[tokio::test]
async fn subscribe_err() {
    let (client, eventloop) = Client::new(&mqtt_options()).await;
    drop(eventloop);

    let err = client
        .subscribe("test", QoS::AtLeastOnce)
        .await
        .unwrap_err();
    assert!(matches!(err, MQTTError::SubscribeErr(_)));
    assert!(err.is_network_conn_err());
}

#[tokio::test]
async fn unsubscribe_err() {
    let (client, eventloop) = Client::new(&mqtt_options()).await;
    drop(eventloop);

    let err = client.unsubscribe("test").await.unwrap_err();
    assert!(matches!(err, MQTTError::UnsubscribeErr(_)));
    assert!(err.is_network_conn_err());
}

#[tokio::test]
async fn disconnect_err() {
    let (client, eventloop) = Client::new(&mqtt_options()).await;
    drop(eventloop);

    let err = client.disconnect().await.unwrap_err();
    assert!(matches!(err, MQTTError::DisconnectErr(_)));
    assert!(err.is_network_conn_err());
}

#[tokio::test]
async fn publish_timeout() {
    let mut options = mqtt_options().with_timeouts(Timeouts {
        publish: Duration::from_millis(10),
        ..Timeouts::default()
    });
    options.capacity = 1;

    let (client, _eventloop) = Client::new(&options).await;

    // First publish fills the capacity-1 channel
    client
        .publish(Publish {
            topic: "test",
            qos: QoS::AtLeastOnce,
            retained: false,
            payload: b"test",
        })
        .await
        .unwrap();

    // Second publish blocks (channel full) and times out
    let err = client
        .publish(Publish {
            topic: "test",
            qos: QoS::AtLeastOnce,
            retained: false,
            payload: b"test",
        })
        .await
        .unwrap_err();
    assert!(matches!(err, MQTTError::TimeoutErr(_)));
    assert!(err.is_network_conn_err());
}
