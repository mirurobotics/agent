//! gzip completeness: never `Complete` in this version. The finalization
//! marker (the CRC32/ISIZE trailer) cannot be validated without decompressing
//! the whole stream, which violates the cheapness constraint — so a plausible
//! gzip file is `Unknown` and callers fall back to the stability window.
//! `.tgz` is treated as gzip (the tar layer is inside the compressed stream).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// gzip magic bytes.
const MAGIC: [u8; 2] = [0x1F, 0x8B];
/// 10-byte header + 8-byte CRC32/ISIZE trailer.
const MIN_SIZE: u64 = 18;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, 2).await?;
    Ok(if head == MAGIC {
        Completeness::Unknown
    } else {
        Completeness::Incomplete
    })
}
