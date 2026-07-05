//! Real-cloud S3 integration tests for [`miru_agent::s3::Store`].
//!
//! Unlike the replay-client unit tests in `tests/s3/`, these hit a **real S3
//! bucket** over the network using the production [`Store::new`] constructor.
//! They exercise the full object lifecycle (single-part put/get/delete,
//! multipart put/get, the multipart primitives, abort, and the not-found path)
//! against live AWS so we catch signing, part-sizing, and wire-format
//! regressions the mock cannot.
//!
//! ## Gating / skipping
//!
//! Every test is gated on real credentials and a target bucket. If any of the
//! required environment variables is absent, [`setup`] returns `None` and the
//! test prints a skip line and returns success. This keeps `cargo test` green
//! locally and in any CI job that does not provide AWS access.
//!
//! Required environment:
//! - `MIRU_S3_IT_BUCKET`     — the bucket these tests read/write (required).
//! - `AWS_ACCESS_KEY_ID`     — access key id (required).
//! - `AWS_SECRET_ACCESS_KEY` — secret access key (required).
//! - `AWS_SESSION_TOKEN`     — STS session token (required; the production
//!   `Store` is built only from temporary credentials).
//! - `MIRU_S3_IT_REGION`     — bucket region (optional, defaults to `us-east-1`).
//!
//! Each run writes under a unique key prefix
//! (`integration-tests/<unix-nanos>-<pid>/`) so parallel and repeated runs never
//! collide, and every test best-effort cleans up the objects and uploads it
//! created. A bucket lifecycle rule that expires the `integration-tests/` prefix
//! (and aborts incomplete multipart uploads) is the ultimate backstop.
//!
//! ## Running locally
//!
//! ```sh
//! export MIRU_S3_IT_BUCKET=my-test-bucket
//! export MIRU_S3_IT_REGION=us-east-1
//! export AWS_ACCESS_KEY_ID=...
//! export AWS_SECRET_ACCESS_KEY=...
//! export AWS_SESSION_TOKEN=...
//! cargo test --package miru-agent --features test --test s3_integration -- --test-threads=1
//! ```
//!
//! Without those variables the same command still passes: every test skips.

// standard crates
use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

// internal crates
use miru_agent::filesys::file::File;
use miru_agent::s3::{
    Config, Credentials, Object, PartToUpload, PutOptions, S3Err, Store, UploadedPart,
};

// external crates
use tempfile::NamedTempFile;

/// The bucket that owns every object a test creates.
struct Bucket(String);

/// The per-run key prefix that isolates one test run's objects from any other.
struct KeyPrefix(String);

/// Chunk size the module uses internally for multipart uploads (8 MiB). Mirrored
/// here so tests can size files to force a known part count.
const PART_SIZE: u64 = 8 * 1024 * 1024;

/// Reads the required credentials and bucket from the environment and builds a
/// real [`Store`]. Returns `None` (so the calling test skips) when the bucket or
/// any credential component is missing.
fn setup() -> Option<(Store, Bucket, KeyPrefix)> {
    let bucket = non_empty_env("MIRU_S3_IT_BUCKET")?;
    let region = non_empty_env("MIRU_S3_IT_REGION").unwrap_or_else(|| "us-east-1".to_string());
    let access_key_id = non_empty_env("AWS_ACCESS_KEY_ID")?;
    let secret_access_key = non_empty_env("AWS_SECRET_ACCESS_KEY")?;
    let session_token = non_empty_env("AWS_SESSION_TOKEN")?;

    let store = Store::new(Config {
        creds: Credentials {
            access_key_id,
            secret_access_key,
            session_token,
        },
        region,
    });

    // Unix nanos + pid keeps concurrent and repeated runs isolated.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the unix epoch")
        .as_nanos();
    let prefix = format!("integration-tests/{}-{}/", nanos, std::process::id());

    Some((store, Bucket(bucket), KeyPrefix(prefix)))
}

