// internal crates
use miru_agent::mqtt::device;
use miru_agent::mqtt::errors::MockErr;
use miru_agent::mqtt::MQTTError;

// test helpers
use super::mock::{MockCall, MockClient};

// external crates
use rumqttc::QoS;

fn mock_error() -> MQTTError {
    MQTTError::MockErr(MockErr {
        is_authentication_error: false,
        is_network_conn_err: false,
    })
}

mod subscribe_all {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn happy_path() {
        let client = MockClient::default();
        device::subscribe_all(&client, "dvc_123").await.unwrap();

        let calls = client.get_calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(
            &calls[0],
            MockCall::Subscribe { topic, qos }
                if topic == "cmd/devices/dvc_123/sync" && *qos == QoS::AtLeastOnce
        ));
        assert!(matches!(
            &calls[1],
            MockCall::Subscribe { topic, qos }
                if topic == "v1/cmd/devices/dvc_123/ping" && *qos == QoS::AtLeastOnce
        ));
    }

    #[tokio::test]
    async fn subscribe_sync_error_short_circuits() {
        let client = MockClient {
            subscribe_fn: Box::new(|| Err(Box::new(mock_error()))),
            ..Default::default()
        };
        let result = device::subscribe_all(&client, "dvc_123").await;
        assert!(result.is_err());

        // The `?` after subscribe_sync must short-circuit before subscribe_ping,
        // so only the sync subscribe was attempted.
        let calls = client.get_calls();
        assert_eq!(calls.len(), 1);
        assert!(matches!(
            &calls[0],
            MockCall::Subscribe { topic, .. } if topic == "cmd/devices/dvc_123/sync"
        ));
    }

    #[tokio::test]
    async fn subscribe_ping_error_propagates() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_fn = counter.clone();
        let client = MockClient {
            subscribe_fn: Box::new(move || {
                if counter_for_fn.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(Box::new(mock_error()))
                }
            }),
            ..Default::default()
        };
        let result = device::subscribe_all(&client, "dvc_123").await;
        assert!(result.is_err());

        // sync succeeded, ping was attempted and failed — both calls recorded.
        let calls = client.get_calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(
            &calls[1],
            MockCall::Subscribe { topic, .. } if topic == "v1/cmd/devices/dvc_123/ping"
        ));
    }
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
            subscribe_fn: Box::new(|| Err(Box::new(mock_error()))),
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
            publish_fn: Box::new(|| Err(Box::new(mock_error()))),
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
            subscribe_fn: Box::new(|| Err(Box::new(mock_error()))),
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
            publish_fn: Box::new(|| Err(Box::new(mock_error()))),
            ..Default::default()
        };
        let result = device::publish_pong(&client, "dvc_123", "msg-456".to_string()).await;
        assert!(result.is_err());
    }
}
