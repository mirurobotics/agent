// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// Parquet file of `total_len` bytes: leading magic + zero filler +
/// `footer_len: u32 LE` + trailing magic.
fn parquet(footer_len: u32, total_len: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PAR1");
    bytes.resize(total_len - 8, 0x00);
    bytes.extend_from_slice(&footer_len.to_le_bytes());
    bytes.extend_from_slice(b"PAR1");
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        assert_eq!(
            check_verdict("f.parquet", &parquet(4, 16)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn minimal() {
        assert_eq!(
            check_verdict("f.parquet", &parquet(0, 12)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn footer_len_boundary() {
        // footer_len + 8 == size pins the `<=` boundary
        assert_eq!(
            check_verdict("f.parquet", &parquet(8, 16)).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn footer_len_too_large() {
        // footer_len + 8 > size: the declared footer cannot fit in the file
        assert_eq!(
            check_verdict("f.parquet", &parquet(9, 16)).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn missing_trailing_magic() {
        let mut bytes = parquet(4, 16);
        let len = bytes.len();
        bytes[len - 4..].copy_from_slice(b"PARX");
        assert_eq!(
            check_verdict("f.parquet", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn missing_leading_magic() {
        let mut bytes = parquet(4, 16);
        bytes[..4].copy_from_slice(b"XXXX");
        assert_eq!(
            check_verdict("f.parquet", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn garbage() {
        assert_eq!(
            check_verdict("f.parquet", &[0xAB; 16]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.parquet", &parquet(0, 12)[..11]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(
            check_verdict("f.parquet", b"").await,
            Completeness::Incomplete
        );
    }
}
