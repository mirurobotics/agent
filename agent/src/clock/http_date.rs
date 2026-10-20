//! Parsing the HTTP `Date` and `Age` headers the backend's responses carry.

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// Parses an HTTP `Date` header (IMF-fixdate, e.g. `Tue, 20 Oct 2026 02:58:04
/// GMT`).
pub fn parse_http_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parses an HTTP `Age` header: how many seconds a cache held the response
/// before serving it. Anything but a non-negative whole number of seconds that
/// fits in a [`TimeDelta`] is rejected.
pub fn parse_age(value: &str) -> Option<TimeDelta> {
    let secs: i64 = value.trim().parse().ok()?;
    if secs < 0 {
        return None;
    }
    TimeDelta::try_seconds(secs)
}
