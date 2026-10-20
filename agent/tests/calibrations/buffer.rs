// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    Buffer, BufferOptions, CalibrationJob, CalibrationsErr, QueueEntry, Report, ReportFile,
};
use miru_agent::disk::layout::CalibrationsLayout;
use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use uuid::Uuid;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(5000, 0).unwrap()
}

/// A job told apart from others by its component slug, `name`.
fn job(name: &str, calibrated_at_secs: i64) -> CalibrationJob {
    CalibrationJob {
        component_slug: name.to_string(),
        role: Some("aligner-1".to_string()),
        aspect: None,
        serial_number: Some("SN-1".to_string()),
        deployment_id: None,
        calibrated_at: DateTime::from_timestamp(calibrated_at_secs, 0).unwrap(),
    }
}

fn report(name: &str) -> Report {
    Report {
        job: job(name, 1000),
        content: json!({ "format": "json", "data": { "name": name, "offset": [1, 2, 3] } }),
    }
}

/// A report whose file is exactly `bytes` long once serialized.
fn report_of_size(name: &str, bytes: u64) -> Report {
    let mut report = report(name);
    report.content = json!("");
    let base = serialized_len(&report);
    report.content = json!("x".repeat((bytes - base) as usize));
    assert_eq!(serialized_len(&report), bytes);
    report
}

fn serialized_len(report: &Report) -> u64 {
    let record = ReportFile {
        job: report.job.clone(),
        content: report.content.clone(),
    };
    serde_json::to_vec(&record).unwrap().len() as u64
}

struct Fixture {
    _dir: test_dirs::TempDir,
    root: Dir,
}

impl Fixture {
    fn new() -> Self {
        let dir = test_dirs::temp("calibration_buffer_test").unwrap();
        let root = dir.to_dir().subdir("calibrations");
        Self { _dir: dir, root }
    }

    fn layout(&self) -> CalibrationsLayout {
        CalibrationsLayout::new(self.root.clone())
    }

    async fn open(&self) -> Buffer {
        self.open_with(BufferOptions::default()).await
    }

    async fn open_with(&self, options: BufferOptions) -> Buffer {
        Buffer::open(&self.layout(), options).await.unwrap()
    }

    fn report_file(&self, id: Uuid) -> File {
        self.layout().content().file(&format!("{id}.json"))
    }

    fn queue_file(&self) -> File {
        self.layout().queue()
    }
}

/// The buffered reports' names (component slugs), oldest first.
fn names(buffer: &Buffer) -> Vec<String> {
    buffer
        .jobs()
        .into_iter()
        .map(|job| job.component_slug)
        .collect()
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// Take and remove every ready report, oldest first.
async fn drain(buffer: &mut Buffer) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(entry) = buffer.next_ready(now()) {
        out.push(entry.job.component_slug.clone());
        buffer.remove(entry.id).await.unwrap();
    }
    out
}

async fn enqueue_all(buffer: &mut Buffer, names: &[&str]) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for name in names {
        ids.push(buffer.enqueue(report(name)).await.unwrap());
    }
    ids
}

/// The names `queue.json` holds, read straight from disk.
async fn queue_on_disk(fixture: &Fixture) -> Vec<String> {
    let value: serde_json::Value = files::read_json(&fixture.queue_file()).await.unwrap();
    value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["job"]["component_slug"].as_str().unwrap().to_string())
        .collect()
}

pub mod open {
    use super::*;

