// internal crates
use crate::services::calibration;

// external crates
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};

/// The calibration content route's response: the data, with the
/// `Content-Type` of its format and an inline `Content-Disposition`.
pub fn calibration_content(content: calibration::Content) -> Response {
    let filename = match content.format.file_ext() {
        Some(ext) => format!("{}.{ext}", content.report_id),
        None => content.report_id.to_string(),
    };
    let disposition = format!("inline; filename=\"{filename}\"");
    let headers = [
        (
            header::CONTENT_TYPE,
            content.format.media_type().to_string(),
        ),
        (header::CONTENT_DISPOSITION, disposition),
    ];
    (StatusCode::OK, headers, content.data).into_response()
}

/// Whole seconds since `at`, or None when `at` is still the epoch it
/// defaults to, so it never happened.
pub(crate) fn secs_ago_if_set(at: DateTime<Utc>, now: DateTime<Utc>) -> Option<i64> {
    (at != DateTime::<Utc>::UNIX_EPOCH).then(|| secs_ago(at, now))
}

/// Whole seconds since `at`, rounded down, and 0 if the clock has moved back
/// past `at`.
pub(crate) fn secs_ago(at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (now - at).num_seconds().max(0)
}

/// Whole seconds until `at`, rounded up, so it is 0 only once `at` has passed.
pub(crate) fn secs_until(at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    if at <= now {
        return 0;
    }
    let remaining = at - now;
    remaining.num_seconds() + i64::from(remaining.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    // external crates
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).unwrap()
    }

    #[test]
    fn secs_ago_if_set_is_none_at_the_epoch() {
        assert_eq!(secs_ago_if_set(DateTime::<Utc>::UNIX_EPOCH, now()), None);
    }

    #[test]
    fn secs_ago_if_set_is_secs_ago_for_a_set_time() {
        let at = now() - Duration::seconds(30);
        assert_eq!(secs_ago_if_set(at, now()), Some(30));
    }

    #[test]
    fn secs_ago_rounds_down() {
        let at = now() - Duration::milliseconds(1_999);
        assert_eq!(secs_ago(at, now()), 1);
    }

    #[test]
    fn secs_ago_is_0_for_a_time_after_now() {
        let at = now() + Duration::seconds(5);
        assert_eq!(secs_ago(at, now()), 0);
    }

    #[test]
    fn secs_until_rounds_up() {
        let at = now() + Duration::milliseconds(1_500);
        assert_eq!(secs_until(at, now()), 2);
    }

    #[test]
    fn secs_until_is_1_for_less_than_a_second_left() {
        let at = now() + Duration::milliseconds(1);
        assert_eq!(secs_until(at, now()), 1);
    }

    #[test]
    fn secs_until_is_0_at_now() {
        assert_eq!(secs_until(now(), now()), 0);
    }

    #[test]
    fn secs_until_is_0_for_a_time_before_now() {
        let at = now() - Duration::seconds(5);
        assert_eq!(secs_until(at, now()), 0);
    }
}
