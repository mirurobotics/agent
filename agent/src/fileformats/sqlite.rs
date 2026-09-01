//! SQLite (including ROS 2 bag) completeness: the database header declares
//! the page size and page count, whose product must equal the file size, and
//! a hot `-wal` / `-journal` sidecar means unflushed state.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File, PathExt};

/// Leading magic: `SQLite format 3\0`.
const MAGIC: [u8; 16] = *b"SQLite format 3\0";
/// The database header is the first 100 bytes.
const HEADER_LEN: u64 = 100;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < HEADER_LEN {
        return Ok(Completeness::Incomplete);
    }
    let head = files::read_range(file, 0, HEADER_LEN).await?;
    if head.len() < HEADER_LEN as usize || head[0..16] != MAGIC {
        return Ok(Completeness::Incomplete);
    }
    // a hot journal or write-ahead log means unflushed state
    if sidecar(file, "-wal").exists() || sidecar(file, "-journal").exists() {
        return Ok(Completeness::Incomplete);
    }
    let page_size = u16::from_be_bytes([head[16], head[17]]);
    // the header encodes a 65536-byte page size as the value 1
    let page_size_bytes = if page_size == 1 {
        65_536u64
    } else {
        page_size as u64
    };
    let page_count = u32::from_be_bytes([head[28], head[29], head[30], head[31]]) as u64;
    Ok(if page_size_bytes * page_count == size {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}

/// Builds the sibling path formed by appending `suffix` to the full filename
/// (e.g. `data.db3` -> `data.db3-wal`).
fn sidecar(file: &File, suffix: &str) -> File {
    let mut path = file.path().clone().into_os_string();
    path.push(suffix);
    File::new(path)
}
