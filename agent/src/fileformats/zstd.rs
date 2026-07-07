//! zstd completeness: the file is a sequence of frames whose headers declare
//! every payload length, so a capped header-hop walk (frame headers + block
//! headers, never payloads) can verify that the frames tile the file exactly.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// zstd frame magic (u32 LE).
const ZSTD_MAGIC: u32 = 0xFD2F_B528;
/// Skippable frame magics (u32 LE): `0x184D2A50 + x` for x in 0..=15.
const SKIPPABLE_MAGIC_MIN: u32 = 0x184D_2A50;
const SKIPPABLE_MAGIC_MAX: u32 = 0x184D_2A5F;
/// Frame-start window: magic (4) + max frame header (descriptor 1 + window
/// descriptor 1 + dictionary ID 4 + frame content size 8).
const FRAME_START_WINDOW: u64 = 18;
/// Global cap on header reads; a walk that exhausts it is not cheaply
/// verifiable.
const MAX_HEADER_READS: u32 = 1024;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size == 0 {
        return Ok(Completeness::Incomplete);
    }
    let mut reads = 0u32;
    let mut cursor = 0u64;
    while cursor < size {
        if reads >= MAX_HEADER_READS {
            return Ok(Completeness::Unknown);
        }
        reads += 1;
        let header = files::read_range(file, cursor, FRAME_START_WINDOW).await?;
        if header.len() < 4 {
            return Ok(Completeness::Incomplete);
        }
        let magic = u32_le(&header[0..4]);
        if (SKIPPABLE_MAGIC_MIN..=SKIPPABLE_MAGIC_MAX).contains(&magic) {
            // skippable frame: magic (4) + frame_size (4) + frame_size bytes
            if header.len() < 8 {
                return Ok(Completeness::Incomplete);
            }
            let frame_size = u32_le(&header[4..8]) as u64;
            cursor = match cursor.checked_add(8 + frame_size) {
                Some(c) if c <= size => c,
                _ => return Ok(Completeness::Incomplete),
            };
            continue;
        }
        if magic != ZSTD_MAGIC {
            return Ok(Completeness::Incomplete);
        }
        if header.len() < 5 {
            return Ok(Completeness::Incomplete);
        }
        let descriptor = header[4];
        let has_checksum = (descriptor >> 2) & 1 == 1;
        let mut pos = match cursor.checked_add(4 + frame_header_len(descriptor)) {
            Some(p) if p <= size => p,
            _ => return Ok(Completeness::Incomplete),
        };
        // hop blocks, reading only the 3-byte headers
        loop {
            if reads >= MAX_HEADER_READS {
                return Ok(Completeness::Unknown);
            }
            reads += 1;
            let block_header = files::read_range(file, pos, 3).await?;
            if block_header.len() < 3 {
                return Ok(Completeness::Incomplete);
            }
            let raw = (block_header[0] as u32)
                | ((block_header[1] as u32) << 8)
                | ((block_header[2] as u32) << 16);
            let last_block = raw & 1 == 1;
            let block_type = (raw >> 1) & 3;
            let block_size = (raw >> 3) as u64;
            let payload_len = match block_type {
                // raw (0) / compressed (2) blocks carry block_size bytes
                0 | 2 => block_size,
                // an RLE block carries a single repeated byte
                1 => 1,
                // type 3 is reserved
                _ => return Ok(Completeness::Incomplete),
            };
            pos = match pos.checked_add(3 + payload_len) {
                Some(p) if p <= size => p,
                _ => return Ok(Completeness::Incomplete),
            };
            if last_block {
                break;
            }
        }
        if has_checksum {
            pos = match pos.checked_add(4) {
                Some(p) if p <= size => p,
                _ => return Ok(Completeness::Incomplete),
            };
        }
        cursor = pos;
    }
    // cursor == size: the frames tile the file exactly
    Ok(Completeness::Complete)
}

/// Length of the frame header past the magic: descriptor (1) + optional
/// window descriptor + dictionary ID + frame content size, all declared by
/// the descriptor byte.
fn frame_header_len(descriptor: u8) -> u64 {
    let fcs_flag = descriptor >> 6;
    let single_segment = (descriptor >> 5) & 1;
    let dict_flag = descriptor & 3;
    let window_len = if single_segment == 0 { 1 } else { 0 };
    let dict_len = [0u64, 1, 2, 4][dict_flag as usize];
    let fcs_len = match fcs_flag {
        0 => {
            if single_segment == 1 {
                1
            } else {
                0
            }
        }
        1 => 2,
        2 => 4,
        _ => 8,
    };
    1 + window_len + dict_len + fcs_len
}

fn u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
