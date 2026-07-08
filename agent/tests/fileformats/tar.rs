// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        // one 512-byte header block + the two-zero-block end-of-archive marker
        let mut bytes = vec![0x41; 512];
        bytes.extend_from_slice(&[0u8; 1024]);
        assert_eq!(check_verdict("f.tar", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn all_zero_min() {
        // an empty archive is just the end-of-archive marker
        assert_eq!(
            check_verdict("f.tar", &[0u8; 1024]).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn size_not_block_multiple() {
        assert_eq!(
            check_verdict("f.tar", &[0u8; 1025]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn nonzero_tail() {
        let mut bytes = vec![0u8; 1536];
        bytes[1535] = 0x01;
        assert_eq!(
            check_verdict("f.tar", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn garbage() {
        // block-aligned but the tail is not all zeros
        assert_eq!(
            check_verdict("f.tar", &[0xAB; 1024]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.tar", &[0u8; 1023]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.tar", b"").await, Completeness::Incomplete);
    }
}
