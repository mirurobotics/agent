// standard crates
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// external crates
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

const ALLOWED_DOMAIN: &str = "mirurobotics.com";
const ALLOWED_DOMAIN_SUFFIX: &str = ".mirurobotics.com";
const DEFAULT_BACKEND_URL: &str = "https://api.mirurobotics.com/agent/v1";
const DEFAULT_MQTT_HOST: &str = "mqtt.mirurobotics.com";

/// Returns true for the literal loopback hostnames we accept (used by both
/// the backend-URL and MQTT-host validators).
pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn is_allowed_host(host: &str) -> bool {
    host == ALLOWED_DOMAIN || host.ends_with(ALLOWED_DOMAIN_SUFFIX)
}

/// Defence-in-depth IP rejection helper used inside `BackendUrl::new`.
/// Returns `Err` when `host` parses as an IP that is not the v4 or v6
/// loopback. Returns `Ok(())` for hostnames that don't look like IPs and
/// for the loopback addresses.
///
/// This is reached today only for hosts already inside `mirurobotics.com`
/// that happen to parse as IPs (none currently exist), and is kept as a
/// guard against future allowlist edits that admit literal IPs.
fn reject_non_loopback_ip(host: &str) -> Result<(), String> {
    let Ok(ip) = host.parse::<IpAddr>() else {
        return Ok(());
    };
    let is_loopback_ip = matches!(ip, IpAddr::V4(v4) if v4 == Ipv4Addr::LOCALHOST)
        || matches!(ip, IpAddr::V6(v6) if v6 == Ipv6Addr::LOCALHOST);
    if is_loopback_ip {
        Ok(())
    } else {
        Err(format!("IP host `{ip}` is not allowed"))
    }
}

/// A backend base URL whose only constructor enforces the allowed-domain rule.
///
/// Any in-memory `BackendUrl` is necessarily valid: parses as a URL, has no
/// userinfo, has a host, uses `https` (or `http` for loopback only), and the
/// host is either a loopback literal or in the allowed domain. The
/// defence-in-depth IP check runs as the final step of construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendUrl(Url);

impl BackendUrl {
    /// Validates `raw` and constructs a `BackendUrl`. See the type docs for
    /// the rule set.
    ///
    /// Rules:
    /// - Must parse as a URL.
    /// - Must not contain userinfo (`user:pass@host`).
    /// - Must contain a host.
    /// - Scheme must be `https`, except `http` is permitted for loopback hosts.
    /// - Host must be either a loopback literal or in the allowed domain.
    /// - Any non-loopback IP host is rejected as defence in depth.
    pub fn new(raw: &str) -> Result<Self, String> {
        let url = Url::parse(raw).map_err(|e| format!("invalid URL: {e}"))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("URL must not contain userinfo".into());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "URL must contain a host".to_string())?;
        // host_str() preserves IPv6 brackets; strip them so the loopback
        // literal "::1" matches our string set.
        let bare_host = host.trim_start_matches('[').trim_end_matches(']');
        let loopback = is_loopback_host(bare_host);
        match (url.scheme(), loopback) {
            ("https", _) => {}
            ("http", true) => {}
            ("http", false) => return Err("non-loopback URL must use https".into()),
            (other, _) => return Err(format!("scheme `{other}` not allowed")),
        }
        if !loopback && !is_allowed_host(bare_host) {
            return Err(format!("host `{bare_host}` is not allowed"));
        }
        // Defence in depth: any non-loopback IP that somehow slipped past the
        // suffix check (e.g. future allowlist edits) is rejected here.
        reject_non_loopback_ip(bare_host)?;
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for BackendUrl {
    fn default() -> Self {
        Self::new(DEFAULT_BACKEND_URL).expect("default backend URL must be valid")
    }
}

impl fmt::Display for BackendUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

// `url::Url` only implements `Serialize`/`Deserialize` when the `serde`
// feature of the `url` crate is enabled. We don't enable it (cheaper to write
// the small impls here than to flip the feature).
impl Serialize for BackendUrl {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for BackendUrl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(serde::de::Error::custom)
    }
}

/// A bare MQTT broker hostname whose only constructor enforces the
/// allowed-domain rule. Any in-memory `MqttHost` is necessarily a loopback
/// literal or matches the allowed-domain suffix rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttHost(String);

impl MqttHost {
    /// Validates `host` and constructs an `MqttHost`. A host is allowed iff
    /// it is a loopback literal or matches the allowed-domain suffix rule.
    pub fn new(host: &str) -> Result<Self, String> {
        if is_loopback_host(host) || is_allowed_host(host) {
            Ok(Self(host.to_string()))
        } else {
            Err(format!("MQTT host `{host}` is not allowed"))
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for MqttHost {
    fn default() -> Self {
        Self::new(DEFAULT_MQTT_HOST).expect("default MQTT host must be valid")
    }
}

impl fmt::Display for MqttHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl Serialize for MqttHost {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for MqttHost {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    // The defence-in-depth IP check is unreachable through the public
    // `BackendUrl::new` entry point today (any non-loopback IP is already
    // rejected by the allowed-domain check that runs first). We exercise it
    // directly here so the guard stays correct if a future allowlist edit
    // makes it reachable.

    use super::*;

    mod reject_non_loopback_ip {
        use super::*;

        #[test]
        fn allows_non_ip_hostname() {
            reject_non_loopback_ip("api.mirurobotics.com").unwrap();
        }

        #[test]
        fn allows_ipv4_loopback() {
            reject_non_loopback_ip("127.0.0.1").unwrap();
        }

        #[test]
        fn allows_ipv6_loopback() {
            reject_non_loopback_ip("::1").unwrap();
        }

        #[test]
        fn rejects_non_loopback_ipv4() {
            let err = reject_non_loopback_ip("192.168.1.1").unwrap_err();
            assert!(
                err.contains("192.168.1.1"),
                "expected IP in message, got: {err}"
            );
        }

        #[test]
        fn rejects_non_loopback_ipv6() {
            let err = reject_non_loopback_ip("2001:db8::1").unwrap_err();
            assert!(
                err.contains("2001:db8::1"),
                "expected IP in message, got: {err}"
            );
        }
    }
}
