//! PNG completeness: a finalized file starts with the PNG signature and ends
//! with a complete IEND chunk (whose CRC is constant, since IEND carries no
//! data).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// PNG signature.
const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
/// The complete IEND chunk: zero length + `IEND` + constant CRC.
const IEND: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
];
/// Signature (8) + IEND chunk (12).
const MIN_SIZE: u64 = 20;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, 8).await?;
    if head != SIGNATURE {
        return Ok(Completeness::Incomplete);
    }
    let tail = files::read_tail(file, 12).await?;
    Ok(if tail == IEND {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}
