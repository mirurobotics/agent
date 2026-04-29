// standard crates
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// external crates
use url::Url;

const ALLOWED_DOMAIN: &str = "mirurobotics.com";
const ALLOWED_DOMAIN_SUFFIX: &str = ".mirurobotics.com";

/// Returns true for the literal loopback hostnames we accept (used by both
/// the backend-URL and MQTT-host validators).
pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn is_allowed_domain(host: &str) -> bool {
    host == ALLOWED_DOMAIN || host.ends_with(ALLOWED_DOMAIN_SUFFIX)
}

/// Validates a backend base URL. Returns the parsed `Url` on success.
///
/// Rules:
/// - Must parse as a URL.
/// - Must not contain userinfo (`user:pass@host`).
/// - Must contain a host.
/// - Scheme must be `https`, except `http` is permitted for loopback hosts.
/// - Host must be either a loopback literal or in the allowed domain.
/// - Any non-loopback IP host is rejected as defence in depth.
pub fn validate_backend_url(raw: &str) -> Result<Url, String> {
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
    if !loopback && !is_allowed_domain(bare_host) {
        return Err(format!("host `{bare_host}` is not allowed"));
    }
    // Defence in depth: parsed-IP rejection for any non-loopback IP that
    // somehow slipped past the suffix check (e.g. future allowlist edits).
    if let Ok(ip) = bare_host.parse::<IpAddr>() {
        let is_loopback_ip = matches!(ip, IpAddr::V4(v4) if v4 == Ipv4Addr::LOCALHOST)
            || matches!(ip, IpAddr::V6(v6) if v6 == Ipv6Addr::LOCALHOST);
        if !is_loopback_ip {
            return Err(format!("IP host `{ip}` is not allowed"));
        }
    }
    Ok(url)
}

/// Validates a bare MQTT broker hostname (not a URL).
///
/// A host is allowed iff it is a loopback literal or matches the allowed-domain
/// suffix rule.
pub fn validate_mqtt_host(host: &str) -> Result<(), String> {
    if is_loopback_host(host) || is_allowed_domain(host) {
        Ok(())
    } else {
        Err(format!("MQTT host `{host}` is not allowed"))
    }
}
