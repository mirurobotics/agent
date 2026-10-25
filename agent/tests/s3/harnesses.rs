//! The upload tests every real bucket runs. A provider implements `RealBucket`
//! and calls `real_bucket_tests!`.
//!
//! Without credentials the tests fail rather than skip, so CI can't silently
//! stop checking a provider. The tests don't delete what they write; each
//! bucket's lifecycle rule does.

// standard crates
use std::collections::HashMap;

// internal crates
use crate::test_utils::filesys::files as test_files;
use miru_agent::filesys::{files, WriteOptions};
use miru_agent::s3::{Object, Options, S3Err, Source, Store};

// external crates
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use uuid::Uuid;

/// The store's multipart part size.
const PART: usize = 8 * 1024 * 1024;
/// Uploads with one `PutObject`.
pub const SMALL: usize = 64 * 1024;
/// Uploads in two parts.
const LARGE: usize = PART + 1024;

/// A real bucket, and credentials that can read and write objects in it.
pub trait RealBucket {
    /// The bucket and credentials from the environment.
    fn load() -> Self;

    fn name(&self) -> &str;

    /// The production store options, with credentials that can only upload
    /// `obj`, as the backend vends for each upload.
    async fn upload_options(&self, obj: &Object) -> Options;

    /// The metadata and bytes stored at `obj`, read back outside the store.
    async fn read(&self, obj: &Object) -> (HashMap<String, String>, Vec<u8>);

    /// An object at a fresh key, so no two runs share one.
    fn object(&self, name: &str) -> Object {
        Object {
            bucket: self.name().to_string(),
            key: format!("agent-ci/{}/{name}", Uuid::new_v4()),
        }
    }

    /// A store that uploads `obj` the way the agent does.
    async fn upload_store(&self, obj: &Object) -> Store {
        Store::new(self.upload_options(obj).await)
    }

    /// A client with the same config, for the calls `Store` doesn't expose.
    async fn upload_client(&self, obj: &Object) -> Client {
        Client::from_conf(self.upload_options(obj).await.config)
    }

    /// Checks `obj` holds `expected` and the test metadata.
    async fn assert_stored(&self, obj: &Object, expected: &[u8]) {
        let (stored_metadata, stored) = self.read(obj).await;
        assert_eq!(stored_metadata, metadata(), "{obj}: metadata");
        assert!(
            stored == expected,
            "{obj}: stored {} bytes that don't match the {} uploaded",
            stored.len(),
            expected.len()
        );
    }
}

/// The variable's value. Panics if it is unset or empty, as the credentials
/// are when CI can't authenticate.
pub fn env_var(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("{name} must be set: these tests run against a real bucket"),
    }
}

/// The metadata every test object carries.
fn metadata() -> HashMap<String, String> {
    HashMap::from([("miru-ci".to_string(), "upload-test".to_string())])
}

/// A repeating pattern whose period (251, a prime) doesn't divide the part
/// size, so a dropped or reordered part changes the bytes.
pub fn test_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// A temp file holding `bytes`, deleted when dropped.
async fn temp_file_with(bytes: &[u8]) -> test_files::TempFile {
    let tf = test_files::temp("upload-test").unwrap();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    tf
}

/// The CRC32C the agent uploads `file` with.
async fn crc32c_of(file: &test_files::TempFile) -> Option<u32> {
    let (_, crc32c) = files::sha256_and_crc32c(file.file()).await.unwrap();
    Some(crc32c)
}

/// Single-part and multipart uploads store the exact bytes and the metadata,
/// and pass the bucket's checks against the file's CRC32C.
pub async fn put_stores_exact_bytes_and_metadata<B: RealBucket>() {
    let bucket = B::load();
    for (name, len) in [("single.bin", SMALL), ("multi.bin", LARGE)] {
        let obj = bucket.object(name);
        let bytes = test_bytes(len);
        let file = temp_file_with(&bytes).await;

        let store = bucket.upload_store(&obj).await;
        store
            .put(file.to_file(), crc32c_of(&file).await, &obj, &metadata())
            .await
            .expect(name);
        bucket.assert_stored(&obj, &bytes).await;
    }
}

