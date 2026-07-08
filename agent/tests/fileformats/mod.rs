// internal crates
use miru_agent::fileformats::{self, Completeness};
use miru_agent::filesys::{dirs, files, WriteOptions};

pub mod avi;
pub mod check;
pub mod gzip;
pub mod hdf5;
pub mod jpeg;
pub mod matroska;
pub mod mcap;
pub mod mp4;
pub mod parquet;
pub mod png;
pub mod rosbag;
pub mod sqlite;
pub mod tar;
pub mod zip;
pub mod zstd;

/// Writes `bytes` to `file_name` in a fresh temp dir and returns the verdict
/// of `fileformats::check` on it.
pub async fn check_verdict(file_name: &str, bytes: &[u8]) -> Completeness {
    let dir = dirs::create_temp("testing").await.unwrap();
    let file = dir.file(file_name);
    files::write_bytes(&file, bytes, WriteOptions::default())
        .await
        .unwrap();
    fileformats::check(&file).await.unwrap()
}
