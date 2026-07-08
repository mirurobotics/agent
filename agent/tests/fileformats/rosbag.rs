// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// Leading magic: `#ROSBAG V2.0\n`.
const MAGIC: &[u8] = b"#ROSBAG V2.0\n";

/// A file-header field: `field_len: u32 LE` + body.
fn field(body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes
}

/// A bag file: magic + `header_len: u32 LE` + the file-header block.
fn bag(header: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
    bytes.extend_from_slice(header);
    bytes
}

/// The body of an `index_pos` field with the given `u64 LE` value.
fn index_pos_body(v: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"index_pos=");
    bytes.extend_from_slice(&v.to_le_bytes());
    bytes
}

/// A finalized bag (59 bytes): the header's middle field has no `=` (covers
/// the missing-separator skip) and `index_pos` is 40 (0 < 40 <= 59).
fn valid_bag() -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(&field(b"op=\x03"));
    header.extend_from_slice(&field(b"noequals"));
    header.extend_from_slice(&field(&index_pos_body(40)));
    bag(&header)
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        assert_eq!(
            check_verdict("f.bag", &valid_bag()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn index_pos_equal_to_size() {
        // magic (13) + header_len (4) + field (22) = 39 bytes; index_pos == 39
        // pins the `<=` boundary
        let bytes = bag(&field(&index_pos_body(39)));
        assert_eq!(check_verdict("f.bag", &bytes).await, Completeness::Complete);
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn index_pos_zero() {
        let bytes = bag(&field(&index_pos_body(0)));
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn index_pos_past_eof() {
        let bytes = bag(&field(&index_pos_body(100_000)));
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn index_pos_field_missing() {
        let bytes = bag(&field(b"chunk_count=\x01\x00\x00\x00"));
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn index_pos_value_wrong_width() {
        // the value must be exactly 8 bytes; this one is 4
        let bytes = bag(&field(b"index_pos=\x01\x02\x03\x04"));
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn field_len_overruns_block() {
        // field_len u32::MAX would have overflowed the pre-fix `pos + field_len`
        // comparison; the wrap-proof guard must reject it
        let mut header = vec![0xFF, 0xFF, 0xFF, 0xFF];
        header.extend_from_slice(&[0x00; 4]);
        let bytes = bag(&header);
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn trailing_partial_field_len() {
        // two stray bytes at the end of the block: pos + 4 > header.len()
        let mut header = field(b"noequals");
        header.extend_from_slice(&[0xAB, 0xAB]);
        let bytes = bag(&header);
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn header_block_truncated() {
        // header_len declares 100 bytes but only 20 follow
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(&[0x00; 20]);
        assert_eq!(
            check_verdict("f.bag", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_magic() {
        assert_eq!(
            check_verdict("f.bag", &[0xAB; 32]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.bag", &bag(b"")[..16]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.bag", b"").await, Completeness::Incomplete);
    }
}

pub mod unknown {
    use super::*;

    #[tokio::test]
    async fn header_len_implausible() {
        // a 9000-byte header block exceeds the 8192-byte plausibility cap
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&9000u32.to_le_bytes());
        assert_eq!(check_verdict("f.bag", &bytes).await, Completeness::Unknown);
    }
}
