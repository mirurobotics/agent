// standard crates
use std::fmt;

// external crates
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing::warn;

/// Returns true for `localhost` and `127.0.0.1`.
pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1")
}

fn is_allowed_host(host: &str) -> bool {
    const ALLOWED_DOMAIN: &str = "mirurobotics.com";
    const ALLOWED_DOMAIN_SUFFIX: &str = ".mirurobotics.com";
    host == ALLOWED_DOMAIN || host.ends_with(ALLOWED_DOMAIN_SUFFIX)
}

/// A bare backend hostname (optionally with a port) whose only constructor
/// enforces the allowed-domain rule. Any in-memory `BackendHost` is a
/// validated host plus optional port; the scheme and `/agent/v1` path are
/// derived per call by [`BackendHost::as_url`] and are not part of the
/// public surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendHost {
    /// Bare hostname.
    host: String,
    /// Optional port from the input.
    port: Option<u16>,
    /// Pre-formatted authority — `host` or `host:port`. Identical to what
    /// `Display` writes; exposed via `as_str`.
    formatted: String,
}

impl BackendHost {
    /// Validates `raw` and constructs a `BackendHost`.
    ///
    /// Rules — `raw` is rejected if any of the following is true:
    /// - empty input,
    /// - contains `/` (a path is being passed, not a host),
    /// - contains `@` (userinfo),
    /// - contains `://` (a scheme; full URL was passed),
    /// - the host is neither loopback (per [`is_loopback_host`]) nor an
    ///   allowed mirurobotics domain (per `is_allowed_host`),
    /// - the port is non-numeric or out of range.
    pub fn new(raw: &str) -> Result<Self, String> {
        if raw.is_empty() {
            return Err("backend host must not be empty".into());
        }
        if raw.contains("://") {
            return Err("backend host must not contain a scheme".into());
        }
        if raw.contains('@') {
            return Err("backend host must not contain userinfo".into());
        }
        if raw.contains('/') {
            return Err("backend host must not contain a path".into());
        }

        let (host, port) = match raw.rsplit_once(':') {
            Some((h, p)) => (
                h,
                Some(
                    p.parse::<u16>()
                        .map_err(|e| format!("invalid port in `{raw}`: {e}"))?,
                ),
            ),
            None => (raw, None),
        };
        if host.is_empty() {
            return Err(format!("backend host `{raw}` has no host"));
        }
        if !is_loopback_host(host) && !is_allowed_host(host) {
            return Err(format!("host `{host}` is not allowed"));
        }

        let formatted = match port {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };

        Ok(Self {
            host: host.to_string(),
            port,
            formatted,
        })
    }

    pub fn new_or(raw: &str, fallback: Self) -> Self {
        match BackendHost::new(raw) {
            Ok(host) => host,
            Err(msg) => {
                warn!("`{raw}` is not a valid backend host: {msg}");
                warn!("falling back to default `{fallback}`");
                fallback
            }
        }
    }

    /// Returns the host (or `host:port`) form — the same string `Display`
    /// writes.
    pub fn as_str(&self) -> &str {
        self.formatted.as_str()
    }

    /// Builds the fully-qualified backend URL. Scheme is `http` for
    /// loopback hosts and `https` otherwise; the path is always
    /// `/agent/v1` with no trailing slash.
    pub fn as_url(&self) -> String {
        let scheme = if is_loopback_host(self.host.as_str()) {
            "http"
        } else {
            "https"
        };
        let authority = match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        };
        format!("{scheme}://{authority}/agent/v1")
    }
}

impl Default for BackendHost {
    fn default() -> Self {
        Self::new("api.mirurobotics.com").expect("default backend host must be valid")
    }
}

impl fmt::Display for BackendHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.formatted.as_str())
    }
}

impl Serialize for BackendHost {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.formatted.as_str())
    }
}

impl<'de> Deserialize<'de> for BackendHost {
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
                warn!("`{host}` is not a valid MQTT host: {msg}");
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
