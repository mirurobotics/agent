// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// Leading and trailing MCAP magic bytes.
const MAGIC: [u8; 8] = [0x89, b'M', b'C', b'A', b'P', b'0', b'\r', b'\n'];

/// Footer record (opcode + length + 20-byte payload) plus trailing magic.
fn footer_tail() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0x02);
    bytes.extend_from_slice(&20u64.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 20]);
    bytes.extend_from_slice(&MAGIC);
    bytes
}

/// Minimal finalized MCAP: leading magic + footer + trailing magic (45 bytes).
fn valid_mcap() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&footer_tail());
    bytes
}

/// Finalized MCAP with one record between the magic and the footer (62 bytes).
fn valid_mcap_with_record() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    bytes.push(0x01);
    bytes.extend_from_slice(&8u64.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.extend_from_slice(&footer_tail());
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn minimal() {
        assert_eq!(
            check_verdict("f.mcap", &valid_mcap()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn with_record() {
        assert_eq!(
            check_verdict("f.mcap", &valid_mcap_with_record()).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn truncated() {
        let mut bytes = valid_mcap_with_record();
        bytes.pop();
        assert_eq!(
            check_verdict("f.mcap", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn wrong_footer_opcode() {
        let mut bytes = valid_mcap();
        bytes[8] = 0x03;
        assert_eq!(
            check_verdict("f.mcap", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn wrong_footer_length() {
        let mut bytes = valid_mcap();
        bytes[9..17].copy_from_slice(&19u64.to_le_bytes());
        assert_eq!(
            check_verdict("f.mcap", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn corrupt_trailing_magic() {
        let mut bytes = valid_mcap();
        let last = bytes.len() - 1;
        bytes[last] = 0x00;
        assert_eq!(
            check_verdict("f.mcap", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_leading_magic() {
        let mut bytes = valid_mcap();
        bytes[0] = 0x00;
        assert_eq!(
            check_verdict("f.mcap", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn garbage() {
        assert_eq!(
            check_verdict("f.mcap", &[0xAB; 64]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.mcap", &valid_mcap()[..44]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.mcap", b"").await, Completeness::Incomplete);
    }
}
