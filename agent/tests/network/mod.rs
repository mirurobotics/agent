// internal crates
use miru_agent::network::{BackendUrl, MqttHost};

mod backend_url_new {
    use super::*;

    #[test]
    fn accepts_https_allowed_domain_with_path() {
        BackendUrl::new("https://api.mirurobotics.com/agent/v1").unwrap();
    }

    #[test]
    fn accepts_https_subdomain() {
        BackendUrl::new("https://staging.mirurobotics.com/x").unwrap();
    }

    #[test]
    fn accepts_http_loopback_localhost() {
        BackendUrl::new("http://localhost:8080").unwrap();
    }

    #[test]
    fn accepts_http_loopback_ipv4() {
        BackendUrl::new("http://127.0.0.1:8080").unwrap();
    }

    #[test]
    fn rejects_http_non_loopback() {
        let err = BackendUrl::new("http://api.mirurobotics.com").unwrap_err();
        assert!(
            err.contains("https"),
            "expected scheme-rule message, got: {err}"
        );
    }

    #[test]
    fn rejects_suffix_attack_no_leading_dot() {
        let err = BackendUrl::new("https://evilmirurobotics.com").unwrap_err();
        assert!(
            err.contains("evilmirurobotics.com"),
            "expected host name in message, got: {err}"
        );
    }