/// A file that no longer matches its CRC32C is refused, single-part and
/// multipart, so the bucket never stores the changed bytes.
pub async fn put_of_a_file_changed_since_its_crc32c_is_refused<B: RealBucket>() {
    let bucket = B::load();
    for (name, len) in [("stale-single.bin", SMALL), ("stale-multi.bin", LARGE)] {
        let obj = bucket.object(name);
        let file = temp_file_with(&test_bytes(len)).await;
        let crc32c = crc32c_of(&file).await;
        // Same size, other bytes.
        let changed = vec![0; len];
        files::write_bytes(file.file(), &changed, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();

        let store = bucket.upload_store(&obj).await;
        let err = store
            .put(file.to_file(), crc32c, &obj, &metadata())
            .await
            .expect_err(name);
        assert!(matches!(err, S3Err::ChecksumMismatchErr(_)), "{err:?}");
    }
}

/// Resuming an upload another client started, with new credentials as after
/// an agent restart, completes it with the exact bytes.
pub async fn resume_completes_a_started_upload<B: RealBucket>() {
    let bucket = B::load();
    let obj = bucket.object("resume.bin");
    let bytes = test_bytes(LARGE);
    let file = temp_file_with(&bytes).await;

    let client = bucket.upload_client(&obj).await;
    let upload = client
        .create_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .set_metadata(Some(metadata()))
        .send()
        .await
        .unwrap();
    let upload_id = upload.upload_id().unwrap();
    client
        .upload_part()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(upload_id)
        .part_number(1)
        .body(ByteStream::from(bytes[..PART].to_vec()))
        .send()
        .await
        .unwrap();

    let store = bucket.upload_store(&obj).await;
    let src = Source::open(file.to_file(), None, &obj).await.unwrap();
    store
        .resume_multipart_upload(src, &obj, upload_id)
        .await
        .unwrap();
    bucket.assert_stored(&obj, &bytes).await;
}

/// Resuming an upload the bucket no longer has fails with a no-such-upload
/// error.
pub async fn resuming_an_aborted_upload_fails_with_no_such_upload<B: RealBucket>() {
    let bucket = B::load();
    let obj = bucket.object("aborted.bin");
    let file = temp_file_with(&test_bytes(LARGE)).await;

    let client = bucket.upload_client(&obj).await;
    let upload = client
        .create_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .send()
        .await
        .unwrap();
    let upload_id = upload.upload_id().unwrap();
    client
        .abort_multipart_upload()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(upload_id)
        .send()
        .await
        .unwrap();

    let store = bucket.upload_store(&obj).await;
    let src = Source::open(file.to_file(), None, &obj).await.unwrap();
    let err = store
        .resume_multipart_upload(src, &obj, upload_id)
        .await
        .unwrap_err();
    assert!(matches!(err, S3Err::NoSuchUploadErr(_)), "{err:?}");
}

/// A put that finds its object already written, as when a retry follows a lost
/// success response, succeeds. On GCS the upload token can't overwrite, so
/// writes are create-only and the store counts GCS's 412 as done.
pub async fn put_of_an_existing_object_succeeds<B: RealBucket>() {
    let bucket = B::load();
    for (name, len) in [("exists-single.bin", SMALL), ("exists-multi.bin", LARGE)] {
        let obj = bucket.object(name);
        let bytes = test_bytes(len);
        let file = temp_file_with(&bytes).await;

        for _ in 0..2 {
            let store = bucket.upload_store(&obj).await;
            store
                .put(file.to_file(), crc32c_of(&file).await, &obj, &metadata())
                .await
                .expect(name);
        }
        bucket.assert_stored(&obj, &bytes).await;
    }
}

/// Generates the shared upload tests for a `RealBucket`.
///
/// Usage:
/// ```ignore
/// real_bucket_tests!(Gcs);
/// ```
macro_rules! real_bucket_tests {
    ($bucket:ty) => {
        mod harness {
            use super::*;
            use crate::s3::harnesses;

            #[tokio::test]
            async fn put_stores_exact_bytes_and_metadata() {
                harnesses::put_stores_exact_bytes_and_metadata::<$bucket>().await;
            }

            #[tokio::test]
            async fn put_of_a_file_changed_since_its_crc32c_is_refused() {
                harnesses::put_of_a_file_changed_since_its_crc32c_is_refused::<$bucket>().await;
            }

            #[tokio::test]
            async fn resume_completes_a_started_upload() {
                harnesses::resume_completes_a_started_upload::<$bucket>().await;
            }

            #[tokio::test]
            async fn resuming_an_aborted_upload_fails_with_no_such_upload() {
                harnesses::resuming_an_aborted_upload_fails_with_no_such_upload::<$bucket>().await;
            }

            #[tokio::test]
            async fn put_of_an_existing_object_succeeds() {
                harnesses::put_of_an_existing_object_succeeds::<$bucket>().await;
            }
        }
    };
}

pub(crate) use real_bucket_tests;