    #[tokio::test]
    async fn creates_an_empty_buffer() {
        let fixture = Fixture::new();
        let buffer = fixture.open().await;

        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.used_bytes(), 0);
        assert!(fixture.queue_file().exists());
        assert!(fixture.layout().content().exists());
        assert!(queue_on_disk(&fixture).await.is_empty());
    }

    #[tokio::test]
    async fn survives_a_restart() {
        let fixture = Fixture::new();
        let (ids, used) = {
            let mut buffer = fixture.open().await;
            let ids = enqueue_all(&mut buffer, &["a", "b", "c"]).await;
            (ids, buffer.used_bytes())
        };

        let mut buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["a", "b", "c"]));
        assert_eq!(buffer.used_bytes(), used);
        assert_eq!(buffer.next_ready(now()).unwrap().id, ids[0]);
        let file = buffer.read(ids[1]).await.unwrap();
        assert_eq!(file.job, report("b").job);
        assert_eq!(file.content, report("b").content);
        assert_eq!(drain(&mut buffer).await, strings(&["a", "b", "c"]));
    }

    #[tokio::test]
    async fn removes_report_files_with_no_queue_entry() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        // a crash between the report file and the queue entry leaves this
        let orphan = fixture.report_file(Uuid::new_v4());
        files::write_string(&orphan, "{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let buffer = fixture.open().await;

        assert!(!orphan.exists());
        assert!(fixture.report_file(ids[0]).exists());
        assert_eq!(names(&buffer), strings(&["a"]));
    }

    #[tokio::test]
    async fn leaves_files_that_are_not_report_files() {
        let fixture = Fixture::new();
        let content = fixture.layout().content();
        let id = Uuid::new_v4();
        let others = [
            content.file(&format!("{id}.json.corrupt")),
            content.file(&format!(".{id}.json.tmp")),
            content.file(&format!("{}.json", id.simple())),
            content.file(&format!("{}.json", id.to_string().to_uppercase())),
            content.file("notes.txt"),
            content.file("not-a-uuid.json"),
        ];
        for file in &others {
            files::write_string(file, "x", WriteOptions::OVERWRITE_ATOMIC)
                .await
                .unwrap();
        }

        let buffer = fixture.open().await;

        assert!(buffer.is_empty());
        for file in &others {
            assert!(file.exists(), "{file} was removed");
        }
    }

    #[tokio::test]
    async fn drops_queue_entries_whose_report_file_is_missing() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b", "c"]).await
        };
        files::delete(&fixture.report_file(ids[1])).await.unwrap();

        let buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["a", "c"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a", "c"]));
        assert_eq!(
            buffer.used_bytes(),
            serialized_len(&report("a")) + serialized_len(&report("c"))
        );
    }

    #[tokio::test]
    async fn rebuilds_an_unreadable_queue_from_the_report_files() {
        let fixture = Fixture::new();
        let mut ids = Vec::new();
        {
            let mut buffer = fixture.open().await;
            // enqueued out of calibration order; the rebuild orders by
            // calibrated_at since the lost queue order cannot be recovered
            for (name, secs) in [("late", 3000), ("early", 1000), ("middle", 2000)] {
                let report = Report {
                    job: job(name, secs),
                    ..report(name)
                };
                ids.push(buffer.enqueue(report).await.unwrap());
            }
        }
        files::write_string(
            &fixture.queue_file(),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
        // an unreadable report file is set aside, not uploaded or deleted
        let broken = Uuid::new_v4();
        files::write_string(
            &fixture.report_file(broken),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let mut buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["early", "middle", "late"]));
        assert_eq!(
            queue_on_disk(&fixture).await,
            strings(&["early", "middle", "late"])
        );
        // each rebuilt entry keeps the id that names its report file
        let entry = buffer.next_ready(now()).unwrap();
        assert_eq!(entry.id, ids[1]);
        let aside = fixture.root.file("queue.json.corrupt");
        assert_eq!(files::read_string(&aside).await.unwrap(), "{not json");
        let content = fixture.layout().content();
        assert!(content.file(&format!("{broken}.json.corrupt")).exists());
        assert!(!fixture.report_file(broken).exists());
        assert_eq!(
            drain(&mut buffer).await,
            strings(&["early", "middle", "late"])
        );
    }

    /// Lock the content directory so it cannot be listed, run `f`, then
    /// restore it so the temp dir can be cleaned up.
    #[cfg(unix)]
    async fn with_unlistable_content<T>(
        fixture: &Fixture,
        f: impl std::future::Future<Output = T>,
    ) -> T {
        use std::os::unix::fs::PermissionsExt;
        let content = fixture.layout().content();
        let mode = |bits| std::fs::Permissions::from_mode(bits);
        dirs::set_permissions(&content, mode(0o300)).await.unwrap();
        let out = f.await;
        dirs::set_permissions(&content, mode(0o700)).await.unwrap();
        out
    }

    /// A transient listing failure must not read as an empty directory: that
    /// would drop every queue entry, and the next open would delete every
    /// report file as an orphan.
    #[cfg(unix)]
    #[tokio::test]
    async fn listing_failure_fails_the_open_and_loses_nothing() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b"]).await
        };

        let result = with_unlistable_content(
            &fixture,
            Buffer::open(&fixture.layout(), BufferOptions::default()),
        )
        .await;

        assert!(matches!(result, Err(CalibrationsErr::FileSysErr(_))));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a", "b"]));
        let buffer = fixture.open().await;
        assert_eq!(names(&buffer), strings(&["a", "b"]));
        assert!(fixture.report_file(ids[0]).exists());
        assert!(fixture.report_file(ids[1]).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn listing_failure_during_a_rebuild_keeps_the_snapshot_for_the_next_open() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b"]).await;
        }
        files::write_string(
            &fixture.queue_file(),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let result = with_unlistable_content(
            &fixture,
            Buffer::open(&fixture.layout(), BufferOptions::default()),
        )
        .await;

        assert!(matches!(result, Err(CalibrationsErr::FileSysErr(_))));
        // the unreadable snapshot stays put, so the next open rebuilds again
        // rather than starting from an empty queue
        let raw = files::read_string(&fixture.queue_file()).await.unwrap();
        assert_eq!(raw, "{not json");
        let buffer = fixture.open().await;
        // same calibration time, so the rebuild's order between them is by id
        let mut rebuilt = names(&buffer);
        rebuilt.sort();
        assert_eq!(rebuilt, strings(&["a", "b"]));
    }

    /// A read error other than a parse failure may be transient, so the
    /// snapshot, with its attempt counts and order, stays for the next open.
    #[tokio::test]
    async fn snapshot_read_failure_fails_the_open_and_keeps_the_snapshot() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        let saved = files::read_string(&fixture.queue_file()).await.unwrap();
        files::delete(&fixture.queue_file()).await.unwrap();
        // a directory in its place reads with an I/O error, not a parse error
        let blocker = Dir::new(fixture.queue_file().path().clone());
        dirs::create(&blocker).await.unwrap();

        let result = Buffer::open(&fixture.layout(), BufferOptions::default()).await;

        assert!(matches!(result, Err(CalibrationsErr::FileSysErr(_))));
        assert!(!fixture.root.file("queue.json.corrupt").exists());
        assert!(fixture.report_file(ids[0]).exists());
        dirs::delete(&blocker).await.unwrap();
        files::write_string(
            &fixture.queue_file(),
            &saved,
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
        let buffer = fixture.open().await;
        assert_eq!(names(&buffer), strings(&["a"]));
    }

    /// A report file that cannot be read (as opposed to parsed) during a
    /// rebuild fails the open instead of being set aside.
    #[cfg(unix)]
    #[tokio::test]
    async fn report_read_failure_during_a_rebuild_fails_the_open() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        files::delete(&fixture.queue_file()).await.unwrap();
        let report = fixture.report_file(ids[0]);
        let mode = |bits| std::fs::Permissions::from_mode(bits);
        std::fs::set_permissions(report.path(), mode(0o000)).unwrap();

        let result = Buffer::open(&fixture.layout(), BufferOptions::default()).await;

        std::fs::set_permissions(report.path(), mode(0o600)).unwrap();
        assert!(matches!(result, Err(CalibrationsErr::FileSysErr(_))));
        assert!(report.exists());
        let buffer = fixture.open().await;
        assert_eq!(names(&buffer), strings(&["a"]));
    }

    #[tokio::test]
    async fn rebuilds_a_missing_queue_from_the_report_files() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        files::delete(&fixture.queue_file()).await.unwrap();

        let buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["a"]));
        assert_eq!(buffer.next_ready(now()).unwrap().id, ids[0]);
        assert!(fixture.report_file(ids[0]).exists());
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a"]));
    }

    #[tokio::test]
    async fn loads_a_backlog_over_a_lowered_cap() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b", "c"]).await;
        }
        let options = BufferOptions {
            max_reports: 2,
            ..BufferOptions::default()
        };

        let mut buffer = fixture.open_with(options).await;

        assert_eq!(buffer.len(), 3);
        let err = buffer.enqueue(report("d")).await.unwrap_err();
        assert!(matches!(err, CalibrationsErr::BufferFullErr(_)), "{err:?}");
    }

    #[tokio::test]
    async fn fails_when_the_buffer_root_is_a_file() {
        let fixture = Fixture::new();
        files::write_string(
            &File::new(fixture.root.path().clone()),
            "x",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let result = Buffer::open(&fixture.layout(), BufferOptions::default()).await;

        assert!(matches!(result, Err(CalibrationsErr::FileSysErr(_))));
    }
}

