//! ROS 1 bag completeness: the file-header record's `index_pos` field holds
//! the offset of the index data section, written only on clean close (it is
//! 0 while recording).

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// Leading magic: `#ROSBAG V2.0\n`.
const MAGIC: [u8; 13] = *b"#ROSBAG V2.0\n";
/// Magic (13) + file-header record length field (4).
const MIN_SIZE: u64 = 17;
/// A file-header block larger than this is implausible and not cheaply
/// verifiable.
const MAX_HEADER_LEN: u32 = 8192;
/// Name of the file-header field holding the index section offset.
const INDEX_POS_FIELD: &[u8] = b"index_pos";

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < MIN_SIZE {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, MIN_SIZE).await?;
    if head.len() < MIN_SIZE as usize || head[0..13] != MAGIC {
        return Ok(Completeness::Incomplete);
    }
    let header_len = u32::from_le_bytes([head[13], head[14], head[15], head[16]]);
    if header_len > MAX_HEADER_LEN {
        return Ok(Completeness::Unknown);
    }
    let header = files::read_range(file, MIN_SIZE, header_len as u64).await?;
    if header.len() < header_len as usize {
        return Ok(Completeness::Incomplete);
    }
    match find_index_pos(&header) {
        Some(index_pos) => Ok(if index_pos > 0 && index_pos <= size {
            Completeness::Complete
        } else {
            Completeness::Incomplete
        }),
        // field missing or field parse ran off the block
        None => Ok(Completeness::Incomplete),
    }
}

/// Scans the file-header block's fields (`field_len: u32 LE` + `name=value`)
/// for the `index_pos` field and returns its `u64 LE` value. Returns `None`
/// if the field is missing, has a value that is not 8 bytes, or the field
/// parse overruns the block.
fn find_index_pos(header: &[u8]) -> Option<u64> {
    let mut pos = 0usize;
    while pos < header.len() {
        if pos + 4 > header.len() {
            return None;
        }
        let field_len = u32::from_le_bytes([
            header[pos],
            header[pos + 1],
            header[pos + 2],
            header[pos + 3],
        ]) as usize;
        pos += 4;
        if pos + field_len > header.len() {
            return None;
        }
        let field = &header[pos..pos + field_len];
        pos += field_len;
        let sep = match field.iter().position(|&b| b == b'=') {
            Some(sep) => sep,
            None => continue,
        };
        if &field[..sep] != INDEX_POS_FIELD {
            continue;
        }
        let value = &field[sep + 1..];
        if value.len() != 8 {
            return None;
        }
        let mut buf = [0u8; 8];
        buf.copy_from_slice(value);
        return Some(u64::from_le_bytes(buf));
    }
    None
}
