// standard crates
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// async crates
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// internal crates
use miru_agent::mqtt::client::{ClientI, Publish};
use miru_agent::mqtt::errors::MQTTError;

// external crates
use rumqttc::QoS;
use rumqttd::{Broker, Config, ConnectionSettings, RouterConfig, ServerSettings};

#[derive(Clone, Debug)]
pub enum MockCall {
    Publish {
        topic: String,
        qos: QoS,
        retained: bool,
        payload: Vec<u8>,
    },
    Subscribe {
        topic: String,
        qos: QoS,
    },
    Unsubscribe {
        topic: String,
    },
    Disconnect,
}

pub struct MockClient {
    pub publish_fn: Box<dyn Fn() -> Result<(), MQTTError> + Send + Sync>,
    pub subscribe_fn: Box<dyn Fn() -> Result<(), MQTTError> + Send + Sync>,
    pub unsubscribe_fn: Box<dyn Fn() -> Result<(), MQTTError> + Send + Sync>,
    pub disconnect_fn: Box<dyn Fn() -> Result<(), MQTTError> + Send + Sync>,
    pub calls: Arc<Mutex<Vec<MockCall>>>,
}

impl Default for MockClient {
    fn default() -> Self {
        Self {
            publish_fn: Box::new(|| Ok(())),
            subscribe_fn: Box::new(|| Ok(())),
            unsubscribe_fn: Box::new(|| Ok(())),
            disconnect_fn: Box::new(|| Ok(())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockClient {
    pub fn get_calls(&self) -> Vec<MockCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn num_publish_calls_to(&self, target_topic: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, MockCall::Publish { topic, .. } if topic == target_topic))
            .count()
    }

    pub fn num_subscribe_calls_to(&self, target_topic: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(
                |call| matches!(call, MockCall::Subscribe { topic, .. } if topic == target_topic),
            )
            .count()
    }
}

impl ClientI for MockClient {
    async fn publish(&self, msg: Publish<'_>) -> Result<(), MQTTError> {
        let Publish {
            topic,
            qos,
            retained,
            payload,
        } = msg;
        self.calls.lock().unwrap().push(MockCall::Publish {
            topic: topic.to_string(),
            qos,
            retained,
            payload: payload.to_vec(),
        });
        (self.publish_fn)()
    }

    async fn subscribe(&self, topic: &str, qos: QoS) -> Result<(), MQTTError> {
        self.calls.lock().unwrap().push(MockCall::Subscribe {
            topic: topic.to_string(),
            qos,
        });
        (self.subscribe_fn)()
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), MQTTError> {
        self.calls.lock().unwrap().push(MockCall::Unsubscribe {
            topic: topic.to_string(),
        });
        (self.unsubscribe_fn)()
    }

    async fn disconnect(&self) -> Result<(), MQTTError> {
        self.calls.lock().unwrap().push(MockCall::Disconnect);
        (self.disconnect_fn)()
    }
}

// ================================ MOCK BROKER ==================================== //

/// Guard that keeps a `rumqttd` broker alive for the duration of a test.
///
/// `rumqttd::Broker::start()` blocks indefinitely and exposes no shutdown API,
/// so the spawned thread will outlive this guard. Dropping the guard is still
/// the correct idiom: it signals intent and keeps port allocation scoped to the
/// test. Each test MUST use a unique port to avoid conflicts.
pub struct BrokerGuard {
    _handle: thread::JoinHandle<()>,
    port: u16,
}

impl BrokerGuard {
    /// Block until the broker is accepting TCP connections (or timeout).
    pub fn wait_ready(&self) {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "broker on port {} did not become ready within 5s",
            self.port
        );
    }
}

/// Starts a minimal TCP server that accepts one MQTT connection, reads the
/// CONNECT packet, and replies with a CONNACK containing the given return code.
///
/// `rumqttd` has a protocol violation where it drops the TCP connection on auth
/// failure instead of sending a CONNACK refusal. This mock sends the correct
/// MQTT 3.1.1 CONNACK so we can test the client's error classification locally.
pub async fn run_rejecting_broker(port: u16, return_code: u8) -> tokio::task::JoinHandle<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Read and discard the CONNECT packet
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            // CONNACK: packet type 0x20, remaining length 0x02, no session (0x00), return code
            let connack = [0x20, 0x02, 0x00, return_code];
            let _ = stream.write_all(&connack).await;
        }
    })
}

pub fn run_broker(port: u16, auth: Option<HashMap<String, String>>) -> BrokerGuard {
    let mut v4 = HashMap::new();
    v4.insert(
        "1".to_string(),
        ServerSettings {
            name: "v4-test".to_string(),
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            tls: None,
            next_connection_delay_ms: 0,
            connections: ConnectionSettings {
                connection_timeout_ms: 5000,
                max_payload_size: 2048,
                max_inflight_count: 100,
                auth,
                external_auth: None,
                dynamic_filters: true,
            },
        },
    );

    let config = Config {
        id: 0,
        router: RouterConfig {
            max_connections: 10,
            max_outgoing_packet_count: 200,
            max_segment_size: 100,
            max_segment_count: 10,
            ..RouterConfig::default()
        },
        v4: Some(v4),
        v5: None,
        ws: None,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    };

    let handle = thread::spawn(move || {
        let mut broker = Broker::new(config);
        broker.start().unwrap();
    });

    let guard = BrokerGuard {
        _handle: handle,
        port,
    };
    guard.wait_ready();
    guard
}
