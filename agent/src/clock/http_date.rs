//! Parsing the HTTP `Date` header the backend sends on every response.

// external crates
use chrono::{DateTime, Utc};

/// Parses an HTTP `Date` header (IMF-fixdate, e.g. `Tue, 20 Oct 2026 02:58:04
/// GMT`).
pub fn parse_http_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
