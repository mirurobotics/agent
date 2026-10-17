// external crates
use chrono::{DateTime, Utc};

/// 2026-10-01T00:00:00Z: no earlier than this agent version's source, so no
/// genuine wall-clock reading taken by it can fall before this instant.
const FLOOR_SECS: i64 = 1_790_812_800;

/// The earliest wall-clock instant this agent build can genuinely observe.
///
/// Robots without a real-time clock boot with the clock at 1970 (or another
/// stale value) until NTP syncs. A timestamp stamped before this floor was
/// taken from such a clock, so it says nothing about how long ago the event
/// happened; callers re-stamp it once the clock is past the floor instead of
/// treating it as decades old.
pub fn floor() -> DateTime<Utc> {
    // FLOOR_SECS is a valid timestamp, so the fallback is unreachable.
    DateTime::from_timestamp(FLOOR_SECS, 0).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Whether `at` was read from a clock that had not been set yet.
pub fn is_before_floor(at: DateTime<Utc>) -> bool {
    at < floor()
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    // external crates
    use chrono::TimeZone;

    #[test]
    fn floor_is_2026_10_01() {
        assert_eq!(floor(), Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap());
    }

    #[test]
    fn is_before_floor_splits_at_the_floor() {
        assert!(is_before_floor(DateTime::UNIX_EPOCH));
        assert!(is_before_floor(floor() - chrono::TimeDelta::seconds(1)));
        assert!(!is_before_floor(floor()));
        assert!(!is_before_floor(Utc::now()));
    }
}
