// internal crates
use crate::server::{errors::*, routes};
use crate::trace;

// external crates
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::http::StatusCode;

/// A body over the limit is 413; any other unreadable body is 422.
pub(crate) fn reject_body(rejection: JsonRejection) -> ServerErr {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ServerErr::CalibrationRequestTooLargeErr(CalibrationRequestTooLargeErr {
            max_request_bytes: routes::MAX_REQUEST_BYTES,
            trace: trace!(),
        });
    }
    ServerErr::InvalidRequestErr(InvalidRequestErr {
        msg: rejection.body_text(),
        trace: trace!(),
    })
}

/// An unreadable query, such as a repeated filter, is 422.
pub(crate) fn reject_query(rejection: QueryRejection) -> ServerErr {
    ServerErr::InvalidRequestErr(InvalidRequestErr {
        msg: rejection.body_text(),
        trace: trace!(),
    })
}

/// The trimmed, non-empty items of a comma-separated list.
pub(crate) fn split_list(list: Option<String>) -> Vec<String> {
    let list = list.unwrap_or_default();
    let items = list.split(',').map(|item| item.trim().to_string());
    items.filter(|item| !item.is_empty()).collect()
}
