//! Miru's validation rules for a calibration report (backend
//! `configs/services/calibrations/clean.go` and `create.go`), checked at
//! intake so that Miru rarely rejects an accepted report.

// internal crates
use crate::services::{calibration::CreateParams, errors::InvalidRequestErr};
use crate::trace;
use backend_api::models::{InstanceContent, InstanceFormat};

// external crates
use serde::{de::IgnoredAny, Deserialize};

/// The most bytes of `content.data`.
pub const MAX_CONTENT_BYTES: usize = 1024 * 1024;
/// The most characters (Unicode scalar values) of a serial number.
pub const MAX_SERIAL_NUMBER_CHARS: usize = 255;
/// The most characters of a slug.
pub const MAX_SLUG_CHARS: usize = 32;

/// Whether `s` is a Miru slug: 1 to [`MAX_SLUG_CHARS`] lowercase ASCII
/// letters, digits and hyphens, starting and ending with a letter or digit
/// (`^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$`). Nothing is trimmed.
pub fn is_slug(s: &str) -> bool {
    let alnum = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = s.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(first), Some(last)) => {
            bytes.len() <= MAX_SLUG_CHARS
                && alnum(first)
                && alnum(last)
                && bytes.iter().all(|b| alnum(b) || *b == b'-')
        }
        _ => false,
    }
}

/// Check `p` in Miru's order: the slugs, the serial number, then the content.
/// yaml and jsonc data aren't parsed (the agent has no parser for them), so a
/// malformed one is rejected by Miru and parked by the upload worker.
pub fn params(p: &CreateParams) -> Result<(), InvalidRequestErr> {
    slug("component_slug", &p.component_slug)?;
    if let Some(role) = &p.role {
        slug("role", role)?;
    }
    if let Some(aspect) = &p.aspect {
        slug("aspect", aspect)?;
    }
    if let Some(serial_number) = &p.serial_number {
        self::serial_number(serial_number)?;
    }
    content(&p.content)
}

fn slug(field: &'static str, value: &str) -> Result<(), InvalidRequestErr> {
    if is_slug(value) {
        return Ok(());
    }
    Err(invalid(
        field,
        format!("must be 1 to {MAX_SLUG_CHARS} lowercase letters, digits and hyphens, starting and ending with a letter or digit"),
    ))
}

fn serial_number(value: &str) -> Result<(), InvalidRequestErr> {
    if value.contains('\0') {
        return Err(invalid(
            "serial_number",
            "must not contain NUL characters".to_string(),
        ));
    }
    if value.chars().count() > MAX_SERIAL_NUMBER_CHARS {
        return Err(invalid(
            "serial_number",
            format!("must be at most {MAX_SERIAL_NUMBER_CHARS} characters"),
        ));
    }
    Ok(())
}

fn content(value: &serde_json::Value) -> Result<(), InvalidRequestErr> {
    let content = InstanceContent::deserialize(value).map_err(|e| {
        invalid(
            "content",
            format!("must be an object with string fields format and data: {e}"),
        )
    })?;
    if content.format == InstanceFormat::InstanceFormatUnknown {
        return Err(invalid(
            "content.format",
            "must be one of json, yaml, jsonc, xml or text".to_string(),
        ));
    }
    let bytes = content.data.len();
    if !(1..=MAX_CONTENT_BYTES).contains(&bytes) {
        return Err(invalid(
            "content.data",
            format!("must be 1 to {MAX_CONTENT_BYTES} bytes, not {bytes}"),
        ));
    }
    if content.format == InstanceFormat::INSTANCE_FORMAT_JSON {
        serde_json::from_str::<IgnoredAny>(&content.data)
            .map_err(|e| invalid("content.data", format!("is not valid json: {e}")))?;
    }
    Ok(())
}

fn invalid(field: &'static str, reason: String) -> InvalidRequestErr {
    InvalidRequestErr {
        field,
        reason,
        trace: trace!(),
    }
}
