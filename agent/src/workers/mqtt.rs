// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

// internal modules
use crate::authn::{token::Token, token_mngr::TokenManagerExt};
use crate::cooldown;
use crate::errors::*;
use crate::models::device::{self, Device, DeviceStatus};
use crate::mqtt;
use crate::mqtt::{
    client::{poll, ClientI, MQTTClient},
    device::{Ping, SyncDevice},
    errors::*,
    options::{ConnectAddress, Credentials, Options as MqttOptions},
    topics,
};
use crate::storage::device::DeviceFile;
use crate::sync::syncer::{SyncEvent, SyncerExt};

// external crates
use rumqttc::{ConnectReturnCode, Event, EventLoop, Incoming, Publish};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct Options {
    pub backoff: cooldown::Backoff,
    pub broker_address: ConnectAddress,
}

impl Default for Options {
    fn default() -> Self {
        let five_mins = 5 * 60;
        Self {
            backoff: cooldown::Backoff {
                base_secs: 1,
                growth_factor: 2,
                max_secs: five_mins,
            },
            broker_address: ConnectAddress::default(),
        }
    }
}

pub async fn run<F, Fut, TokenManagerT: TokenManagerExt, SyncerT: SyncerExt>(
    options: &Options,
    token_mngr: &TokenManagerT,
    syncer: &SyncerT,
    device_file: &DeviceFile,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("MQTT worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(
            options,
            token_mngr,
            syncer,
            device_file,
            sleep_fn,
        ) => {}
    }
}

pub async fn run_impl<F, Fut, TokenManagerT: TokenManagerExt, SyncerT: SyncerExt>(
    options: &Options,
    token_mngr: &TokenManagerT,
    syncer: &SyncerT,
    device_file: &DeviceFile,
    sleep_fn: F,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running mqtt worker");

    // subscribe to syncer events
    let mut syncer_subscriber = syncer.subscribe().await.unwrap_or_else(|e| {
        error!("error subscribing to syncer events: {e:?}");
        // Create a dummy receiver that never sends anything
        watch::channel(SyncEvent::SyncSuccess).1
    });

    let device = device_file
        .read()
        .await
        .unwrap_or_else(|_| Arc::new(device::Device::default()));

    // create the mqtt client
    let (mqtt_client, eventloop) = init_client(
        &device.id,
        &device.session_id,
        token_mngr,
        options.broker_address.clone(),
    )
    .await;

    let mut state = State {
        client: mqtt_client,
        eventloop,
        err_streak: 0,
    };

    loop {
        tokio::select! {
            // listen for syncer events from the syncer worker (this device)
            _ = syncer_subscriber.changed() => {
                let syncer_event = syncer_subscriber.borrow().clone();
                handle_syncer_event(
                    &syncer_event,
                    &device.id,
                    &state.client,
                ).await;
            }

            // listen for sync commands from the backend (via mqtt broker)
            mqtt_result = poll(&mut state.eventloop) => {
                match mqtt_result {
                    Ok(mqtt_event) => {
                        state.err_streak = handle_event(
                            &mqtt_event,
                            &state.client,
                            syncer,
                            &device.id,
                            device_file,
                        ).await;
                    }
                    Err(e) => {
                        state = handle_error(
                            state,
                            e,
                            &device,
                            token_mngr,
                            &options.broker_address,
                            device_file,
                        ).await;
                    }
                }
            }
        }

        // sleep for the cooldown period to prevent throttling from mqtt errors
        let cooldown_secs = cooldown::calc(&options.backoff, state.err_streak);
        let cooldown_duration = Duration::from_secs(cooldown_secs as u64);
        sleep_fn(cooldown_duration).await;
    }
}

async fn init_client<TokenManagerT: TokenManagerExt>(
    device_id: &str,
    device_session_id: &str,
    token_mngr: &TokenManagerT,
    broker_address: ConnectAddress,
) -> (MQTTClient, EventLoop) {
    // update the mqtt password
    let token = match token_mngr.get_token().await {
        Ok(token) => token.token.clone(),
        Err(_) => Token::default().token,
    };

    // initialize the mqtt client
    let credentials = Credentials {
        username: device_session_id.to_string(),
        password: token,
    };
    let options = MqttOptions::new(credentials)
        .with_connect_address(broker_address)
        .with_client_id(device_id.to_string());
    let (mqtt_client, eventloop) = MQTTClient::new(&options).await;

    // subscribe to device synchronization updates
    if let Err(e) = mqtt::device::subscribe_sync(&mqtt_client, device_id).await {
        error!("error subscribing to device synchronization updates: {e:?}");
    };
    if let Err(e) = mqtt::device::subscribe_ping(&mqtt_client, device_id).await {
        error!("error subscribing to device ping updates: {e:?}");
    };

    (mqtt_client, eventloop)
}

