//! MCAP completeness: a finalized file starts with the MCAP magic and ends
//! with a Footer record followed by the magic again.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// Leading and trailing MCAP magic bytes: `\x89 M C A P 0 \r \n`.
const MAGIC: [u8; 8] = [0x89, b'M', b'C', b'A', b'P', b'0', b'\r', b'\n'];
/// Opcode of the Footer record.
const FOOTER_OPCODE: u8 = 0x02;
/// The Footer record has a fixed 20-byte payload.
const FOOTER_PAYLOAD_LEN: u64 = 20;
/// Trailing region: opcode (1) + length (8) + payload (20) + magic (8).
const TAIL_LEN: u64 = 37;
/// Leading magic (8) + trailing region (37).
const MIN_SIZE: u64 = 45;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, 8).await?;
    if head != MAGIC {
        return Ok(Completeness::Incomplete);
    }
    let tail = files::read_tail(file, TAIL_LEN).await?;
    if tail.len() < TAIL_LEN as usize {
        return Ok(Completeness::Incomplete);
    }
    let footer_len = u64_le(&tail[1..9]);
    let finalized =
        tail[0] == FOOTER_OPCODE && footer_len == FOOTER_PAYLOAD_LEN && tail[29..37] == MAGIC;
    Ok(if finalized {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}

fn u64_le(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(buf)
}
