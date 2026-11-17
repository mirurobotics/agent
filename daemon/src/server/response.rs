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

/// Whole seconds since `at`, or None when `at` is before the agent last
/// started: a time from an earlier run, or one never set.
pub(crate) fn secs_ago_since_start(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    started_at: DateTime<Utc>,
) -> Option<i64> {
    (at >= started_at).then(|| secs_ago(at, now))
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
