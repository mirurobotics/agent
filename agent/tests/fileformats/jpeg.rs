// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// Start-of-image marker.
const SOI: [u8; 2] = [0xFF, 0xD8];
/// End-of-image marker.
const EOI: [u8; 2] = [0xFF, 0xD9];

/// A finalized JPEG (14 bytes): SOI + nonzero payload + EOI.
fn valid_jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SOI);
    bytes.extend_from_slice(&[0x11; 10]);
    bytes.extend_from_slice(&EOI);
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        assert_eq!(
            check_verdict("f.jpg", &valid_jpeg()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn minimal() {
        assert_eq!(
            check_verdict("f.jpg", &[0xFF, 0xD8, 0xFF, 0xD9]).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn trailing_nuls() {
        // some writers append a few NUL padding bytes after the EOI
        let mut bytes = valid_jpeg();
        bytes.extend_from_slice(&[0x00; 4]);
        assert_eq!(check_verdict("f.jpg", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn trailing_nuls_at_window_boundary() {
        // 14 NULs: the EOI still lands inside the 16-byte tail window
        let mut bytes = valid_jpeg();
        bytes.extend_from_slice(&[0x00; 14]);
        assert_eq!(check_verdict("f.jpg", &bytes).await, Completeness::Complete);
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn trailing_nuls_beyond_window() {
        // 15 NULs push all but one EOI byte out of the 16-byte tail window
        let mut bytes = valid_jpeg();
        bytes.extend_from_slice(&[0x00; 15]);
        assert_eq!(
            check_verdict("f.jpg", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn missing_eoi() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SOI);
        bytes.extend_from_slice(&[0x11; 14]);
        assert_eq!(
            check_verdict("f.jpg", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn all_nul_tail() {
        // a tail window of only NULs trims to nothing
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SOI);
        bytes.extend_from_slice(&[0x00; 20]);
        assert_eq!(
            check_verdict("f.jpg", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_soi() {
        assert_eq!(
            check_verdict("f.jpg", &[0xAB; 32]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.jpg", &[0xFF, 0xD8, 0xFF]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.jpg", b"").await, Completeness::Incomplete);
    }
}
