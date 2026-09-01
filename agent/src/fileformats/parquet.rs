//! Parquet completeness: a finalized file ends with `footer_len: u32 LE` +
//! the ASCII magic `PAR1`, and starts with the same magic.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// Leading and trailing Parquet magic bytes.
const MAGIC: [u8; 4] = *b"PAR1";
/// Leading magic (4) + footer length (4) + trailing magic (4).
const MIN_SIZE: u64 = 12;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, 4).await?;
    if head != MAGIC {
        return Ok(Completeness::Incomplete);
    }
    let tail = files::read_tail(file, 8).await?;
    if tail.len() < 8 || tail[4..8] != MAGIC {
        return Ok(Completeness::Incomplete);
    }
    // the footer (plus both magics and its length field) must fit in the file
    let footer_len = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]) as u64;
    Ok(if footer_len + 8 <= size {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}
