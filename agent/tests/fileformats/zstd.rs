// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// zstd frame magic (little-endian bytes of 0xFD2FB528).
const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// The lowest skippable-frame magic (little-endian bytes of 0x184D2A50).
const SKIPPABLE_MAGIC: [u8; 4] = [0x50, 0x2A, 0x4D, 0x18];

/// A 3-byte block header: `last` bit + 2-bit block type + 21-bit size.
fn block_header(last: bool, block_type: u32, size: u32) -> [u8; 3] {
    let raw = (last as u32) | (block_type << 1) | (size << 3);
    let le = raw.to_le_bytes();
    [le[0], le[1], le[2]]
}

/// The minimal finalized frame (9 bytes): magic + descriptor + window byte +
/// an empty last raw block.
fn minimal_frame() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    bytes.push(0x00);
    bytes.push(0x00);
    bytes.extend_from_slice(&block_header(true, 0, 0));
    bytes
}

/// A skippable frame: magic + `frame_size: u32 LE` + payload.
fn skippable(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SKIPPABLE_MAGIC);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn minimal_frame_only() {
        assert_eq!(
            check_verdict("f.zst", &minimal_frame()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn raw_block_with_payload() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 0, 4));
        bytes.extend_from_slice(&[0x11; 4]);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn compressed_block() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 2, 4));
        bytes.extend_from_slice(&[0x11; 4]);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn rle_block() {
        // an RLE block carries a single payload byte regardless of its size
        // field
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 1, 100));
        bytes.push(0x11);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn multi_block() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(false, 0, 2));
        bytes.extend_from_slice(&[0x11; 2]);
        bytes.extend_from_slice(&block_header(true, 0, 0));
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn multi_frame() {
        let bytes = minimal_frame().repeat(2);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn with_checksum() {
        // descriptor 0x04 declares a 4-byte content checksum after the blocks
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x04);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 0, 0));
        bytes.extend_from_slice(&[0u8; 4]);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn single_segment_frame() {
        // descriptor 0x20: no window byte, one frame-content-size byte
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x20);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 0, 0));
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn fcs_field_widths() {
        // fcs_flag 1/2/3 declare 2/4/8 frame-content-size bytes
        for (descriptor, fcs_len) in [(0x40u8, 2usize), (0x80, 4), (0xC0, 8)] {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&MAGIC);
            bytes.push(descriptor);
            bytes.push(0x00);
            bytes.extend_from_slice(&vec![0u8; fcs_len]);
            bytes.extend_from_slice(&block_header(true, 0, 0));
            assert_eq!(
                check_verdict("f.zst", &bytes).await,
                Completeness::Complete,
                "descriptor: {descriptor:#04x}"
            );
        }
    }

    #[tokio::test]
    async fn dict_id_widths() {
        // dict flags 1/2/3 declare 1/2/4 dictionary-ID bytes
        for (descriptor, dict_len) in [(0x01u8, 1usize), (0x02, 2), (0x03, 4)] {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&MAGIC);
            bytes.push(descriptor);
            bytes.push(0x00);
            bytes.extend_from_slice(&vec![0u8; dict_len]);
            bytes.extend_from_slice(&block_header(true, 0, 0));
            assert_eq!(
                check_verdict("f.zst", &bytes).await,
                Completeness::Complete,
                "descriptor: {descriptor:#04x}"
            );
        }
    }

    #[tokio::test]
    async fn skippable_frames_only() {
        // a file of only skippable frames still tiles exactly
        let bytes = skippable(b"meta").repeat(2);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn skippable_then_normal() {
        let mut bytes = skippable(b"");
        bytes.extend_from_slice(&minimal_frame());
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }

    #[tokio::test]
    async fn frame_cap_boundary() {
        // exactly 1024 header reads: the cap is `>=` after the last read
        let bytes = skippable(b"").repeat(1024);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Complete);
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn truncated_frame() {
        let mut bytes = minimal_frame();
        bytes.pop();
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn block_payload_overruns() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 0, 100));
        bytes.extend_from_slice(&[0x11; 2]);
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn reserved_block_type() {
        // block type 3 is reserved
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 3, 0));
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn checksum_missing() {
        // descriptor 0x04 declares a checksum, but the file ends at the block
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x04);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(true, 0, 0));
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn frame_header_overruns() {
        // descriptor 0xC0 needs a window byte plus 8 FCS bytes that are absent
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0xC0);
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn magic_only() {
        assert_eq!(
            check_verdict("f.zst", &MAGIC).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn trailing_bytes_after_frame() {
        // two stray bytes cannot start another frame
        let mut bytes = minimal_frame();
        bytes.extend_from_slice(&[0x11; 2]);
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn skippable_truncated_size() {
        // the skippable frame's size field itself is cut short
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SKIPPABLE_MAGIC);
        bytes.extend_from_slice(&[0x00; 2]);
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn skippable_overruns() {
        // the declared payload runs past EOF
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SKIPPABLE_MAGIC);
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(&[0x11; 4]);
        assert_eq!(
            check_verdict("f.zst", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_magic() {
        assert_eq!(
            check_verdict("f.zst", &[0xAB; 32]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.zst", b"").await, Completeness::Incomplete);
    }
}

pub mod unknown {
    use super::*;

    #[tokio::test]
    async fn frame_cap_exceeded() {
        // 1025 frames exhaust the header-read cap at the frame loop
        let bytes = skippable(b"").repeat(1025);
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Unknown);
    }

    #[tokio::test]
    async fn block_cap_exceeded() {
        // 1100 blocks exhaust the header-read cap inside the block loop
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.extend_from_slice(&block_header(false, 0, 0).repeat(1100));
        assert_eq!(check_verdict("f.zst", &bytes).await, Completeness::Unknown);
    }
}

pub mod errors {
    use miru_agent::fileformats::{self, FileFormatsErr};
    use miru_agent::filesys::{dirs, File, FileSysErr, PathExt};

    #[tokio::test]
    async fn read_error_on_directory() {
        // opening a directory read-only succeeds on linux, but reading it
        // fails (EISDIR), exercising the frame-header read-error path
        let dir = dirs::create_temp("testing").await.unwrap();
        let sub = dir.subdir("d.zst");
        dirs::create(&sub).await.unwrap();
        let file = File::new(sub.path());
        assert!(matches!(
            fileformats::check(&file).await.unwrap_err(),
            FileFormatsErr::FileSysErr(FileSysErr::ReadFileErr { .. })
        ));
    }
}
