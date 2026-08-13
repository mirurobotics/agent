// internal crates
use miru_agent::data_uploads::retention::{DeleteErr, Deleter, DeleterArgs, DeleterExt, Job};
use miru_agent::filesys::{files, File, PathExt, WriteOptions};

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
