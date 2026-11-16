// internal crates
use crate::services::calibration;

// external crates
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

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
