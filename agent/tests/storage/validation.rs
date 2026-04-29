// internal crates
use miru_agent::storage::validation::{validate_backend_url, validate_mqtt_host};

mod validate_backend_url {
    use super::*;

    #[test]
    fn accepts_https_allowed_domain_with_path() {
        validate_backend_url("https://api.mirurobotics.com/agent/v1").unwrap();
    }

    #[test]
    fn accepts_https_subdomain() {
        validate_backend_url("https://staging.mirurobotics.com/x").unwrap();
    }

    #[test]
    fn accepts_http_loopback_localhost() {
        validate_backend_url("http://localhost:8080").unwrap();
    }

    #[test]
    fn accepts_http_loopback_ipv4() {
        validate_backend_url("http://127.0.0.1:8080").unwrap();
    }

    #[test]
    fn rejects_http_non_loopback() {
        let err = validate_backend_url("http://api.mirurobotics.com").unwrap_err();
        assert!(
            err.contains("https"),
            "expected scheme-rule message, got: {err}"
        );
    }

    #[test]
    fn rejects_suffix_attack_no_leading_dot() {
        let err = validate_backend_url("https://evilmirurobotics.com").unwrap_err();
        assert!(
            err.contains("evilmirurobotics.com"),
            "expected host name in message, got: {err}"
        );
    }

    #[test]
    fn rejects_suffix_attack_wrong_end() {
        let err = validate_backend_url("https://api.mirurobotics.com.attacker.com").unwrap_err();
        assert!(
            err.contains("attacker.com"),
            "expected attacker host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_userinfo_explicit() {
        let err = validate_backend_url("https://user:pass@api.mirurobotics.com").unwrap_err();
        assert!(
            err.contains("userinfo"),
            "expected userinfo message, got: {err}"
        );
    }

    #[test]
    fn rejects_userinfo_confusion() {
        // The Url crate parses host=`api.mirurobotics.com`, username=`attacker.com`
        // (no password). The userinfo check still rejects on non-empty username.
        let err = validate_backend_url("https://attacker.com@api.mirurobotics.com").unwrap_err();
        assert!(
            err.contains("userinfo"),
            "expected userinfo message, got: {err}"
        );
    }

    #[test]
    fn rejects_private_ip() {
        let err = validate_backend_url("https://192.168.1.1").unwrap_err();
        assert!(
            err.contains("192.168.1.1"),
            "expected IP host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_disallowed_scheme() {
        let err = validate_backend_url("ftp://api.mirurobotics.com").unwrap_err();
        assert!(err.contains("ftp"), "expected scheme name, got: {err}");
    }

    #[test]
    fn rejects_unparseable_url() {
        // Not a valid URL at all — the Url crate fails to parse it.
        let err = validate_backend_url("not a url").unwrap_err();
        assert!(
            err.contains("invalid URL"),
            "expected parse error message, got: {err}"
        );
    }
}

mod validate_mqtt_host {
    use super::*;

    #[test]
    fn accepts_subdomain() {
        validate_mqtt_host("mqtt.mirurobotics.com").unwrap();
    }

    #[test]
    fn accepts_localhost() {
        validate_mqtt_host("localhost").unwrap();
    }

    #[test]
    fn accepts_loopback_ipv4() {
        validate_mqtt_host("127.0.0.1").unwrap();
    }

    #[test]
    fn accepts_loopback_ipv6() {
        validate_mqtt_host("::1").unwrap();
    }

    #[test]
    fn accepts_exact_allowed_domain() {
        validate_mqtt_host("mirurobotics.com").unwrap();
    }

    #[test]
    fn rejects_suffix_attack_no_leading_dot() {
        let err = validate_mqtt_host("evilmirurobotics.com").unwrap_err();
        assert!(
            err.contains("evilmirurobotics.com"),
            "expected host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_suffix_attack_wrong_end() {
        let err = validate_mqtt_host("mqtt.mirurobotics.com.attacker.com").unwrap_err();
        assert!(
            err.contains("attacker.com"),
            "expected attacker host in message, got: {err}"
        );
    }

    #[test]
    fn rejects_private_ip() {
        let err = validate_mqtt_host("192.168.1.1").unwrap_err();
        assert!(
            err.contains("192.168.1.1"),
            "expected IP in message, got: {err}"
        );
    }
}