pub async fn handle_syncer_event<MQTTClientT: ClientI>(
    event: &SyncEvent,
    device_id: &str,
    mqtt_client: &MQTTClientT,
) {
    if !matches!(event, SyncEvent::SyncSuccess) {
        return;
    }

    // whenever the syncer has synced, we need to publish this synchronization to the
    // backend
    match mqtt::device::publish_sync(mqtt_client, device_id).await {
        Ok(_) => {
            info!("successfully published device sync to backend");
        }
        Err(e) => {
            error!("error publishing device sync: {e:?}");
        }
    }
}

type ErrStreak = u32;

pub async fn handle_event<MQTTClientT: ClientI, SyncerT: SyncerExt>(
    event: &Event,
    mqtt_client: &MQTTClientT,
    syncer: &SyncerT,
    device_id: &str,
    device_file: &DeviceFile,
) -> ErrStreak {
    let err_streak = 0;

    match event {
        // update the device connection status on successful connections
        Event::Incoming(Incoming::ConnAck(connack)) => {
            if connack.code != ConnectReturnCode::Success {
                return err_streak;
            }
            info!("Established connection to mqtt broker");
            let _ = device_file.patch(device::Updates::connected()).await;
        }
        // update the device connection status on successful disconnections
        Event::Incoming(Incoming::Disconnect) => {
            info!("Disconnected from mqtt broker");
            let _ = device_file.patch(device::Updates::disconnected()).await;
        }

        // sync the device if the payload is a sync request
        Event::Incoming(Incoming::Publish(publish)) => {
            let topic = topics::parse_subscription(device_id, &publish.topic);
            match topic {
                topics::SubscriptionTopics::Sync => {
                    handle_sync_event(publish, syncer).await;
                }
                topics::SubscriptionTopics::Ping => {
                    handle_ping_event(publish, mqtt_client, device_id).await;
                }
                topics::SubscriptionTopics::Unknown => {
                    warn!("unknown topic: {}", publish.topic);
                }
            }
        }

        _ => {}
    }

    err_streak
}

async fn handle_sync_event<SyncerT: SyncerExt>(publish: &Publish, syncer: &SyncerT) {
    let is_synced = match serde_json::from_slice::<SyncDevice>(&publish.payload) {
        Ok(sync_req) => sync_req.is_synced,
        Err(e) => {
            error!("error deserializing sync request: {e:?}");
            false
        }
    };
    if is_synced {
        return;
    }
    if let Err(e) = syncer.sync_if_not_in_cooldown().await {
        error!("error syncing device: {e:?}");
    }
}

async fn handle_ping_event<MQTTClientT: ClientI>(
    publish: &Publish,
    client: &MQTTClientT,
    device_id: &str,
) {
    let message_id = match serde_json::from_slice::<Ping>(&publish.payload) {
        Ok(ping) => {
            info!(
                "received ping request at {} with message id {}",
                ping.timestamp, ping.message_id,
            );
            ping.message_id
        }
        Err(e) => {
            error!("error deserializing ping request: {e:?}");
            return;
        }
    };
    if let Err(e) = mqtt::device::publish_pong(client, device_id, message_id).await {
        error!("error publishing ping response: {e:?}");
    } else {
        info!("successfully published ping response");
    }
}

pub struct State {
    pub client: MQTTClient,
    pub eventloop: EventLoop,
    pub err_streak: ErrStreak,
}

pub async fn handle_error<TokenManagerT: TokenManagerExt>(
    mut state: State,
    e: MQTTError,
    device: &Device,
    token_mngr: &TokenManagerT,
    broker_address: &ConnectAddress,
    device_file: &DeviceFile,
) -> State {
    state.err_streak = if e.is_network_connection_error() {
        // don't increment the error streak on network connection errors
        state.err_streak
    } else {
        state.err_streak + 1
    };

    // update the device to be offline
    match device_file.read().await {
        Ok(device) => {
            if device.status == DeviceStatus::Online {
                let _ = device_file.patch(device::Updates::disconnected()).await;
            }
        }
        Err(_) => {
            let _ = device_file.patch(device::Updates::disconnected()).await;
        }
    }

    // auth error -> refresh token and reinitialize the mqtt client
    if e.is_authentication_error() {
        error!("authentication error while polling backend for sync command via mqtt: {e:?}");
        info!("attempting to refresh token");
        if let Err(e) = token_mngr.refresh_token().await {
            error!("error refreshing token for backend sync worker: {e:?}");
        }
        let (mqtt_client, eventloop) = init_client(
            &device.id,
            &device.session_id,
            token_mngr,
            broker_address.clone(),
        )
        .await;
        state.client = mqtt_client;
        state.eventloop = eventloop;
        state
    }
    // network connection error -> ignore
    else if e.is_network_connection_error() {
        debug!("network connection error while polling backend for sync command via mqtt: {e:?}");
        state
    // other errors -> log
    } else {
        error!("error polling backend for sync command via mqtt: {e:?}");
        state
    }
}
