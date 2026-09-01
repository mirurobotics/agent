// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// PNG signature.
const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
/// The complete IEND chunk: zero length + `IEND` + constant CRC.
const IEND: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
];

/// A finalized PNG (45 bytes): signature + IHDR chunk + IEND chunk. The IHDR
/// CRC value is irrelevant to the check.
fn valid_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SIGNATURE);
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&[0u8; 13]);
    bytes.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    bytes.extend_from_slice(&IEND);
    bytes
}

/// The minimal finalized PNG (20 bytes): signature + IEND chunk.
fn minimal_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SIGNATURE);
    bytes.extend_from_slice(&IEND);
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        assert_eq!(
            check_verdict("f.png", &valid_png()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn minimal() {
        assert_eq!(
            check_verdict("f.png", &minimal_png()).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn truncated() {
        let mut bytes = valid_png();
        bytes.pop();
        assert_eq!(
            check_verdict("f.png", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_signature() {
        let mut bytes = valid_png();
        bytes[0] = 0x00;
        assert_eq!(
            check_verdict("f.png", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn garbage() {
        assert_eq!(
            check_verdict("f.png", &[0xAB; 45]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.png", &minimal_png()[..19]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.png", b"").await, Completeness::Incomplete);
    }
}
