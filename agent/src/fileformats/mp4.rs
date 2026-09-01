//! MP4/MOV completeness: the file is a sequence of top-level boxes whose
//! declared sizes must tile the file exactly, and a finalized file contains a
//! `moov` box (writers emit it on clean close).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// The movie box, written on clean close.
const MOOV: [u8; 4] = *b"moov";
/// Minimum box header: size (4) + fourcc (4).
const MIN_SIZE: u64 = 8;
/// Cap on the header-hop walk; real files have well under 100 top-level
/// boxes.
const MAX_BOXES: u32 = 10_000;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let mut cursor = 0u64;
    let mut moov_seen = false;
    let mut boxes = 0u32;
    while cursor < size {
        if boxes >= MAX_BOXES {
            return Ok(Completeness::Unknown);
        }
        boxes += 1;
        let header = files::read_range(file, cursor, 16).await?;
        if header.len() < 8 {
            return Ok(Completeness::Incomplete);
        }
        if header[4..8] == MOOV {
            moov_seen = true;
        }
        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let box_size = match size32 {
            // the box extends to EOF; legal only as the final box, so it
            // tiles by definition
            0 => size - cursor,
            // a largesize (which includes the 16-byte header) follows
            1 => {
                if header.len() < 16 {
                    return Ok(Completeness::Incomplete);
                }
                let largesize = u64_be(&header[8..16]);
                if largesize < 16 {
                    return Ok(Completeness::Incomplete);
                }
                largesize
            }
            2..=7 => return Ok(Completeness::Incomplete),
            n => n,
        };
        cursor = match cursor.checked_add(box_size) {
            Some(c) if c <= size => c,
            _ => return Ok(Completeness::Incomplete),
        };
    }
    // cursor == size: the boxes tile the file exactly
    Ok(if moov_seen {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}

fn u64_be(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(buf)
}
