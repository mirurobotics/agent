// internal crates
use miru_agent::errors::Error;
use miru_agent::mqtt::client::{
    poll, ConnectAddress, Credentials, MQTTClient, OptionsBuilder, Protocol,
};

// external crates
use rumqttc::QoS;

#[tokio::test]
async fn test_mqtt_client() {
    let username = "username";
    let password = "password";
    let options = OptionsBuilder::new(Credentials::new(username.to_string(), password.to_string()))
        .with_connect_address(ConnectAddress {
            protocol: Protocol::TCP,
            broker: "broker.emqx.io".to_string(),
            port: 1883,
        })
        .build();

    // create the client and subscribe to the device sync topic
    let (client, mut eventloop) = MQTTClient::new(&options).await;

    let topic = "a/unique/topic/string/for/miru";

    client.subscribe(topic, QoS::AtLeastOnce).await.unwrap();

    let payload = "test";
    client
        .publish(topic, QoS::AtLeastOnce, false, payload.as_bytes())
        .await
        .unwrap();

    // wait for an event
    poll(&mut eventloop).await.unwrap();

    client.unsubscribe(topic).await.unwrap();

    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn invalid_broker_url() {
    let credentials = Credentials::new("test".to_string(), "test".to_string());
    let options = OptionsBuilder::new(credentials)
        .with_connect_address(ConnectAddress {
            protocol: Protocol::TCP,
            broker: "arglebargle.com".to_string(),
            port: 1883,
        })
        .build();

    // create the client and subscribe to the device sync topic
    let (_, mut eventloop) = MQTTClient::new(&options).await;

    let event = poll(&mut eventloop).await.unwrap_err();
    assert!(event.is_network_connection_error());
}

#[tokio::test]
async fn invalid_username_or_password() {
    let credentials = Credentials::new("username".to_string(), "password".to_string());
    let options = OptionsBuilder::new(credentials)
        .with_connect_address(ConnectAddress {
            protocol: Protocol::SSL,
            broker: "staging.mqtt.mirurobotics.com".to_string(),
            port: 8883,
        })
        .build();

    // create the client and subscribe to the device sync topic
    let (_, mut eventloop) = MQTTClient::new(&options).await;

    let event = poll(&mut eventloop).await.unwrap_err();
    assert!(!event.is_network_connection_error());
}
