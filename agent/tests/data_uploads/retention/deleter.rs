// internal crates
use miru_agent::data_uploads::retention::{DeleteErr, Deleter, DeleterArgs, DeleterExt, Job};
use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};

// external crates
use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;

/// A real on-disk temp file holding `contents`; the returned guard deletes it
/// on drop.
async fn temp_file(contents: &[u8]) -> files::TempFile {
    let tmp = files::temp("delete-actor-test").unwrap();
    files::write_bytes(tmp.file(), contents, WriteOptions::OVERWRITE_NONATOMIC)
        .await
        .unwrap();
    tmp
}

/// A zero-delay `Job` for `file` that is due immediately: its
/// size/mtime/digest reflect the file's current on-disk state.
async fn make_job(file: &File) -> Job {
    let now = Utc::now();
    Job {
        file: file.clone(),
        size: files::size(file).await.unwrap(),
        digest: files::hash(file).await.unwrap(),
        mtime: DateTime::<Utc>::from(files::last_modified(file).await.unwrap()),
        first_observed_at: now,
        last_observed_at: now,
        ttl_secs: 0,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}

/// Two symlinks pointing at each other. `stat` and `open` on either fail with
/// ELOOP, which classifies as a counted retry rather than a terminal failure.
/// Built with `std::os::unix::fs::symlink` rather than `files::create_symlink`
/// because the latter asserts the target exists.
fn symlink_loop(dir: &Dir) -> File {
    let a = dir.file("loop-a");
    let b = dir.file("loop-b");
    std::os::unix::fs::symlink(b.path(), a.path()).unwrap();
    std::os::unix::fs::symlink(a.path(), b.path()).unwrap();
    a
}

/// A `Job` for a path whose stat cannot succeed. The recorded size/digest/mtime
/// are never compared: the sweep fails at the stat step before any identity
/// check runs, so `make_job` — which stats the file in setup — is unusable.
fn wedged_job(file: File) -> Job {
    let now = Utc::now();
    Job {
        file,
        size: 0,
        digest: "sha256:unused".to_string(),
        mtime: now,
        first_observed_at: now,
        last_observed_at: now,
        ttl_secs: 0,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}

/// Spawn a deleter actor with default args (wall clock, no persistence).
fn spawn_deleter() -> (Deleter, JoinHandle<()>) {
    Deleter::spawn(16, DeleterArgs::default()).unwrap()
}

#[tokio::test]
async fn actor_round_trip() {
    let src = temp_file(b"hello world").await;
    let (deleter, handle) = spawn_deleter();

    deleter.enqueue(make_job(src.file()).await).await.unwrap();
    assert_eq!(deleter.len().await.unwrap(), 1);

    // the zero-delay record is due immediately: one sweep deletes the file
    // and drops the entry.
    deleter.sweep().await.unwrap();
    assert_eq!(deleter.len().await.unwrap(), 0);
    assert!(!src.file().exists());

    deleter.shutdown().await.unwrap();
    handle.await.unwrap();
}

// A domain error crosses the actor boundary intact: the worker's dispatch
// plumbing must not flatten an inner Err into a successful send.
#[tokio::test]
async fn enqueue_on_full_queue_errors_through_the_handle() {
    let src_a = temp_file(b"hello world").await;
    let src_b = temp_file(b"other bytes").await;
    let (deleter, handle) = Deleter::spawn(
        16,
        DeleterArgs {
            queue_capacity: 1,
            ..DeleterArgs::default()
        },
    )
    .unwrap();

    deleter.enqueue(make_job(src_a.file()).await).await.unwrap();
    let err = deleter
        .enqueue(make_job(src_b.file()).await)
        .await
        .unwrap_err();

    let DeleteErr::QueueFullErr(err) = err else {
        panic!("expected QueueFullErr, got: {err:?}");
    };
    assert_eq!(err.capacity, 1);
    assert_eq!(err.file, src_b.file().to_string());
    assert_eq!(deleter.len().await.unwrap(), 1);

    deleter.shutdown().await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn enqueue_after_shutdown_errors() {
    let src = temp_file(b"hello world").await;
    let (deleter, handle) = spawn_deleter();

    deleter.shutdown().await.unwrap();
    handle.await.unwrap();

    let err = deleter
        .enqueue(make_job(src.file()).await)
        .await
        .unwrap_err();
    assert!(matches!(err, DeleteErr::SendActorMessageErr(_)));
}

// The attempt budget is enforced through the actor, not just in the
// single-threaded deleter: a job whose sweep keeps failing is given up on and
// leaves the queue. `sweep()` awaits the worker's response, so each sweep has
// completed before the following `len()` is asked for.
#[tokio::test]
async fn wedged_job_is_given_up_on_through_the_actor() {
    let dir = dirs::temp("delete-actor-wedged").unwrap();
    let (deleter, handle) = Deleter::spawn(
        16,
        DeleterArgs {
            attempts: 2,
            ..DeleterArgs::default()
        },
    )
    .unwrap();

    deleter
        .enqueue(wedged_job(symlink_loop(&dir)))
        .await
        .unwrap();
    assert_eq!(deleter.len().await.unwrap(), 1);

    deleter.sweep().await.unwrap();
    deleter.sweep().await.unwrap();
    assert_eq!(deleter.len().await.unwrap(), 0);

    deleter.shutdown().await.unwrap();
    handle.await.unwrap();
}
