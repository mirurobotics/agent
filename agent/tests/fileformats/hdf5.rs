// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// HDF5 superblock signature.
const SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', 0x0D, 0x0A, 0x1A, 0x0A];

/// A zero-padded superblock of `file_len` bytes with the signature, version,
/// size-of-offsets byte, base address, and end-of-file address at the
/// version's offsets.
fn superblock(version: u8, size_of_offsets: u8, base: u64, eof: u64, file_len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; file_len];
    bytes[0..8].copy_from_slice(&SIGNATURE);
    bytes[8] = version;
    let (so_offset, base_offset) = match version {
        1 => (13usize, 28usize),
        2 | 3 => (9, 12),
        _ => (13, 24),
    };
    bytes[so_offset] = size_of_offsets;
    bytes[base_offset..base_offset + 8].copy_from_slice(&base.to_le_bytes());
    let eof_offset = base_offset + 16;
    bytes[eof_offset..eof_offset + 8].copy_from_slice(&eof.to_le_bytes());
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid_v0() {
        assert_eq!(
            check_verdict("f.h5", &superblock(0, 8, 0, 64, 64)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn valid_v1() {
        assert_eq!(
            check_verdict("f.h5", &superblock(1, 8, 0, 64, 64)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn valid_v2() {
        assert_eq!(
            check_verdict("f.h5", &superblock(2, 8, 0, 64, 64)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn valid_v3() {
        assert_eq!(
            check_verdict("f.h5", &superblock(3, 8, 0, 64, 64)).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn eof_address_mismatch() {
        assert_eq!(
            check_verdict("f.h5", &superblock(0, 8, 0, 65, 64)).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn so_field_past_eof() {
        // 13 bytes: the v0 size-of-offsets byte at offset 13 is past EOF
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SIGNATURE);
        bytes.push(0x00);
        bytes.extend_from_slice(&[0x00; 4]);
        assert_eq!(
            check_verdict("f.h5", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn eof_field_truncated() {
        // the v0 EOF address lives at 40..48; a 47-byte file cuts it short
        assert_eq!(
            check_verdict("f.h5", &superblock(0, 8, 0, 64, 64)[..47]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_signature() {
        assert_eq!(
            check_verdict("f.h5", &[0xAB; 64]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn tiny_file() {
        // the signature alone has no version byte
        assert_eq!(
            check_verdict("f.h5", &SIGNATURE).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.h5", b"").await, Completeness::Incomplete);
    }
}

pub mod unknown {
    use super::*;

    #[tokio::test]
    async fn unsupported_version() {
        assert_eq!(
            check_verdict("f.h5", &superblock(4, 8, 0, 64, 64)).await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn size_of_offsets_not_8() {
        assert_eq!(
            check_verdict("f.h5", &superblock(0, 4, 0, 64, 64)).await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn nonzero_base_address() {
        // a userblock shifts every address; not cheaply verifiable
        assert_eq!(
            check_verdict("f.h5", &superblock(0, 8, 512, 64, 64)).await,
            Completeness::Unknown
        );
    }
}
