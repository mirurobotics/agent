// internal crates
use miru_agent::delete::{DeleteErr, Deleter, DeleterArgs, DeleterExt, PendingDelete};
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

/// A zero-delay `PendingDelete` for `file` that is due immediately: its
/// size/mtime/digest reflect the file's current on-disk state.
async fn make_record(file: &File) -> PendingDelete {
    PendingDelete {
        file: file.clone(),
        size: files::size(file).await.unwrap(),
        mtime: DateTime::<Utc>::from(files::last_modified(file).await.unwrap()),
        digest: files::hash(file).await.unwrap(),
        eligible_at: Utc::now(),
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

    deleter
        .enqueue(make_record(src.file()).await)
        .await
        .unwrap();
    assert_eq!(deleter.len().await.unwrap(), 1);

    // the zero-delay record is due immediately: one sweep deletes the file
    // and drops the entry.
    deleter.sweep().await.unwrap();
    assert_eq!(deleter.len().await.unwrap(), 0);
    assert!(!src.file().exists());

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
        .enqueue(make_record(src.file()).await)
        .await
        .unwrap_err();
    assert!(matches!(err, DeleteErr::SendActorMessageErr(_)));
}
