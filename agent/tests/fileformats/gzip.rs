// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// A plausible gzip file (18 bytes): magic + deflate method byte + flags,
/// zero-padded to the 10-byte header + 8-byte trailer minimum.
fn valid_gzip() -> Vec<u8> {
    let mut bytes = vec![0x1F, 0x8B, 0x08, 0x00];
    bytes.resize(18, 0x00);
    bytes
}

pub mod unknown {
    use super::*;

    #[tokio::test]
    async fn valid_magic_and_min_size() {
        // gzip is never Complete: the trailer cannot be validated without
        // decompressing, so a plausible file is Unknown
        assert_eq!(
            check_verdict("f.gz", &valid_gzip()).await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn larger_file() {
        let mut bytes = valid_gzip();
        bytes.extend_from_slice(&[0x11; 64]);
        assert_eq!(check_verdict("f.gz", &bytes).await, Completeness::Unknown);
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn bad_magic() {
        assert_eq!(
            check_verdict("f.gz", &[0xAB; 18]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size_with_valid_magic() {
        // the size gate precedes the magic check, so a valid magic in a
        // 17-byte file is still Incomplete
        assert_eq!(
            check_verdict("f.gz", &valid_gzip()[..17]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.gz", b"").await, Completeness::Incomplete);
    }
}
