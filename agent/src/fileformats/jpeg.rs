//! JPEG completeness: a finalized file starts with the SOI marker and ends
//! with the EOI marker, tolerating a few trailing NUL padding bytes that some
//! writers append.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// Start-of-image marker.
const SOI: [u8; 2] = [0xFF, 0xD8];
/// End-of-image marker.
const EOI: [u8; 2] = [0xFF, 0xD9];
/// SOI (2) + EOI (2).
const MIN_SIZE: u64 = 4;
/// Tail window; the EOI must appear here after stripping NUL padding.
const TAIL_LEN: u64 = 16;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, 2).await?;
    if head != SOI {
        return Ok(Completeness::Incomplete);
    }
    let tail = files::read_tail(file, TAIL_LEN).await?;
    let trimmed_len = tail
        .iter()
        .rposition(|&b| b != 0x00)
        .map(|i| i + 1)
        .unwrap_or(0);
    let finalized = trimmed_len >= 2 && tail[trimmed_len - 2..trimmed_len] == EOI;
    Ok(if finalized {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}
