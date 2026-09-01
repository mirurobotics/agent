// internal crates
use crate::fileformats::{
    check_verdict, gzip::valid_gzip, hdf5::superblock, jpeg::valid_jpeg, matroska::mkv,
    mp4::valid_mp4, sqlite::sqlite_db,
};
use miru_agent::fileformats::{self, Completeness, FileFormatsErr};
use miru_agent::filesys::{dirs, FileSysErr};

/// PNG signature.
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
/// The complete IEND chunk: zero length + `IEND` + constant CRC.
const PNG_IEND: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
];

/// A minimal finalized PNG: signature + IEND chunk (20 bytes).
fn minimal_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&PNG_SIGNATURE);
    bytes.extend_from_slice(&PNG_IEND);
    bytes
}

pub mod routing {
    use super::*;

    #[tokio::test]
    async fn unknown_extension_is_unknown() {
        assert_eq!(
            check_verdict("f.xyz", b"some bytes").await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn no_extension_is_unknown() {
        assert_eq!(
            check_verdict("noext", b"some bytes").await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn uppercase_extension_routes_case_insensitively() {
        assert_eq!(
            check_verdict("f.PNG", &minimal_png()).await,
            Completeness::Complete
        );
    }

    #[tokio::test]
    async fn alias_extensions_route_to_their_formats() {
        // valid fixture bytes pin each alias to its checker: only the right
        // format calls them Complete (empty files cannot distinguish, since
        // every checker calls them Incomplete)
        let cases: [(&str, Vec<u8>); 8] = [
            ("db", sqlite_db(512, 1, 512)),
            ("sqlite", sqlite_db(512, 1, 512)),
            ("sqlite3", sqlite_db(512, 1, 512)),
            ("mov", valid_mp4()),
            ("m4v", valid_mp4()),
            ("jpeg", valid_jpeg()),
            ("hdf5", superblock(0, 8, 0, 64, 64)),
            ("webm", mkv(&[0x88], &[0u8; 8])),
        ];
        for (ext, bytes) in cases {
            let verdict = check_verdict(&format!("f.{ext}"), &bytes).await;
            assert_eq!(verdict, Completeness::Complete, "extension: {ext}");
        }
    }

    #[tokio::test]
    async fn tgz_routes_to_gzip() {
        // gzip is never Complete: a plausible gzip file is Unknown, which
        // pins tgz to gzip (tar would call these 18 bytes Incomplete)
        assert_eq!(
            check_verdict("f.tgz", &valid_gzip()).await,
            Completeness::Unknown
        );
    }

    #[tokio::test]
    async fn all_recognized_extensions_route_and_empty_is_incomplete() {
        let extensions = [
            "mcap", "parquet", "bag", "db", "db3", "sqlite", "sqlite3", "mp4", "mov", "m4v", "jpg",
            "jpeg", "png", "zip", "gz", "tgz", "tar", "zst", "h5", "hdf5", "mkv", "webm", "avi",
        ];
        for ext in extensions {
            let verdict = check_verdict(&format!("f.{ext}"), b"").await;
            assert_eq!(verdict, Completeness::Incomplete, "extension: {ext}");
        }
    }
}

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn missing_file_recognized_ext_is_err() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let file = dir.file("missing.png");
        assert!(matches!(
            fileformats::check(&file).await.unwrap_err(),
            FileFormatsErr::FileSysErr(FileSysErr::PathDoesNotExistErr { .. })
        ));
    }

    #[tokio::test]
    async fn missing_file_unknown_ext_is_err() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let file = dir.file("missing.xyz");
        assert!(matches!(
            fileformats::check(&file).await.unwrap_err(),
            FileFormatsErr::FileSysErr(FileSysErr::PathDoesNotExistErr { .. })
        ));
    }
}
