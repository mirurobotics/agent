// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::{self, Completeness};
use miru_agent::filesys::{dirs, files, WriteOptions};

/// SQLite database of `file_len` zero-padded bytes with the magic, the
/// big-endian page-size field, and the big-endian page count in place.
fn sqlite_db(page_size_field: u16, page_count: u32, file_len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; file_len];
    bytes[0..16].copy_from_slice(b"SQLite format 3\0");
    bytes[16..18].copy_from_slice(&page_size_field.to_be_bytes());
    bytes[28..32].copy_from_slice(&page_count.to_be_bytes());
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        assert_eq!(
            check_verdict("f.db3", &sqlite_db(512, 1, 512)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn page_size_one_means_65536() {
        // the header encodes a 65536-byte page size as the value 1
        assert_eq!(
            check_verdict("f.db3", &sqlite_db(1, 1, 65_536)).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn page_math_mismatch() {
        // 512 bytes/page * 1 page != 100-byte file
        assert_eq!(
            check_verdict("f.db3", &sqlite_db(512, 1, 100)).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn wal_sidecar() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let db = dir.file("data.db3");
        files::write_bytes(&db, &sqlite_db(512, 1, 512), WriteOptions::default())
            .await
            .unwrap();
        let wal = dir.file("data.db3-wal");
        files::write_bytes(&wal, b"", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            fileformats::check(&db).await.unwrap(),
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn journal_sidecar() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let db = dir.file("data.db3");
        files::write_bytes(&db, &sqlite_db(512, 1, 512), WriteOptions::default())
            .await
            .unwrap();
        let journal = dir.file("data.db3-journal");
        files::write_bytes(&journal, b"", WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            fileformats::check(&db).await.unwrap(),
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_magic() {
        assert_eq!(
            check_verdict("f.db3", &[0xAB; 512]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.db3", &sqlite_db(512, 1, 512)[..99]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.db3", b"").await, Completeness::Incomplete);
    }
}