    #[test]
    fn rejects_suffix_attack_wrong_end() {
        let err = BackendUrl::new("https://api.mirurobotics.com.attacker.com").unwrap_err();
        assert!(
            err.contains("attacker.com"),
            "expected attacker host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_userinfo_explicit() {
        let err = BackendUrl::new("https://user:pass@api.mirurobotics.com").unwrap_err();
        assert!(
            err.contains("userinfo"),
            "expected userinfo message, got: {err}"
        );
    }

    #[test]
    fn rejects_userinfo_confusion() {
        // The Url crate parses host=`api.mirurobotics.com`, username=`attacker.com`
        // (no password). The userinfo check still rejects on non-empty username.
        let err = BackendUrl::new("https://attacker.com@api.mirurobotics.com").unwrap_err();
        assert!(
            err.contains("userinfo"),
            "expected userinfo message, got: {err}"
        );
    }

    #[test]
    fn rejects_private_ip() {
        let err = BackendUrl::new("https://192.168.1.1").unwrap_err();
        assert!(
            err.contains("192.168.1.1"),
            "expected IP host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_disallowed_scheme() {
        let err = BackendUrl::new("ftp://api.mirurobotics.com").unwrap_err();
        assert!(err.contains("ftp"), "expected scheme name, got: {err}");
    }

    #[test]
    fn rejects_unparseable_url() {
        // Not a valid URL at all; the Url crate fails to parse it.
        let err = BackendUrl::new("not a url").unwrap_err();
        assert!(
            err.contains("invalid URL"),
            "expected parse error message, got: {err}"
        );
    }

    #[test]
    fn rejects_url_without_host() {
        // `data:` URLs parse but expose no host. Exercises the
        // `URL must contain a host` branch.
        let err = BackendUrl::new("data:text/plain,hello").unwrap_err();
        assert!(
            err.contains("host"),
            "expected host-missing message, got: {err}"
        );
    }

    #[test]
    fn display_matches_as_str() {
        let url = BackendUrl::new("https://api.mirurobotics.com/agent/v1").unwrap();
        assert_eq!(format!("{url}"), "https://api.mirurobotics.com/agent/v1");
        assert_eq!(format!("{url}"), url.as_str());
    }

    #[test]
    fn default_is_valid_production_url() {
        // The Default impl uses `expect`; this test fails loudly if the
        // default constant ever stops being valid.
        let url = BackendUrl::default();
        assert_eq!(url.as_str(), "https://api.mirurobotics.com/agent/v1");
    }

    #[test]
    fn deserialize_accepts_valid_url() {
        let url: BackendUrl =
            serde_json::from_str("\"https://api.mirurobotics.com/agent/v1\"").unwrap();
        assert_eq!(url.as_str(), "https://api.mirurobotics.com/agent/v1");
    }

    #[test]
    fn deserialize_rejects_invalid_url() {
        // The newtype's own Deserialize is strict. The warn-and-fall-back
        // semantics live one layer up in `Backend::deserialize`.
        let err =
            serde_json::from_str::<BackendUrl>("\"https://evilmirurobotics.com\"").unwrap_err();
        assert!(
            err.to_string().contains("evilmirurobotics.com"),
            "expected host in message, got: {err}"
        );
    }

    #[test]
    fn serialize_round_trip() {
        let url = BackendUrl::new("https://staging.mirurobotics.com/x").unwrap();
        let json = serde_json::to_string(&url).unwrap();
        assert_eq!(json, "\"https://staging.mirurobotics.com/x\"");
    }

    #[test]
    fn new_or_returns_constructed_url_on_valid_input() {
        let fallback = BackendUrl::default();
        let url = BackendUrl::new_or("https://staging.mirurobotics.com/x", fallback);
        assert_eq!(url.as_str(), "https://staging.mirurobotics.com/x");
    }

    #[test]
    fn new_or_returns_fallback_on_invalid_input() {
        let fallback = BackendUrl::new("https://api.mirurobotics.com/agent/v1").unwrap();
        let url = BackendUrl::new_or("https://evilmirurobotics.com", fallback.clone());
        assert_eq!(url, fallback);
    }
}

mod mqtt_host_new {
    use super::*;

    #[test]
    fn accepts_subdomain() {
        MqttHost::new("mqtt.mirurobotics.com").unwrap();
    }

    #[test]
    fn accepts_localhost() {
        MqttHost::new("localhost").unwrap();
    }

    #[test]
    fn accepts_loopback_ipv4() {
        MqttHost::new("127.0.0.1").unwrap();
    }

    #[test]
    fn accepts_loopback_ipv6() {
        MqttHost::new("::1").unwrap();
    }

    #[test]
    fn accepts_exact_allowed_domain() {
        MqttHost::new("mirurobotics.com").unwrap();
    }

    #[test]
    fn rejects_suffix_attack_no_leading_dot() {
        let err = MqttHost::new("evilmirurobotics.com").unwrap_err();
        assert!(
            err.contains("evilmirurobotics.com"),
            "expected host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_suffix_attack_wrong_end() {
        let err = MqttHost::new("mqtt.mirurobotics.com.attacker.com").unwrap_err();
        assert!(
            err.contains("attacker.com"),
            "expected attacker host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_private_ip() {
        let err = MqttHost::new("192.168.1.1").unwrap_err();
        assert!(
            err.contains("192.168.1.1"),
            "expected IP in message, got: {err}"
        );
    }

    #[test]
    fn display_matches_as_str() {
        let host = MqttHost::new("mqtt.mirurobotics.com").unwrap();
        assert_eq!(format!("{host}"), "mqtt.mirurobotics.com");
        assert_eq!(format!("{host}"), host.as_str());
    }

    #[test]
    fn default_is_valid_production_host() {
        let host = MqttHost::default();
        assert_eq!(host.as_str(), "mqtt.mirurobotics.com");
    }

    #[test]
    fn deserialize_accepts_valid_host() {
        let host: MqttHost = serde_json::from_str("\"mqtt.mirurobotics.com\"").unwrap();
        assert_eq!(host.as_str(), "mqtt.mirurobotics.com");
    }

    #[test]
    fn deserialize_rejects_invalid_host() {
        // Newtype Deserialize is strict; settings-layer impls handle fall-back.
        let err = serde_json::from_str::<MqttHost>("\"evilmirurobotics.com\"").unwrap_err();
        assert!(
            err.to_string().contains("evilmirurobotics.com"),
            "expected host in message, got: {err}"
        );
    }

    #[test]
    fn serialize_round_trip() {
        let host = MqttHost::new("mqtt.mirurobotics.com").unwrap();
        let json = serde_json::to_string(&host).unwrap();
        assert_eq!(json, "\"mqtt.mirurobotics.com\"");
    }

    #[test]
    fn new_or_returns_constructed_host_on_valid_input() {
        let fallback = MqttHost::default();
        let host = MqttHost::new_or("mqtt.mirurobotics.com", fallback);
        assert_eq!(host.as_str(), "mqtt.mirurobotics.com");
    }

    #[test]
    fn new_or_returns_fallback_on_invalid_input() {
        let fallback = MqttHost::new("mqtt.mirurobotics.com").unwrap();
        let host = MqttHost::new_or("evilmirurobotics.com", fallback.clone());
        assert_eq!(host, fallback);
    }
}
