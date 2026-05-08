// standard crates
use std::fmt;

// external crates
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
                warn!("`{raw}` is not a valid backend URL: {msg}");
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

/// A bare backend hostname (optionally with a port) whose only constructor
/// enforces the allowed-domain rule. Any in-memory `BackendHost` is a
/// validated host plus optional port; the scheme and `/agent/v1` path are
/// derived per call by [`BackendHost::as_url`] and are not part of the
/// public surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendHost {
    /// Bare hostname (no brackets for IPv6 literals).
    host: String,
    /// Optional port from the input.
    port: Option<u16>,
    /// Pre-formatted authority — `host` or `host:port` (with brackets
    /// around IPv6 hosts when a port is present). Identical to what
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
    ///
    /// IPv6 host+port form requires brackets (e.g. `[::1]:8080`); bare IPv6
    /// loopback (`::1`) is accepted port-less.
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

        // Parse via url::Url with a synthetic scheme. This handles IPv6
        // bracket form (`[::1]:8080`) and surfaces port parse errors.
        // url::Url requires IPv6 hosts to be bracketed; the literal `::1`
        // is the one unbracketed form we accept, so wrap it before parsing.
        let url_input = if raw == "::1" {
            "http://[::1]".to_string()
        } else {
            format!("http://{raw}")
        };
        let parsed =
            Url::parse(&url_input).map_err(|e| format!("invalid backend host `{raw}`: {e}"))?;

        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err("backend host must not contain userinfo".into());
        }
        if parsed.path() != "" && parsed.path() != "/" {
            return Err("backend host must not contain a path".into());
        }
        if parsed.query().is_some() {
            return Err("backend host must not contain a query".into());
        }
        if parsed.fragment().is_some() {
            return Err("backend host must not contain a fragment".into());
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| format!("backend host `{raw}` has no host"))?;
        // host_str() preserves IPv6 brackets; strip them so the loopback
        // literal "::1" matches our string set.
        let bare_host = host.trim_start_matches('[').trim_end_matches(']');
        if !is_loopback_host(bare_host) && !is_allowed_host(bare_host) {
            return Err(format!("host `{bare_host}` is not allowed"));
        }

        let port = parsed.port();
        let host_is_ipv6 = bare_host.contains(':');
        let formatted = match (port, host_is_ipv6) {
            (Some(p), true) => format!("[{bare_host}]:{p}"),
            (Some(p), false) => format!("{bare_host}:{p}"),
            (None, _) => bare_host.to_string(),
        };

        Ok(Self {
            host: bare_host.to_string(),
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
    /// writes. IPv6 hosts are bracketed when a port is present.
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
        // IPv6 hosts must be bracketed inside a URL even when port-less,
        // unlike the `as_str` form which mirrors what the user typed.
        let host_is_ipv6 = self.host.contains(':');
        let authority = match (self.port, host_is_ipv6) {
            (Some(port), true) => format!("[{}]:{port}", self.host),
            (Some(port), false) => format!("{}:{port}", self.host),
            (None, true) => format!("[{}]", self.host),
            (None, false) => self.host.clone(),
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
        serializer.serialize_str(&self.to_string())
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
