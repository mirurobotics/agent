//! HDF5 completeness: the superblock stores an end-of-file address — the
//! first byte past all HDF5 data — which equals the file size for a base
//! address of 0.
//!
//! v1 scope: only a superblock at offset 0 is recognized. Files with
//! userblocks place the superblock at 512/1024/... bytes; those fail the
//! signature check at offset 0 and report `Incomplete` (accepted caveat).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// HDF5 superblock signature: `\x89 H D F \r \n \x1a \n`.
const SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', 0x0D, 0x0A, 0x1A, 0x0A];
/// Head window covering every field this check reads (max end: v1 EOF
/// address at 44..52).
const HEAD_LEN: u64 = 64;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    let head = files::read_range(file, 0, HEAD_LEN).await?;
    if head.len() < 9 || head[0..8] != SIGNATURE {
        return Ok(Completeness::Incomplete);
    }
    // field offsets by superblock version: (size-of-offsets byte, first
    // address field — the base address)
    let version = head[8];
    let (so_offset, base_offset) = match version {
        0 => (13usize, 24usize),
        1 => (13, 28),
        2 | 3 => (9, 12),
        _ => return Ok(Completeness::Unknown),
    };
    if so_offset >= head.len() {
        return Ok(Completeness::Incomplete);
    }
    let so = head[so_offset];
    if so != 8 {
        return Ok(Completeness::Unknown);
    }
    // the EOF address is two address-sized fields past the base address
    let eof_offset = base_offset + 2 * so as usize;
    if eof_offset + 8 > head.len() {
        return Ok(Completeness::Incomplete);
    }
    let base_addr = u64_le(&head[base_offset..base_offset + 8]);
    if base_addr != 0 {
        return Ok(Completeness::Unknown);
    }
    let eof_addr = u64_le(&head[eof_offset..eof_offset + 8]);
    Ok(if eof_addr == size {
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