pub mod enqueue {
    use super::*;

    /// Every report in a single listing of the content directory.
    async fn content_files(fixture: &Fixture) -> Vec<File> {
        dirs::files(&fixture.layout().content()).await.unwrap()
    }

    #[tokio::test]
    async fn writes_the_report_file_and_the_queue_entry() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        let id = buffer.enqueue(report("a")).await.unwrap();

        let raw = files::read_string(&fixture.report_file(id)).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["content", "job"], "{raw}");
        assert_eq!(value["job"]["component_slug"], "a", "{raw}");
        assert_eq!(value["content"], report("a").content, "{raw}");
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a"]));
        let entries: serde_json::Value = files::read_json(&fixture.queue_file()).await.unwrap();
        assert_eq!(entries["entries"][0]["id"], id.to_string());
        assert_eq!(buffer.used_bytes(), raw.len() as u64);
    }

    #[tokio::test]
    async fn keeps_fifo_order() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        enqueue_all(&mut buffer, &["c", "a", "b"]).await;

        assert_eq!(drain(&mut buffer).await, strings(&["c", "a", "b"]));
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn identical_reports_are_both_buffered() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        let ids = enqueue_all(&mut buffer, &["a", "a"]).await;

        assert_ne!(ids[0], ids[1]);
        assert_eq!(drain(&mut buffer).await, strings(&["a", "a"]));
    }

    #[tokio::test]
    async fn rejects_at_the_report_cap() {
        let fixture = Fixture::new();
        let options = BufferOptions {
            max_reports: 2,
            ..BufferOptions::default()
        };
        let mut buffer = fixture.open_with(options).await;
        enqueue_all(&mut buffer, &["a", "b"]).await;

        let err = buffer.enqueue(report("c")).await.unwrap_err();

        let CalibrationsErr::BufferFullErr(err) = err else {
            panic!("expected BufferFullErr, got {err:?}");
        };
        assert_eq!((err.reports, err.max_reports), (2, 2));
        assert_eq!(content_files(&fixture).await.len(), 2);
        assert_eq!(names(&buffer), strings(&["a", "b"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a", "b"]));
    }

    #[tokio::test]
    async fn rejects_at_the_byte_cap() {
        let fixture = Fixture::new();
        let options = BufferOptions {
            max_reports: 100,
            max_bytes: 1000,
        };
        let mut buffer = fixture.open_with(options).await;
        buffer.enqueue(report_of_size("a", 600)).await.unwrap();
        // exactly at the cap is accepted
        buffer.enqueue(report_of_size("b", 400)).await.unwrap();
        assert_eq!(buffer.used_bytes(), 1000);

        let err = buffer.enqueue(report_of_size("c", 300)).await.unwrap_err();

        let CalibrationsErr::BufferFullErr(err) = err else {
            panic!("expected BufferFullErr, got {err:?}");
        };
        assert_eq!(
            (err.bytes, err.max_bytes, err.report_bytes),
            (1000, 1000, 300)
        );
        assert_eq!(content_files(&fixture).await.len(), 2);
        assert_eq!(names(&buffer), strings(&["a", "b"]));
    }

    #[tokio::test]
    async fn accepts_again_once_a_report_is_removed() {
        let fixture = Fixture::new();
        let options = BufferOptions {
            max_reports: 100,
            max_bytes: 1000,
        };
        let mut buffer = fixture.open_with(options).await;
        buffer.enqueue(report_of_size("a", 600)).await.unwrap();
        assert!(buffer.enqueue(report_of_size("b", 600)).await.is_err());

        let entry = buffer.next_ready(now()).unwrap();
        buffer.remove(entry.id).await.unwrap();

        buffer.enqueue(report_of_size("b", 600)).await.unwrap();
        assert_eq!(buffer.used_bytes(), 600);
    }

    #[tokio::test]
    async fn queue_write_failure_rejects_and_removes_the_report_file() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        buffer.enqueue(report("a")).await.unwrap();
        let used = buffer.used_bytes();

        // make queue.json unwritable: a DIRECTORY now sits there
        files::delete(&fixture.queue_file()).await.unwrap();
        dirs::create(&Dir::new(fixture.queue_file().path().clone()))
            .await
            .unwrap();

        let err = buffer.enqueue(report("b")).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::EnqueueErr(_)), "{err:?}");
        assert_eq!(content_files(&fixture).await.len(), 1);
        assert_eq!(names(&buffer), strings(&["a"]));
        assert_eq!(buffer.used_bytes(), used);
    }

    #[tokio::test]
    async fn report_file_write_failure_rejects() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        // a file where the content directory should be
        let content = fixture.layout().content();
        dirs::delete(&content).await.unwrap();
        files::write_string(
            &File::new(content.path().clone()),
            "x",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let err = buffer.enqueue(report("a")).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
        assert!(buffer.is_empty());
        assert!(queue_on_disk(&fixture).await.is_empty());
    }
}

