// internal crates
use miru_agent::mqtt::device;
use miru_agent::mqtt::errors::{MQTTError, MockErr};

// test helpers
use super::mock::{MockCall, MockClient};

// external crates
use rumqttc::QoS;

fn mock_error() -> MQTTError {
    MQTTError::MockErr(MockErr {
        is_authentication_error: false,
        is_network_connection_error: false,
    })
}

mod subscribe_sync {
    use super::*;

    #[tokio::test]
    async fn happy_path() {
        let client = MockClient::default();
        device::subscribe_sync(&client, "dvc_123").await.unwrap();

        let calls = client.get_calls();
        assert_eq!(calls.len(), 1);
        assert!(matches!(
            &calls[0],
            MockCall::Subscribe { topic, qos }
                if topic == "cmd/devices/dvc_123/sync" && *qos == QoS::AtLeastOnce
        ));
    }

    #[tokio::test]
    async fn error_propagation() {
        let client = MockClient {
            subscribe_fn: Box::new(|| Err(mock_error())),
            ..Default::default()
        };
        let result = device::subscribe_sync(&client, "dvc_123").await;
        assert!(result.is_err());
    }
}

mod publish_sync {
    use super::*;
    use miru_agent::mqtt::device::SyncDevice;

    #[tokio::test]
    async fn happy_path() {
        let client = MockClient::default();
        device::publish_sync(&client, "dvc_123").await.unwrap();

        let calls = client.get_calls();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            MockCall::Publish {
                topic,
                qos,
                retained,
                payload,
            } => {
                assert_eq!(topic, "cmd/devices/dvc_123/sync");
                assert_eq!(*qos, QoS::AtLeastOnce);
                assert!(*retained);
                let sync: SyncDevice = serde_json::from_slice(payload).unwrap();
                assert!(sync.is_synced);
            }
            other => panic!("expected Publish, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_propagation() {
        let client = MockClient {
            publish_fn: Box::new(|| Err(mock_error())),
            ..Default::default()
        };
        let result = device::publish_sync(&client, "dvc_123").await;
        assert!(result.is_err());
    }
}

mod subscribe_ping {
    use super::*;

    #[tokio::test]
    async fn happy_path() {
        let client = MockClient::default();
        device::subscribe_ping(&client, "dvc_123").await.unwrap();

        let calls = client.get_calls();
        assert_eq!(calls.len(), 1);
        assert!(matches!(
            &calls[0],
            MockCall::Subscribe { topic, qos }
                if topic == "v1/cmd/devices/dvc_123/ping" && *qos == QoS::AtLeastOnce
        ));
    }

    #[tokio::test]
    async fn error_propagation() {
        let client = MockClient {
            subscribe_fn: Box::new(|| Err(mock_error())),
            ..Default::default()
        };
        let result = device::subscribe_ping(&client, "dvc_123").await;
        assert!(result.is_err());
    }
}

mod publish_pong {
    use super::*;
    use miru_agent::mqtt::device::Pong;

    #[tokio::test]
    async fn happy_path() {
        let client = MockClient::default();
        let message_id = "msg-123".to_string();
        device::publish_pong(&client, "dvc_123", message_id.clone())
            .await
            .unwrap();

        let calls = client.get_calls();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            MockCall::Publish {
                topic,
                qos,
                retained,
                payload,
            } => {
                assert_eq!(topic, "v1/resp/devices/dvc_123/pong");
                assert_eq!(*qos, QoS::AtLeastOnce);
                assert!(!*retained);
                let pong: Pong = serde_json::from_slice(payload).unwrap();
                assert_eq!(pong.message_id, "msg-123");
                // timestamp should be valid RFC3339
                chrono::DateTime::parse_from_rfc3339(&pong.timestamp).unwrap();
            }
            other => panic!("expected Publish, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_propagation() {
        let client = MockClient {
            publish_fn: Box::new(|| Err(mock_error())),
            ..Default::default()
        };
        let result = device::publish_pong(&client, "dvc_123", "msg-456".to_string()).await;
        assert!(result.is_err());
    }
}
