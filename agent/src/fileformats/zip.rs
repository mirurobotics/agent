//! ZIP completeness: a finalized archive ends with an End of Central
//! Directory (EOCD) record whose comment field runs exactly to EOF. The EOCD
//! sits within the last 22 + 65,535 bytes (record minimum + maximum comment
//! length).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// EOCD record signature.
const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
/// Minimum EOCD record length (with an empty comment).
const EOCD_MIN_LEN: usize = 22;
/// Offset of `comment_len: u16 LE` within the EOCD record.
const COMMENT_LEN_OFFSET: usize = 20;
/// EOCD minimum (22) + maximum comment length (65,535).
const MAX_TAIL: u64 = 65_557;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < EOCD_MIN_LEN as u64 {
        return Ok(Completeness::Incomplete);
    }
    let tail = files::read_tail(file, size.min(MAX_TAIL)).await?;
    let tail_len = tail.len();
    if tail_len < EOCD_MIN_LEN {
        return Ok(Completeness::Incomplete);
    }
    // scan backwards for an EOCD record ending (record + comment) exactly at EOF
    for p in (0..=tail_len - EOCD_MIN_LEN).rev() {
        if tail[p..p + 4] != EOCD_SIG {
            continue;
        }
        let comment_len = u16::from_le_bytes([
            tail[p + COMMENT_LEN_OFFSET],
            tail[p + COMMENT_LEN_OFFSET + 1],
        ]) as usize;
        if p + EOCD_MIN_LEN + comment_len == tail_len {
            return Ok(Completeness::Complete);
        }
    }
    Ok(Completeness::Incomplete)
}
