// standard crates
use std::fmt;

// external crates
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[allow(unused_imports)]
use tracing::warn;
use url::Url;

/// Returns true for the literal loopback hostnames we accept.
pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn is_allowed_host(host: &str) -> bool {
    const ALLOWED_DOMAIN: &str = "mirurobotics.com";
    const ALLOWED_DOMAIN_SUFFIX: &str = ".mirurobotics.com";
    host == ALLOWED_DOMAIN || host.ends_with(ALLOWED_DOMAIN_SUFFIX)
}

/// A backend base URL whose only constructor enforces the allowed-domain rule.
///
/// Any in-memory `BackendUrl` is necessarily valid: parses as a URL, has no
/// userinfo, has a host, uses `https` (or `http` for loopback only), and the
/// host is either a loopback literal or in the allowed domain.
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
        Ok(Self(url))
    }

    pub fn new_or(raw: &str, fallback: Self) -> Self {
        match BackendUrl::new(raw) {
            Ok(url) => url,
            Err(msg) => {
                warn!("`{raw}` is not a valid backend URL: {msg})");
                warn!("falling back to default `{fallback}`");
                fallback
            }
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for BackendUrl {
    fn default() -> Self {
        const DEFAULT_BACKEND_URL: &str = "https://api.mirurobotics.com/agent/v1";
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

    pub fn new_or(host: &str, fallback: Self) -> Self {
        match MqttHost::new(host) {
            Ok(host) => host,
            Err(msg) => {
                warn!("`{host}` is not a valid MQTT host: {msg})");
                warn!("falling back to default `{fallback}`");
                fallback
            }
        }
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for MqttHost {
    fn default() -> Self {
        Self::new("mqtt.mirurobotics.com").expect("default MQTT host must be valid")
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
