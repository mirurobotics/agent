// Real-cloud S3 integration tests. These hit an actual S3 bucket, so they only
// run when AWS creds and a target bucket are supplied via env; otherwise each
// test prints a skip notice and passes as a no-op, keeping `cargo test` green in
// dev/CI with no credentials. This binary uses only `Store::new` (always
// available), so it compiles in a normal build without the `test` feature.

// standard crates
use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

// internal crates
use miru_agent::filesys::file::File;
use miru_agent::filesys::path::PathExt;
use miru_agent::filesys::{files, WriteOptions};
use miru_agent::s3::{Config, Credentials, Object, S3Err, Source, Store};

// external crates
use tempfile::NamedTempFile;

/// A temp file kept alive for a test: the guard drops (and deletes) with `self`,
/// and `file` is our handle into it. Not using `filesys::files::temp` because
/// that helper is gated behind the `test` feature, which this binary must build
/// without.
struct TempFile {
    _guard: NamedTempFile,
    file: File,
}

impl TempFile {
    fn file(&self) -> &File {
        &self.file
    }
}

/// A fresh temp file — empty, ready to receive a `get` or be written to.
fn temp_file() -> TempFile {
    let guard = NamedTempFile::new().expect("create temp file");
    let file = File::new(guard.path().to_path_buf());
    TempFile {
        _guard: guard,
        file,
    }
}

/// A temp file pre-filled with `bytes`.
async fn temp_file_with(bytes: &[u8]) -> TempFile {
    let tf = temp_file();
    files::write_bytes(tf.file(), bytes, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .expect("write temp file");
    tf
}

/// A `Store` wired to a real bucket, plus the bucket name and a per-run key
/// prefix, or `None` when creds/bucket are absent (tests then no-op).
struct Fixture {
    store: Store,
    bucket: String,
    key_prefix: String,
}

/// Resolves real AWS creds + target bucket from the environment. Returns `None`
/// (after an `eprintln!` naming the missing var) whenever anything required is
/// absent, so callers skip cleanly. The key prefix is unique per run
/// (`integration-tests/<unix_nanos>-<pid>/`) so concurrent runs never collide.
fn setup() -> Option<Fixture> {
    let bucket = std::env::var("MIRU_S3_IT_BUCKET").unwrap_or_default();
    if bucket.is_empty() {
        eprintln!("skipping s3 integration test: MIRU_S3_IT_BUCKET is unset/empty");
        return None;
    }
    let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
    if access_key_id.is_empty() {
        eprintln!("skipping s3 integration test: AWS_ACCESS_KEY_ID is unset/empty");
        return None;
    }
    let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
    if secret_access_key.is_empty() {
        eprintln!("skipping s3 integration test: AWS_SECRET_ACCESS_KEY is unset/empty");
        return None;
    }
    let session_token = std::env::var("AWS_SESSION_TOKEN").unwrap_or_default();
    let region = std::env::var("MIRU_S3_IT_REGION").unwrap_or_else(|_| "us-east-1".to_string());

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the unix epoch")
        .as_nanos();
    let key_prefix = format!("integration-tests/{nanos}-{}/", std::process::id());

    let store = Store::new(Config {
        region,
        creds: Credentials {
            access_key_id,
            secret_access_key,
            session_token,
        },
    });
    Some(Fixture {
        store,
        bucket,
        key_prefix,
    })
}

impl Fixture {
    /// An `Object` at `<key_prefix><name>`, so each test namespaces its own keys.
    fn object(&self, name: &str) -> Object {
        Object {
            bucket: self.bucket.clone(),
            key: format!("{}{name}", self.key_prefix),
        }
    }
}

/// Fills `buf` with a cheap deterministic byte pattern so writes and reads can be
/// compared without carrying fixture data around.
fn fill_deterministic(buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (i % 251) as u8; // 251 is prime => long non-repeating stride.
    }
}

/// Writes `size` deterministic bytes straight to `dst` off disk, never holding
/// the whole payload in one `Vec` beyond a small streaming buffer.
async fn write_deterministic_file(dst: &File, size: u64) {
    let mut f = std::fs::File::create(dst.path()).expect("create fixture file");
    let mut chunk = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    while written < size {
        let n = std::cmp::min(chunk.len() as u64, size - written) as usize;
        for (i, b) in chunk[..n].iter_mut().enumerate() {
            *b = ((written as usize + i) % 251) as u8;
        }
        f.write_all(&chunk[..n]).expect("write fixture chunk");
        written += n as u64;
    }
    f.flush().expect("flush fixture file");
}

