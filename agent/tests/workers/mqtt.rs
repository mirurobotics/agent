// internal crates
use miru_agent::authn::token::Token;
use miru_agent::filesys::dir::Dir;
use miru_agent::models::device::{Device, DeviceStatus};
use miru_agent::mqtt::{
    client::{MQTTClient, Options},
    device::{Ping, SyncDevice},
    errors::*,
    topics,
};
use miru_agent::storage::{device::DeviceFile, layout::StorageLayout};
use miru_agent::sync::{
    errors::{MockErr as SyncMockErr, SyncErr},
    syncer::{CooldownEnd, SyncEvent, SyncFailure},
};
use miru_agent::workers::mqtt::{self, handle_error, handle_event, handle_syncer_event};

use crate::authn::mock::MockTokenManager;
use crate::mqtt::mock::MockDeviceClient;
use crate::sync::mock::MockSyncer;

// external crates
use chrono::Utc;
use rumqttc::{ConnAck, ConnectReturnCode, Event, Incoming, Publish, QoS};

pub mod handle_syncer_event {
    use super::*;

    #[tokio::test]
    async fn sync_success_publishes_device_sync() {
        let event = SyncEvent::SyncSuccess;
        let mqtt_client = MockDeviceClient::default();
        handle_syncer_event(&event, "device_id", &mqtt_client).await;
        assert_eq!(mqtt_client.num_publish_device_sync_calls(), 1);
    }

    #[tokio::test]
    async fn ignored_syncer_events() {
        for event in [
            SyncEvent::SyncFailed(SyncFailure {
                is_network_connection_error: true,
            }),
            SyncEvent::CooldownEnd(CooldownEnd::FromSyncSuccess),
            SyncEvent::CooldownEnd(CooldownEnd::FromSyncFailure),
        ] {
            let mqtt_client = MockDeviceClient::default();
            handle_syncer_event(&event, "device_id", &mqtt_client).await;
            assert_eq!(mqtt_client.num_publish_device_sync_calls(), 0);
        }
    }
}

pub mod handle_connection_events {
    use super::*;

    #[tokio::test]
    async fn unsuccessful_connack_event_is_ignored() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), Device::default())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::ConnAck(ConnAck {
            code: ConnectReturnCode::RefusedProtocolVersion,
            session_present: false,
        }));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(syncer.num_sync_calls(), 0);
    }

    #[tokio::test]
    async fn successful_connack_event() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) = DeviceFile::spawn_with_default(
            64,
            layout.device_file(),
            Device {
                status: DeviceStatus::Offline,
                last_connected_at: Utc::now(),
                ..Device::default()
            },
        )
        .await
        .unwrap();

        let event = Event::Incoming(Incoming::ConnAck(ConnAck {
            code: ConnectReturnCode::Success,
            session_present: false,
        }));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let before_event = Utc::now();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file).await;
        assert_eq!(err_streak, 0);

        let device = device_file.read().await.unwrap();
        assert_eq!(device.status, DeviceStatus::Online);
        assert!(device.last_connected_at >= before_event);
        assert!(device.last_connected_at <= Utc::now());
    }

    #[tokio::test]
    async fn disconnect_event() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) = DeviceFile::spawn_with_default(
            64,
            layout.device_file(),
            Device {
                status: DeviceStatus::Online,
                last_disconnected_at: Utc::now(),
                ..Device::default()
            },
        )
        .await
        .unwrap();

        let event = Event::Incoming(Incoming::Disconnect);
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let before_event = Utc::now();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file).await;
        assert_eq!(err_streak, 0);

        let device = device_file.read().await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
        assert!(device.last_disconnected_at >= before_event);
        assert!(device.last_disconnected_at <= Utc::now());
    }
}

pub mod handle_sync_events {
    use super::*;

