//! AVI completeness: writers fill the RIFF size field on close (a recording
//! AVI has 0 or a stale value), so `riff_size + 8` must equal the file size.
//!
//! Known caveat: OpenDML (AVI 2.0) files > ~1 GiB append additional
//! `RIFF AVIX` chunks after the first RIFF chunk, so this check reports them
//! `Incomplete`. Extending to a bounded top-level RIFF-chunk walk (like MP4)
//! is deliberate future work, not v1.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// `RIFF` (4) + riff_size (4) + `AVI ` (4).
const MIN_SIZE: u64 = 12;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, MIN_SIZE).await?;
    if head.len() < MIN_SIZE as usize || head[0..4] != *b"RIFF" || head[8..12] != *b"AVI " {
        return Ok(Completeness::Incomplete);
    }
    let riff_size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as u64;
    Ok(if riff_size + 8 == size {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}
