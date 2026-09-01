// internal crates
use crate::fileformats::check_verdict;
use miru_agent::fileformats::Completeness;

/// An AVI file of `file_len` zero-padded bytes: `RIFF` + `riff_size: u32 LE`
/// + `AVI ` form type.
fn avi(riff_size: u32, file_len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; file_len];
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    bytes[8..12].copy_from_slice(b"AVI ");
    bytes
}

pub mod complete {
    use super::*;

    #[tokio::test]
    async fn valid() {
        // riff_size + 8 == file size
        assert_eq!(
            check_verdict("f.avi", &avi(16, 24)).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn minimal() {
        assert_eq!(
            check_verdict("f.avi", &avi(4, 12)).await,
            Completeness::Complete
        );
    }
}

pub mod incomplete {
    use super::*;

    #[tokio::test]
    async fn zero_size_field() {
        // a zero riff_size is the still-recording state
        assert_eq!(
            check_verdict("f.avi", &avi(0, 24)).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn stale_size_field() {
        assert_eq!(
            check_verdict("f.avi", &avi(16, 20)).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn wrong_form_type() {
        let mut bytes = avi(16, 24);
        bytes[8..12].copy_from_slice(b"WAVE");
        assert_eq!(
            check_verdict("f.avi", &bytes).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn bad_riff_magic() {
        assert_eq!(
            check_verdict("f.avi", &[0xAB; 24]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn below_min_size() {
        assert_eq!(
            check_verdict("f.avi", &avi(4, 12)[..11]).await,
            Completeness::Incomplete
        );
    }

    #[tokio::test]
    async fn empty() {
        assert_eq!(check_verdict("f.avi", b"").await, Completeness::Incomplete);
    }
}
