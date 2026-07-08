// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// EBML header element ID.
const EBML_ID: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// Segment element ID.
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];

/// An EBML document: EBML header (4-byte payload) + Segment with the given
/// size vint and payload.
fn mkv(segment_size_vint: &[u8], segment_payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&EBML_ID);
    bytes.push(0x84); // header size vint: 4-byte payload
    bytes.extend_from_slice(&[0x11; 4]);
    bytes.extend_from_slice(&SEGMENT_ID);
    bytes.extend_from_slice(segment_size_vint);
    bytes.extend_from_slice(segment_payload);
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        // 1-byte size vint declaring 8: payload runs exactly to EOF
        assert_eq!(
            check_verdict("f.mkv", &mkv(&[0x88], &[0u8; 8])).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn valid_multibyte_size_vint() {
        // 2-byte size vint declaring 8: covers multi-byte value accumulation
        assert_eq!(
            check_verdict("f.mkv", &mkv(&[0x40, 0x08], &[0u8; 8])).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn unknown_size_segment_vint() {
        // an unknown-size Segment is exactly the still-recording state
        assert_eq!(
            check_verdict("f.mkv", &mkv(&[0xFF], &[0u8; 8])).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn unknown_size_8_byte_vint() {
        let vint = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(
            check_verdict("f.mkv", &mkv(&vint, &[0u8; 8])).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn segment_size_mismatch() {
        // the Segment declares 8 payload bytes but only 4 follow
        assert_eq!(
            check_verdict("f.mkv", &mkv(&[0x88], &[0u8; 4])).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_ebml_id() {
        assert_eq!(
            check_verdict("f.mkv", &[0xAB; 32]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.mkv", b"").await, Completeness::Incomplete);
    }
}

pub mod unknown {
    use super::*;

    #[tokio::test]
    async fn header_vint_truncated() {
        assert_eq!(
            check_verdict("f.mkv", &EBML_ID).await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn header_vint_zero_width() {
        let mut bytes = EBML_ID.to_vec();
        bytes.push(0x00);
        assert_eq!(check_verdict("f.mkv", &bytes).await, Completeness::Unknown);
    }

    #[tokio::test]
    async fn header_size_unknown() {
        // an unknown-size EBML header is an inconclusive parse
        let mut bytes = EBML_ID.to_vec();
        bytes.push(0xFF);
        assert_eq!(check_verdict("f.mkv", &bytes).await, Completeness::Unknown);
    }

    #[tokio::test]
    async fn header_size_implausible() {
        // a 2-byte vint declaring an 8000-byte EBML header exceeds the cap
        let mut bytes = EBML_ID.to_vec();
        bytes.extend_from_slice(&[0x5F, 0x40]);
        assert_eq!(check_verdict("f.mkv", &bytes).await, Completeness::Unknown);
    }

    #[tokio::test]
    async fn second_element_not_segment() {
        let mut bytes = EBML_ID.to_vec();
        bytes.push(0x80); // zero-length EBML header payload
        bytes.extend_from_slice(b"ABCD");
        bytes.extend_from_slice(&[0x11; 4]);
        assert_eq!(check_verdict("f.mkv", &bytes).await, Completeness::Unknown);
    }

    #[tokio::test]
    async fn ends_after_ebml_header() {
        let mut bytes = EBML_ID.to_vec();
        bytes.push(0x80);
        assert_eq!(check_verdict("f.mkv", &bytes).await, Completeness::Unknown);
    }

    #[tokio::test]
    async fn segment_size_vint_truncated() {
        let mut bytes = EBML_ID.to_vec();
        bytes.push(0x80);
        bytes.extend_from_slice(&SEGMENT_ID);
        assert_eq!(check_verdict("f.mkv", &bytes).await, Completeness::Unknown);
    }
}