/// FNV-1a hash over a file's bytes, streamed so we never hold two large payloads
/// in memory at once. Returns `(len, hash)` for cheap content comparison.
async fn file_len_and_hash(file: &File) -> (u64, u64) {
    let mut f = tokio::fs::File::open(file.path())
        .await
        .expect("open file for hashing");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut buf = vec![0u8; 64 * 1024];
    let mut len: u64 = 0;
    loop {
        let n = tokio::io::AsyncReadExt::read(&mut f, &mut buf)
            .await
            .expect("read file for hashing");
        if n == 0 {
            break;
        }
        len += n as u64;
        for &b in &buf[..n] {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    (len, hash)
}

/// Round-trips a small object through the size-routing `put` (single PutObject):
/// put, confirm it exists, get it back byte-for-byte, delete, confirm it's gone.
#[tokio::test]
async fn single_part_round_trip() {
    let Some(fx) = setup() else {
        return;
    };
    let obj = fx.object("single_part_round_trip.bin");
    let payload = b"miru-s3-integration-single-part-payload\x00\x01\x02\xff".repeat(64);
    let src = temp_file_with(&payload).await;

    fx.store
        .put(src.file().clone(), &obj)
        .await
        .expect("put small object");
    assert!(fx.store.exists(&obj).await.expect("exists after put"));

    let dest = temp_file();
    fx.store.get(&obj, dest.file()).await.expect("get object");
    assert_eq!(
        files::read_bytes(dest.file()).await.expect("read dest"),
        payload,
        "round-tripped bytes must match exactly"
    );

    fx.store.delete(&obj).await.expect("delete object");
    assert!(!fx.store.exists(&obj).await.expect("exists after delete"));
}

/// Round-trips a ~10 MiB object through `put_multipart` (> 8 MiB PART_SIZE, so
/// multiple parts with a sub-5-MiB tail). Compares length + a streamed content
/// hash rather than holding two 10 MiB `Vec`s.
#[tokio::test]
async fn multipart_round_trip() {
    let Some(fx) = setup() else {
        return;
    };
    let obj = fx.object("multipart_round_trip.bin");
    let size: u64 = 10 * 1024 * 1024; // > 8 MiB PART_SIZE => multipart.
    let src = temp_file_with(b"").await;
    write_deterministic_file(src.file(), size).await;
    let src_meta = file_len_and_hash(src.file()).await;

    let source = Source {
        file: src.file().clone(),
        size: files::size(src.file()).await.expect("stat source"),
    };
    assert_eq!(source.size, size);

    fx.store
        .put_multipart(&source, &obj)
        .await
        .expect("put_multipart");

    let dest = temp_file();
    fx.store.get(&obj, dest.file()).await.expect("get object");
    assert_eq!(
        file_len_and_hash(dest.file()).await,
        src_meta,
        "multipart round-trip length + content hash must match"
    );

    fx.store.delete(&obj).await.expect("delete object");
}

/// `put` (size-routed) on a > 8 MiB file must itself pick the multipart path and
/// round-trip correctly, covering the auto-selection branch end to end.
#[tokio::test]
async fn put_auto_selects_multipart() {
    let Some(fx) = setup() else {
        return;
    };
    let obj = fx.object("put_auto_selects_multipart.bin");
    let size: u64 = 9 * 1024 * 1024; // > 8 MiB PART_SIZE => `put` routes to multipart.
    let src = temp_file_with(b"").await;
    write_deterministic_file(src.file(), size).await;
    let src_meta = file_len_and_hash(src.file()).await;

    fx.store
        .put(src.file().clone(), &obj)
        .await
        .expect("put large object");

    let dest = temp_file();
    fx.store.get(&obj, dest.file()).await.expect("get object");
    assert_eq!(
        file_len_and_hash(dest.file()).await,
        src_meta,
        "auto-multipart round-trip length + content hash must match"
    );

    fx.store.delete(&obj).await.expect("delete object");
}

/// `get` on a key that was never written maps to `ObjectNotFoundErr`.
#[tokio::test]
async fn get_missing_key_is_not_found() {
    let Some(fx) = setup() else {
        return;
    };
    let obj = fx.object("get_missing_key_is_not_found.bin");
    let dest = temp_file();

    let err = fx.store.get(&obj, dest.file()).await.unwrap_err();
    assert!(matches!(err, S3Err::ObjectNotFoundErr(_)), "got {err:?}");
}

/// `exists` on a missing key is `Ok(false)`, not an error.
#[tokio::test]
async fn exists_missing_key_is_false() {
    let Some(fx) = setup() else {
        return;
    };
    let obj = fx.object("exists_missing_key_is_false.bin");
    assert!(!fx.store.exists(&obj).await.expect("exists on missing key"));
}

/// Resuming with a bogus upload id maps to `NoSuchUploadErr` — the closest
/// publicly reachable coverage of the not-found-upload path (the low-level abort
/// primitive is private). `fill_deterministic` gives the `Source` real bytes so
/// the failure comes from the unknown upload, not an empty file.
#[tokio::test]
async fn resume_unknown_upload_is_no_such_upload() {
    let Some(fx) = setup() else {
        return;
    };
    let obj = fx.object("resume_unknown_upload_is_no_such_upload.bin");
    let mut bytes = vec![0u8; 1024];
    fill_deterministic(&mut bytes);
    let src = temp_file_with(&bytes).await;
    let source = Source {
        file: src.file().clone(),
        size: files::size(src.file()).await.expect("stat source"),
    };

    let err = fx
        .store
        .resume_multipart_upload(&source, &obj, "this-upload-id-does-not-exist")
        .await
        .unwrap_err();
    assert!(matches!(err, S3Err::NoSuchUploadErr(_)), "got {err:?}");
}