pub mod remove {
    use super::*;

    #[tokio::test]
    async fn deletes_the_entry_and_its_report_file() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let ids = enqueue_all(&mut buffer, &["a", "b"]).await;

        let removed = buffer.remove(ids[0]).await.unwrap();

        assert_eq!(removed.job.component_slug, "a");
        assert!(!fixture.report_file(ids[0]).exists());
        assert!(fixture.report_file(ids[1]).exists());
        assert_eq!(queue_on_disk(&fixture).await, strings(&["b"]));
        assert_eq!(buffer.used_bytes(), serialized_len(&report("b")));
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        enqueue_all(&mut buffer, &["a"]).await;

        assert!(buffer.remove(Uuid::new_v4()).await.is_none());
        assert_eq!(buffer.len(), 1);
    }
}

pub mod requeue {
    use super::*;

    #[tokio::test]
    async fn backoff_survives_a_restart() {
        let fixture = Fixture::new();
        let deadline = now() + TimeDelta::minutes(5);
        let ids = {
            let mut buffer = fixture.open().await;
            let ids = enqueue_all(&mut buffer, &["a", "b"]).await;
            let entry = buffer.next_ready(now()).unwrap();
            buffer
                .requeue(QueueEntry {
                    attempts: 3,
                    next_attempt_at: Some(deadline),
                    ..entry
                })
                .await;
            assert_eq!(
                buffer.earliest_next_attempt(),
                Some(DateTime::<Utc>::MIN_UTC)
            );
            ids
        };

        let mut buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["b", "a"]));
        let ready = buffer.next_ready(now()).unwrap();
        assert_eq!(ready.id, ids[1]);
        buffer.remove(ready.id).await.unwrap();
        // `a` waits out its backoff, then comes back with its attempt count
        assert!(buffer.next_ready(now()).is_none());
        assert_eq!(buffer.earliest_next_attempt(), Some(deadline));
        let waiting = buffer.next_ready(deadline).unwrap();
        assert_eq!(waiting.id, ids[0]);
        assert_eq!(waiting.attempts, 3);
        assert!(buffer.read(ids[0]).await.is_ok());
    }
}

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_the_job_and_content() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let id = buffer.enqueue(report("a")).await.unwrap();

        let file = buffer.read(id).await.unwrap();

        assert_eq!(file.job, report("a").job);
        assert_eq!(file.content, report("a").content);
    }

    #[tokio::test]
    async fn errors_for_an_unknown_entry() {
        let fixture = Fixture::new();
        let buffer = fixture.open().await;

        let err = buffer.read(Uuid::new_v4()).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    }
}
