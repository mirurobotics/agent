// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// EOCD record signature.
const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];

/// An EOCD record: signature + 16 zero bytes + `comment_len: u16 LE` +
/// comment.
fn eocd(comment: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&EOCD_SIG);
    bytes.extend_from_slice(&[0u8; 16]);
    bytes.extend_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn minimal_eocd() {
        assert_eq!(
            check_verdict("f.zip", &eocd(b"")).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn with_comment() {
        assert_eq!(
            check_verdict("f.zip", &eocd(b"hello")).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn with_preceding_data() {
        // the backward scan skips over payload bytes before the EOCD
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PK\x03\x04");
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.extend_from_slice(&eocd(b""));
        assert_eq!(check_verdict("f.zip", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn decoy_eocd_before_real() {
        // a decoy EOCD whose comment_len does not run to EOF precedes the
        // real one; the rear-most valid candidate wins
        let mut bytes = eocd(b"");
        bytes[20..22].copy_from_slice(&9u16.to_le_bytes());
        bytes.extend_from_slice(&eocd(b""));
        assert_eq!(check_verdict("f.zip", &bytes).await, Completeness::Complete);
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn comment_len_mismatch() {
        // comment_len declares 5 bytes but no comment follows
        let mut bytes = eocd(b"");
        bytes[20..22].copy_from_slice(&5u16.to_le_bytes());
        assert_eq!(
            check_verdict("f.zip", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn truncated() {
        let mut bytes = eocd(b"hello");
        bytes.pop();
        assert_eq!(
            check_verdict("f.zip", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn no_eocd() {
        assert_eq!(
            check_verdict("f.zip", &[0xAB; 64]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.zip", &eocd(b"")[..21]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.zip", b"").await, Completeness::Incomplete);
    }
}
