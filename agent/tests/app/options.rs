// standard crates
use std::time::Duration;

// internal crates
use miru_agent::app::options::{AppOptions, LifecycleOptions};

pub mod lifecycle_options_default {
    use super::*;

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
    fn backend_host() {
        assert_eq!(
            AppOptions::default().backend_host.as_str(),
            "api.mirurobotics.com"
        );
    }

    #[test]
    fn socket_server_enabled() {
        assert!(AppOptions::default().enable_socket_server);
    }

    #[test]
    fn tcp_server_enabled_only_on_windows() {
        let options = AppOptions::default();
        assert_eq!(options.enable_tcp_server, cfg!(windows));
        assert_eq!(
            options.server.tcp_port,
            miru_agent::server::DEFAULT_TCP_PORT
        );
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
