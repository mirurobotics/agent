//! tar completeness: a finalized archive ends with two 512-byte zero blocks
//! (any further padding is also zeros), and its size is a multiple of the
//! 512-byte block size. No leading-magic check: pre-POSIX v7 tars have none
//! at a fixed offset.

// internal crates
use crate::fileformats::{errors::FileFormatsErr, Completeness};
use crate::filesys::{files, File};

/// tar block size.
const BLOCK_SIZE: u64 = 512;
/// The end-of-archive marker: two zero blocks.
const EOA_LEN: u64 = 1024;

pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr> {
    if size < EOA_LEN || !size.is_multiple_of(BLOCK_SIZE) {
        return Ok(Completeness::Incomplete);
    }
    let tail = files::read_tail(file, EOA_LEN).await?;
    let finalized = tail.len() == EOA_LEN as usize && tail.iter().all(|&b| b == 0x00);
    Ok(if finalized {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    })
}
