// standard library
use std::time::Duration;

// internal crates
use crate::deploy::fsm;
use crate::server::serve::ServerOptions;
use crate::storage::{caches::CacheCapacities, layout::StorageLayout};
use crate::workers::{mqtt, poller, token_refresh::TokenRefreshWorkerOptions};

#[derive(Debug, Clone, Copy)]
pub struct LifecycleOptions {
    pub is_persistent: bool,
    pub max_runtime: Duration,
    pub idle_timeout: Duration,
    pub idle_timeout_poll_interval: Duration,
    pub max_shutdown_delay: Duration,
}

impl Default for LifecycleOptions {
    fn default() -> Self {
        Self {
            is_persistent: true,
            max_runtime: Duration::from_secs(60 * 15), // 15 minutes
            idle_timeout: Duration::from_secs(60),
            idle_timeout_poll_interval: Duration::from_secs(5),
            max_shutdown_delay: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Default)]
pub struct StorageOptions {
    pub layout: StorageLayout,
    pub cache_capacities: CacheCapacities,
}

#[derive(Debug)]
pub struct AppOptions {
    pub lifecycle: LifecycleOptions,

    pub storage: StorageOptions,
    pub token_refresh_worker: TokenRefreshWorkerOptions,
    pub dpl_retry_policy: fsm::RetryPolicy,

    pub backend_base_url: String,

    pub enable_socket_server: bool,
    pub server: ServerOptions,

    pub enable_mqtt_worker: bool,
    pub mqtt_worker: mqtt::Options,

    pub enable_poller: bool,
    pub poller: poller::Options,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            lifecycle: LifecycleOptions::default(),

            storage: StorageOptions::default(),
            token_refresh_worker: TokenRefreshWorkerOptions::default(),
            dpl_retry_policy: fsm::RetryPolicy::default(),

            backend_base_url: "https://api.mirurobotics.com/agent/v1".to_string(),

            enable_socket_server: true,
            server: ServerOptions::default(),

            enable_mqtt_worker: true,
            mqtt_worker: mqtt::Options::default(),

            enable_poller: true,
            poller: poller::Options::default(),
        }
    }
}