    #[tokio::test]
    async fn sync_request_unserializable() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device::default();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::Publish(Publish::new(
            topics::device_sync(&device.id),
            QoS::AtLeastOnce,
            "invalid".to_string(),
        )));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, &device.id, &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(syncer.num_sync_calls(), 1);
    }

    #[tokio::test]
    async fn sync_request_is_synced() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device::default();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let payload = SyncDevice { is_synced: true };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let event = Event::Incoming(Incoming::Publish(Publish::new(
            topics::device_sync(&device.id),
            QoS::AtLeastOnce,
            payload_bytes,
        )));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, &device.id, &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(syncer.num_sync_calls(), 0);
    }

    #[tokio::test]
    async fn sync_request_is_not_synced() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device::default();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let payload = SyncDevice { is_synced: false };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let event = Event::Incoming(Incoming::Publish(Publish::new(
            topics::device_sync(&device.id),
            QoS::AtLeastOnce,
            payload_bytes,
        )));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, &device.id, &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(syncer.num_sync_calls(), 1);
    }

    #[tokio::test]
    async fn sync_error() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device::default();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let payload = SyncDevice { is_synced: false };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let event = Event::Incoming(Incoming::Publish(Publish::new(
            topics::device_sync(&device.id),
            QoS::AtLeastOnce,
            payload_bytes,
        )));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        syncer.set_sync(|| {
            Err(SyncErr::MockErr(SyncMockErr {
                is_network_connection_error: false,
            }))
        });
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, &device.id, &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(syncer.num_sync_calls(), 1);
    }
}

pub mod handle_ping_events {
    use super::*;

    #[tokio::test]
    async fn ping_request_unserializable() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device::default();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::Publish(Publish::new(
            topics::device_ping(&device.id),
            QoS::AtLeastOnce,
            "invalid".to_string(),
        )));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, &device.id, &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(mqtt_client.num_publish_device_pong_calls(), 0);
    }

    #[tokio::test]
    async fn pong_success() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device::default();
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let payload = Ping {
            message_id: "123e4567-e89b-12d3-a456-426614174000".to_string(),
            timestamp: Utc::now().to_rfc3339(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let event = Event::Incoming(Incoming::Publish(Publish::new(
            topics::device_ping(&device.id),
            QoS::AtLeastOnce,
            payload_bytes,
        )));
        let mqtt_client = MockDeviceClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, &device.id, &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(mqtt_client.num_publish_device_pong_calls(), 1);
    }
}

pub mod handle_mqtt_error {
    use super::*;

    #[tokio::test]
    async fn authentication_error_triggers_token_refresh() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device {
            id: "device_id".to_string(),
            session_id: "device_session_id".to_string(),
            status: DeviceStatus::Offline,
            ..Device::default()
        };
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now(),
        };
        let token_mngr = MockTokenManager::new(token);
        let error = MQTTError::MockErr(MockErr {
            is_authentication_error: true,
            is_network_connection_error: false,
        });

        let options = Options::default();
        let (client, eventloop) = MQTTClient::new(&options).await;
        let created_at = client.created_at;

        let before_patch = Utc::now();
        let state = mqtt::State {
            client,
            eventloop,
            err_streak: 2,
        };
        let state = handle_error(
            state,
            error,
            &device,
            &token_mngr,
            &options.connect_address,
            &device_file,
        )
        .await;
        assert_eq!(token_mngr.num_refresh_token_calls(), 1);

        // should increment the error streak
        assert_eq!(state.err_streak, 3);

        // should reinitialize the mqtt client
        assert_ne!(state.client.created_at, created_at);

        // shouldn't update the last disconnected at time since it was already offline
        let device = device_file.read().await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
        assert!(device.last_disconnected_at <= before_patch);
    }

    #[tokio::test]
    async fn other_errors_are_ignored() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let device = Device {
            id: "device_id".to_string(),
            session_id: "device_session_id".to_string(),
            status: DeviceStatus::Online,
            ..Device::default()
        };
        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), device.clone())
                .await
                .unwrap();

        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now(),
        };
        let token_mngr = MockTokenManager::new(token);
        let error = MQTTError::MockErr(MockErr {
            is_authentication_error: false,
            is_network_connection_error: true,
        });

        let options = Options::default();
        let (client, eventloop) = MQTTClient::new(&options).await;
        let created_at = client.created_at;

        let before_patch = Utc::now();
        let state = mqtt::State {
            client,
            eventloop,
            err_streak: 1,
        };
        let state = handle_error(
            state,
            error,
            &device,
            &token_mngr,
            &options.connect_address,
            &device_file,
        )
        .await;
        assert_eq!(token_mngr.num_refresh_token_calls(), 0);

        // should not increment the error streak
        assert_eq!(state.err_streak, 1);

        // should not reinitialize the mqtt client
        assert_eq!(state.client.created_at, created_at);

        // should patch the device file to disconnected since it was online
        let device = device_file.read().await.unwrap();
        assert_eq!(device.status, DeviceStatus::Offline);
        assert!(device.last_disconnected_at >= before_patch);
        assert!(device.last_disconnected_at <= Utc::now());
    }
}
