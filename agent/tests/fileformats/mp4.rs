// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// A top-level box: `size: u32 BE` (header included) + fourcc + payload.
fn mp4_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    bytes.extend_from_slice(fourcc);
    bytes.extend_from_slice(payload);
    bytes
}

/// A finalized MP4 (40 bytes): ftyp + moov + mdat, tiling the file exactly.
fn valid_mp4() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&mp4_box(b"ftyp", &[0u8; 8]));
    bytes.extend_from_slice(&mp4_box(b"moov", &[]));
    bytes.extend_from_slice(&mp4_box(b"mdat", &[0u8; 8]));
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        assert_eq!(
            check_verdict("f.mp4", &valid_mp4()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn size_zero_final_box() {
        // a size-0 final box legally extends to EOF
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&mp4_box(b"ftyp", &[0u8; 8]));
        bytes.extend_from_slice(&mp4_box(b"moov", &[]));
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(check_verdict("f.mp4", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn size_zero_box_is_moov() {
        // the moov flag is recorded before size handling
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&mp4_box(b"ftyp", &[0u8; 8]));
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"moov");
        assert_eq!(check_verdict("f.mp4", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn largesize_box() {
        // size32 == 1: a u64 largesize (header included) follows the fourcc
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&mp4_box(b"moov", &[]));
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&24u64.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(check_verdict("f.mp4", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn cap_boundary_ten_thousand_boxes() {
        // exactly 10,000 boxes: the walk cap is `>`, not `>=`
        let mut bytes = mp4_box(b"free", &[]).repeat(9_999);
        bytes.extend_from_slice(&mp4_box(b"moov", &[]));
        assert_eq!(check_verdict("f.mp4", &bytes).await, Completeness::Complete);
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn tiling_without_moov() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&mp4_box(b"ftyp", &[0u8; 8]));
        bytes.extend_from_slice(&mp4_box(b"mdat", &[0u8; 8]));
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn size_zero_final_box_without_moov() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&mp4_box(b"ftyp", &[0u8; 8]));
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"mdat");
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn truncated() {
        let mut bytes = valid_mp4();
        bytes.truncate(bytes.len() - 4);
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn box_overruns_file() {
        // a single box declaring 64 bytes in a 16-byte file
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x40]);
        bytes.extend_from_slice(b"moov");
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn largesize_below_16() {
        // a largesize includes the 16-byte header, so < 16 is invalid
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&8u64.to_be_bytes());
        bytes.extend_from_slice(&mp4_box(b"moov", &[]));
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn largesize_header_truncated() {
        // size32 == 1 but the file ends before the 16-byte extended header
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&[0u8; 4]);
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn largesize_overflow() {
        // u64::MAX overflows the cursor's checked_add
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn size_in_reserved_range() {
        // size32 values 2..=7 are invalid
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x04]);
        bytes.extend_from_slice(b"free");
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn trailing_partial_header() {
        // 4 stray bytes after the last box cannot hold a box header
        let mut bytes = mp4_box(b"moov", &[]);
        bytes.extend_from_slice(&[0u8; 4]);
        assert_eq!(
            check_verdict("f.mp4", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn garbage() {
        assert_eq!(
            check_verdict("f.mp4", &[0xAB; 32]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.mp4", &[0xAB; 7]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.mp4", b"").await, Completeness::Incomplete);
    }
}

pub mod unknown {
    use super::*;

    #[tokio::test]
    async fn cap_exceeded() {
        // 10,001 boxes exhaust the walk cap
        let bytes = mp4_box(b"free", &[]).repeat(10_001);
        assert_eq!(check_verdict("f.mp4", &bytes).await, Completeness::Unknown);
    }
}
