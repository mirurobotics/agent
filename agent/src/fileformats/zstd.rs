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

/// Where a frame hop landed: the offset just past the frame, or a verdict
/// that ends the walk.
enum Hop {
    End(u64),
    Incomplete,
    Unknown,
}

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
        let hop = if (SKIPPABLE_MAGIC_MIN..=SKIPPABLE_MAGIC_MAX).contains(&magic) {
            skippable_frame_end(&header, cursor, size)
        } else if magic == ZSTD_MAGIC {
            zstd_frame_end(file, &header, cursor, size, &mut reads).await?
        } else {
            return Ok(Completeness::Incomplete);
        };
        cursor = match hop {
            Hop::End(end) => end,
            Hop::Incomplete => return Ok(Completeness::Incomplete),
            Hop::Unknown => return Ok(Completeness::Unknown),
        };
    }
    // cursor == size: the frames tile the file exactly
    Ok(Completeness::Complete)
}

/// Skippable frame: magic (4) + frame_size (4) + frame_size payload bytes.
fn skippable_frame_end(header: &[u8], cursor: u64, size: u64) -> Hop {
    if header.len() < 8 {
        return Hop::Incomplete;
    }
    let frame_size = u32_le(&header[4..8]) as u64;
    match cursor.checked_add(8 + frame_size) {
        Some(end) if end <= size => Hop::End(end),
        _ => Hop::Incomplete,
    }
}

/// Standard frame: hop from the frame header across blocks, reading only the
/// 3-byte block headers, to the offset just past the frame.
async fn zstd_frame_end(
    file: &File,
    header: &[u8],
    cursor: u64,
    size: u64,
    reads: &mut u32,
) -> Result<Hop, FileFormatsErr> {
    if header.len() < 5 {
        return Ok(Hop::Incomplete);
    }
    let descriptor = header[4];
    let has_checksum = (descriptor >> 2) & 1 == 1;
    let mut pos = match cursor.checked_add(4 + frame_header_len(descriptor)) {
        Some(p) if p <= size => p,
        _ => return Ok(Hop::Incomplete),
    };
    loop {
        if *reads >= MAX_HEADER_READS {
            return Ok(Hop::Unknown);
        }
        *reads += 1;
        let block_header = files::read_range(file, pos, 3).await?;
        if block_header.len() < 3 {
            return Ok(Hop::Incomplete);
        }
        let (last_block, payload_len) = match block_payload(&block_header) {
            Some(block) => block,
            None => return Ok(Hop::Incomplete),
        };
        pos = match pos.checked_add(3 + payload_len) {
            Some(p) if p <= size => p,
            _ => return Ok(Hop::Incomplete),
        };
        if last_block {
            break;
        }
    }
    if has_checksum {
        pos = match pos.checked_add(4) {
            Some(p) if p <= size => p,
            _ => return Ok(Hop::Incomplete),
        };
    }
    Ok(Hop::End(pos))
}

/// Decode a 3-byte block header into (last_block, payload length); None for
/// the reserved block type.
fn block_payload(block_header: &[u8]) -> Option<(bool, u64)> {
    let raw = (block_header[0] as u32)
        | ((block_header[1] as u32) << 8)
        | ((block_header[2] as u32) << 16);
    let last_block = raw & 1 == 1;
    let block_size = (raw >> 3) as u64;
    let payload_len = match (raw >> 1) & 3 {
        // raw (0) / compressed (2) blocks carry block_size bytes
        0 | 2 => block_size,
        // an RLE block carries a single repeated byte
        1 => 1,
        // type 3 is reserved
        _ => return None,
    };
    Some((last_block, payload_len))
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
