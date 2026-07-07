//! Cheap file-completeness (finalization) checks.
//!
//! Many file formats carry an explicit finalization marker — a footer record,
//! end-of-stream blocks, or a self-describing size field — that a writer emits
//! only on clean close. Each check here verifies that marker by reading a
//! small fixed-size head and/or tail, plus at most a bounded header-hop walk
//! for container formats. No check ever reads or hashes a whole file.

// internal crates
use crate::filesys::{files, File, PathExt};

pub mod errors;

mod avi;
mod gzip;
mod hdf5;
mod jpeg;
mod matroska;
mod mcap;
mod mp4;
mod parquet;
mod png;
mod rosbag;
mod sqlite;
mod tar;
mod zip;
mod zstd;

pub use self::errors::FileFormatsErr;

/// Verdict of a cheap file-finalization check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    /// Finalization marker present and consistent with the file size.
    Complete,
    /// Format recognized; marker missing or inconsistent (truncated, still
    /// being written, or garbage).
    Incomplete,
    /// Extension not recognized, or format not cheaply verifiable.
    Unknown,
}

/// Checks whether `file` looks like a finalized instance of the format its
/// extension claims. Detection is by extension (case-insensitive); files with
/// no extension or an unrecognized one return [`Completeness::Unknown`].
/// Parse anomalies are verdicts, never errors — only real I/O failures
/// return `Err`.
pub async fn check(file: &File) -> Result<Completeness, FileFormatsErr> {
    let size = files::size(file).await?;
    let ext = match file.path().extension().and_then(|e| e.to_str()) {
        Some(ext) => ext.to_lowercase(),
        None => return Ok(Completeness::Unknown),
    };
    match ext.as_str() {
        "mcap" => mcap::check(file, size).await,
        "parquet" => parquet::check(file, size).await,
        "bag" => rosbag::check(file, size).await,
        "db" | "db3" | "sqlite" | "sqlite3" => sqlite::check(file, size).await,
        "mp4" | "mov" | "m4v" => mp4::check(file, size).await,
        "jpg" | "jpeg" => jpeg::check(file, size).await,
        "png" => png::check(file, size).await,
        "zip" => zip::check(file, size).await,
        "gz" | "tgz" => gzip::check(file, size).await,
        "tar" => tar::check(file, size).await,
        "zst" => zstd::check(file, size).await,
        "h5" | "hdf5" => hdf5::check(file, size).await,
        "mkv" | "webm" => matroska::check(file, size).await,
        "avi" => avi::check(file, size).await,
        _ => Ok(Completeness::Unknown),
    }
}
