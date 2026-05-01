// standard crates
use std::time::Duration;

// internal crates
use miru_agent::app::options::{AppOptions, LifecycleOptions};

pub mod lifecycle_options_default {
    use super::*;

    #[test]
    fn is_persistent() {
        assert!(LifecycleOptions::default().is_persistent);
    }

    #[test]
    fn max_runtime_is_15_minutes() {
        assert_eq!(
            LifecycleOptions::default().max_runtime,
            Duration::from_secs(60 * 15)
        );
    }

    #[test]
    fn idle_timeout_is_60_seconds() {
        assert_eq!(
            LifecycleOptions::default().idle_timeout,
            Duration::from_secs(60)
        );
    }

    #[test]
    fn idle_timeout_poll_interval_is_5_seconds() {
        assert_eq!(
            LifecycleOptions::default().idle_timeout_poll_interval,
            Duration::from_secs(5)
        );
    }

    #[test]
    fn max_shutdown_delay_is_15_seconds() {
        assert_eq!(
            LifecycleOptions::default().max_shutdown_delay,
            Duration::from_secs(15)
        );
    }
}

pub mod app_options_default {
    use super::*;

    #[test]
    fn backend_base_url() {
        assert_eq!(
            AppOptions::default().backend_base_url.as_str(),
            "https://api.mirurobotics.com/agent/v1"
        );
    }

    #[test]
    fn socket_server_enabled() {
        assert!(AppOptions::default().enable_socket_server);
    }

    #[test]
    fn mqtt_worker_enabled() {
        assert!(AppOptions::default().enable_mqtt_worker);
    }

    #[test]
    fn poller_enabled() {
        assert!(AppOptions::default().enable_poller);
    }
}