/// Returns the value of `var` only when it is set and non-empty.
fn non_empty_env(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Builds an [`Object`] under this run's prefix for the given leaf key.
fn object(bucket: &Bucket, prefix: &KeyPrefix, leaf: &str) -> Object {
    Object {
        bucket: bucket.0.clone(),
        key: format!("{}{}", prefix.0, leaf),
    }
}

/// Writes `bytes` to a fresh temp file and returns the handle (kept alive so the
/// file is not deleted until the test drops it).
fn temp_file_with(bytes: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create temp source file");
    f.write_all(bytes).expect("write temp source file");
    f.flush().expect("flush temp source file");
    f
}

/// Deterministic pseudo-random-ish payload of `len` bytes. A rolling byte
/// pattern (rather than all-zeros) makes a truncated or mis-ordered round-trip
/// detectable.
fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Best-effort teardown: delete every object and abort every upload the test
/// created. All errors are ignored — the bucket lifecycle rule is the backstop.
async fn cleanup(store: &Store, objects: &[Object], uploads: &[(Object, String)]) {
    for (obj, upload_id) in uploads {
        let _ = store.abort_multipart_upload(obj, upload_id).await;
    }
    for obj in objects {
        let _ = store.delete(obj).await;
    }
}

/// 1. Single-part round trip: put a small file, confirm it exists, get it back,
///    assert the bytes match, delete it, and confirm it is gone.
#[tokio::test]
async fn single_part_round_trip() {
    let Some((store, bucket, prefix)) = setup() else {
        eprintln!("skipping single_part_round_trip: MIRU_S3_IT_* / AWS creds not set");
        return;
    };

    let obj = object(&bucket, &prefix, "single/hello.bin");
    let body = payload(4096);
    let src = temp_file_with(&body);

    store
        .put_singlepart(&File::new(src.path()), &obj)
        .await
        .expect("put_singlepart");

    assert!(store.exists(&obj).await.expect("exists after put"));

    let dest = NamedTempFile::new().expect("create temp dest file");
    store
        .get(&obj, &File::new(dest.path()))
        .await
        .expect("get object");
    let got = std::fs::read(dest.path()).expect("read dest file");
    assert_eq!(got, body, "round-tripped bytes must match");

    store.delete(&obj).await.expect("delete object");
    assert!(!store.exists(&obj).await.expect("exists after delete"));

    cleanup(&store, &[obj], &[]).await;
}

/// 2. Multipart round trip: put a ~10 MiB file with an 8 MiB part size (size >
///    part_size forces the multipart path; the module chunks at 8 MiB, so this
///    is 2 parts), get it back, assert the bytes match, then delete.
#[tokio::test]
async fn multipart_round_trip() {
    let Some((store, bucket, prefix)) = setup() else {
        eprintln!("skipping multipart_round_trip: MIRU_S3_IT_* / AWS creds not set");
        return;
    };

    let obj = object(&bucket, &prefix, "multipart/blob.bin");
    // 10 MiB > 8 MiB part size => multipart path, 2 parts (8 MiB + 2 MiB).
    let body = payload(10 * 1024 * 1024);
    let src = temp_file_with(&body);

    store
        .put(
            &File::new(src.path()),
            &obj,
            PutOptions {
                part_size: PART_SIZE,
            },
        )
        .await
        .expect("multipart put");

    let dest = NamedTempFile::new().expect("create temp dest file");
    store
        .get(&obj, &File::new(dest.path()))
        .await
        .expect("get object");
    let got = std::fs::read(dest.path()).expect("read dest file");
    assert_eq!(got.len(), body.len(), "round-tripped length must match");
    assert_eq!(got, body, "round-tripped bytes must match");

    store.delete(&obj).await.expect("delete object");

    cleanup(&store, &[obj], &[]).await;
}

/// 3. Multipart primitives: drive create -> upload_part (two 8 MiB chunks of a
///    13 MiB file) -> list_parts -> complete -> get -> verify -> delete, using
///    the low-level API directly.
#[tokio::test]
async fn multipart_primitives() {
    let Some((store, bucket, prefix)) = setup() else {
        eprintln!("skipping multipart_primitives: MIRU_S3_IT_* / AWS creds not set");
        return;
    };

    let obj = object(&bucket, &prefix, "primitives/blob.bin");
    // 13 MiB => two parts of 8 MiB and 5 MiB (both >= the 5 MiB S3 minimum).
    let total: u64 = 13 * 1024 * 1024;
    let body = payload(total as usize);
    let src = temp_file_with(&body);
    let file = File::new(src.path());

    let upload_id = store
        .create_multipart_upload(&obj)
        .await
        .expect("create_multipart_upload");

    // Build parts by hand: [0, 8 MiB) and [8 MiB, 13 MiB).
    let mut uploaded: Vec<UploadedPart> = Vec::new();
    let mut offset: u64 = 0;
    let mut number: i32 = 1;
    while offset < total {
        let length = PART_SIZE.min(total - offset);
        let part = PartToUpload {
            upload_id: upload_id.clone(),
            number,
            offset,
            length,
        };
        let etag = store
            .upload_part(&file, &obj, &part)
            .await
            .expect("upload_part");
        uploaded.push(UploadedPart { number, etag });
        offset += length;
        number += 1;
    }
    assert_eq!(uploaded.len(), 2, "13 MiB at 8 MiB parts must be 2 parts");

    let listed = store
        .list_parts(&obj, &upload_id)
        .await
        .expect("list_parts")
        .expect("list_parts returns Some for a live upload");
    let mut listed_numbers: Vec<i32> = listed.iter().map(|p| p.number).collect();
    listed_numbers.sort_unstable();
    assert_eq!(listed_numbers, vec![1, 2], "both parts must be listed");

    store
        .complete_multipart_upload(&obj, &upload_id, &uploaded)
        .await
        .expect("complete_multipart_upload");

    let dest = NamedTempFile::new().expect("create temp dest file");
    store
        .get(&obj, &File::new(dest.path()))
        .await
        .expect("get object");
    let got = std::fs::read(dest.path()).expect("read dest file");
    assert_eq!(got, body, "assembled object bytes must match");

    store.delete(&obj).await.expect("delete object");

    cleanup(&store, &[obj], &[]).await;
}

/// 4. Abort discards the upload: create an upload, land one part, abort it, then
///    confirm `list_parts` reports the upload is gone (`Ok(None)`) and no object
///    exists.
#[tokio::test]
async fn abort_discards_upload() {
    let Some((store, bucket, prefix)) = setup() else {
        eprintln!("skipping abort_discards_upload: MIRU_S3_IT_* / AWS creds not set");
        return;
    };

    let obj = object(&bucket, &prefix, "abort/blob.bin");
    let body = payload(PART_SIZE as usize); // exactly one 8 MiB part.
    let src = temp_file_with(&body);
    let file = File::new(src.path());

    let upload_id = store
        .create_multipart_upload(&obj)
        .await
        .expect("create_multipart_upload");

    let part = PartToUpload {
        upload_id: upload_id.clone(),
        number: 1,
        offset: 0,
        length: PART_SIZE,
    };
    store
        .upload_part(&file, &obj, &part)
        .await
        .expect("upload_part");

    store
        .abort_multipart_upload(&obj, &upload_id)
        .await
        .expect("abort_multipart_upload");

    let after = store
        .list_parts(&obj, &upload_id)
        .await
        .expect("list_parts after abort");
    assert!(
        after.is_none(),
        "aborted upload must be gone (Ok(None)), got {after:?}"
    );
    assert!(
        !store.exists(&obj).await.expect("exists after abort"),
        "aborting must not materialize an object"
    );

    // Nothing to clean: the object was never completed and the upload is gone.
    cleanup(&store, &[], &[]).await;
}

/// 5. Getting a nonexistent key maps to `ObjectNotFoundErr`, and `exists`
///    returns `false` for it.
#[tokio::test]
async fn get_missing_key_is_not_found() {
    let Some((store, bucket, prefix)) = setup() else {
        eprintln!("skipping get_missing_key_is_not_found: MIRU_S3_IT_* / AWS creds not set");
        return;
    };

    let obj = object(&bucket, &prefix, "missing/nope.bin");
    let dest = NamedTempFile::new().expect("create temp dest file");

    let err = store
        .get(&obj, &File::new(dest.path()))
        .await
        .expect_err("get of a missing key must error");
    assert!(
        matches!(err, S3Err::ObjectNotFoundErr(_)),
        "expected ObjectNotFoundErr, got {err:?}"
    );

    assert!(
        !store.exists(&obj).await.expect("exists on missing key"),
        "a missing key must not exist"
    );

    cleanup(&store, &[], &[]).await;
}
