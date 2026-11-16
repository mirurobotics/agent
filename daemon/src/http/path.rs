// internal crates
use crate::http::errors::{HTTPErr, InvalidURLErr};
use crate::trace;

// external crates
use reqwest::Url;

/// Builds `base` followed by `segments`, percent-encoding each segment so it
/// cannot change which backend path the request goes to. Empty, `.` and `..`
/// segments are rejected because URL paths treat them as directory moves, which
/// `Url` silently drops.
pub fn url(base: &str, segments: &[&str]) -> Result<String, HTTPErr> {
    let invalid = |msg: String| {
        HTTPErr::InvalidURLErr(InvalidURLErr {
            url: base.to_string(),
            msg,
            trace: trace!(),
        })
    };
    if let Some(seg) = segments.iter().find(|s| matches!(**s, "" | "." | "..")) {
        return Err(invalid(format!("'{seg}' is not a valid path segment")));
    }
    let mut url = Url::parse(base).map_err(|e| invalid(e.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| invalid("base URL cannot have a path".to_string()))?
        .pop_if_empty()
        .extend(segments);
    Ok(url.into())
}
