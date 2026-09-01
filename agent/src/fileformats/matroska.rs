//! MKV/WebM completeness: an EBML document is an EBML header element
//! followed by a Segment element; a finalized file declares a finite Segment
//! size that runs exactly to EOF, while a still-recording file leaves the
//! Segment size unknown (the all-ones vint).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// EBML header element ID.
const EBML_ID: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// Segment element ID.
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
/// An EBML header payload larger than this (typically ~40 bytes) is
/// implausible and not cheaply verifiable.
const MAX_EBML_HEADER_LEN: u64 = 4096;
/// Element window: 4-byte ID + up to 8-byte size vint.
const ELEMENT_WINDOW: u64 = 12;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    let head = files::read_range(file, 0, ELEMENT_WINDOW).await?;
    if head.len() < 4 || head[0..4] != EBML_ID {
        return Ok(Completeness::Incomplete);
    }
    let header_size = match parse_vint(&head[4..]) {
        Some(vint) => vint,
        // truncated or zero-width vint: inconclusive parse
        None => return Ok(Completeness::Unknown),
    };
    if header_size.unknown || header_size.value > MAX_EBML_HEADER_LEN {
        return Ok(Completeness::Unknown);
    }
    let segment_start = 4 + header_size.width as u64 + header_size.value;
    let segment_head = files::read_range(file, segment_start, ELEMENT_WINDOW).await?;
    if segment_head.len() < 4 || segment_head[0..4] != SEGMENT_ID {
        // inconclusive parse: the element after the EBML header is not a
        // Segment
        return Ok(Completeness::Unknown);
    }
    let segment_size = match parse_vint(&segment_head[4..]) {
        Some(vint) => vint,
        None => return Ok(Completeness::Unknown),
    };
    if segment_size.unknown {
        // an unknown-size Segment is exactly the still-recording state
        return Ok(Completeness::Incomplete);
    }
    let segment_payload_start = segment_start + 4 + segment_size.width as u64;
    Ok(if segment_payload_start + segment_size.value == size {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}

/// A parsed EBML variable-width integer.
struct Vint {
    /// Total width in bytes (1-8).
    width: usize,
    /// The value bits after the marker bit.
    value: u64,
    /// Whether all value bits are set, meaning "unknown size".
    unknown: bool,
}

/// Parses the size vint at the start of `bytes`. The count of leading zero
/// bits of the first byte determines the total width (1-8 bytes); the bits
/// after the marker bit are the value. Returns `None` for a zero-width vint
/// (first byte 0x00) or a truncated one.
fn parse_vint(bytes: &[u8]) -> Option<Vint> {
    let first = *bytes.first()?;
    if first == 0 {
        return None;
    }
    let width = first.leading_zeros() as usize + 1;
    if bytes.len() < width {
        return None;
    }
    let mask = (1u8 << (8 - width)) - 1;
    let mut value = (first & mask) as u64;
    for &b in &bytes[1..width] {
        value = (value << 8) | b as u64;
    }
    let max = (1u64 << (7 * width)) - 1;
    Some(Vint {
        width,
        value,
        unknown: value == max,
    })
}
