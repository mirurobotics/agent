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

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(5000, 0).unwrap()
}

fn job(name: &str, calibrated_at_secs: i64) -> CalibrationJob {
    CalibrationJob {
        report_id: format!("clb_rpt_{name}"),
        component_slug: "arm".to_string(),
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

    fn report_file(&self, name: &str) -> File {
        self.layout()
            .content()
            .file(&format!("clb_rpt_{name}.json"))
    }

    fn queue_file(&self) -> File {
        self.layout().queue()
    }
}

fn report_ids(buffer: &Buffer) -> Vec<String> {
    buffer.jobs().into_iter().map(|job| job.report_id).collect()
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| format!("clb_rpt_{name}")).collect()
}

/// Take and remove every ready report, oldest first.
async fn drain(buffer: &mut Buffer) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(entry) = buffer.next_ready(now()) {
        out.push(entry.job.report_id.clone());
        buffer.remove(entry.id).await.unwrap();
    }
    out
}

async fn enqueue_all(buffer: &mut Buffer, names: &[&str]) {
    for name in names {
        buffer.enqueue(report(name)).await.unwrap();
    }
}

/// The report ids `queue.json` holds, read straight from disk.
async fn queue_on_disk(fixture: &Fixture) -> Vec<String> {
    let value: serde_json::Value = files::read_json(&fixture.queue_file()).await.unwrap();
    value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["job"]["report_id"].as_str().unwrap().to_string())
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
        let used = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b", "c"]).await;
            buffer.used_bytes()
        };

        let mut buffer = fixture.open().await;

        assert_eq!(report_ids(&buffer), ids(&["a", "b", "c"]));
        assert_eq!(buffer.used_bytes(), used);
        let file = buffer.read("clb_rpt_b").await.unwrap();
        assert_eq!(file.job, report("b").job);
        assert_eq!(file.content, report("b").content);
        assert_eq!(drain(&mut buffer).await, ids(&["a", "b", "c"]));
    }

    #[tokio::test]
    async fn removes_report_files_with_no_queue_entry() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await;
        }
        // a crash between the report file and the queue entry leaves this
        let orphan = fixture.report_file("orphan");
        files::write_string(&orphan, "{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let buffer = fixture.open().await;

        assert!(!orphan.exists());
        assert!(fixture.report_file("a").exists());
        assert_eq!(report_ids(&buffer), ids(&["a"]));
    }

    #[tokio::test]
    async fn leaves_files_that_are_not_report_files() {
        let fixture = Fixture::new();
        let content = fixture.layout().content();
        let others = [
            content.file("clb_rpt_a.json.corrupt"),
            content.file(".clb_rpt_b.json.tmp"),
            content.file("notes.txt"),
            content.file("bad.id.json"),
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
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b", "c"]).await;
        }
        files::delete(&fixture.report_file("b")).await.unwrap();

        let buffer = fixture.open().await;

        assert_eq!(report_ids(&buffer), ids(&["a", "c"]));
        assert_eq!(queue_on_disk(&fixture).await, ids(&["a", "c"]));
        assert_eq!(
            buffer.used_bytes(),
            serialized_len(&report("a")) + serialized_len(&report("c"))
        );
    }

    #[tokio::test]
    async fn rebuilds_an_unreadable_queue_from_the_report_files() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            // enqueued out of calibration order; the rebuild orders by
            // calibrated_at since the lost queue order cannot be recovered
            for (name, secs) in [("late", 3000), ("early", 1000), ("middle", 2000)] {
                let report = Report {
                    job: job(name, secs),
                    ..report(name)
                };
                buffer.enqueue(report).await.unwrap();
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
        files::write_string(
            &fixture.report_file("broken"),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let mut buffer = fixture.open().await;

        assert_eq!(report_ids(&buffer), ids(&["early", "middle", "late"]));
        assert_eq!(
            queue_on_disk(&fixture).await,
            ids(&["early", "middle", "late"])
        );
        let aside = fixture.root.file("queue.json.corrupt");
        assert_eq!(files::read_string(&aside).await.unwrap(), "{not json");
        let content = fixture.layout().content();
        assert!(content.file("clb_rpt_broken.json.corrupt").exists());
        assert!(!fixture.report_file("broken").exists());
        assert_eq!(drain(&mut buffer).await, ids(&["early", "middle", "late"]));
    }

    #[tokio::test]
    async fn rebuild_sets_aside_a_report_file_under_the_wrong_name() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await;
        }
        files::copy_to(
            &fixture.report_file("a"),
            &fixture.report_file("other"),
            miru_agent::filesys::Overwrite::Deny,
        )
        .await
        .unwrap();
        files::write_string(&fixture.queue_file(), "[]", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let buffer = fixture.open().await;

        assert_eq!(report_ids(&buffer), ids(&["a"]));
        let content = fixture.layout().content();
        assert!(content.file("clb_rpt_other.json.corrupt").exists());
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

    #[tokio::test]
    async fn writes_the_report_file_and_the_queue_entry() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        buffer.enqueue(report("a")).await.unwrap();

        let raw = files::read_string(&fixture.report_file("a")).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["content", "job"], "{raw}");
        assert_eq!(value["job"]["report_id"], "clb_rpt_a", "{raw}");
        assert_eq!(value["content"], report("a").content, "{raw}");
        assert_eq!(queue_on_disk(&fixture).await, ids(&["a"]));
        assert_eq!(buffer.used_bytes(), raw.len() as u64);
    }

    #[tokio::test]
    async fn keeps_fifo_order() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        enqueue_all(&mut buffer, &["c", "a", "b"]).await;

        assert_eq!(drain(&mut buffer).await, ids(&["c", "a", "b"]));
        assert!(buffer.is_empty());
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
        assert_eq!(err.report_id, "clb_rpt_c");
        assert!(!fixture.report_file("c").exists());
        assert_eq!(report_ids(&buffer), ids(&["a", "b"]));
        assert_eq!(queue_on_disk(&fixture).await, ids(&["a", "b"]));
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
        assert!(!fixture.report_file("c").exists());
        assert_eq!(report_ids(&buffer), ids(&["a", "b"]));
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
    async fn rejects_a_duplicate_report_id() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        buffer.enqueue(report("a")).await.unwrap();
        let before = files::read_string(&fixture.report_file("a")).await.unwrap();

        let mut duplicate = report("a");
        duplicate.content = json!("different");
        let err = buffer.enqueue(duplicate).await.unwrap_err();

        assert!(
            matches!(err, CalibrationsErr::DuplicateReportErr(_)),
            "{err:?}"
        );
        let after = files::read_string(&fixture.report_file("a")).await.unwrap();
        assert_eq!(before, after);
        assert_eq!(buffer.len(), 1);
    }

    #[tokio::test]
    async fn rejects_report_ids_unsafe_as_file_names() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let long = "a".repeat(129);
        for bad in [
            "",
            "../escape",
            "a/b",
            "a\\b",
            "C:x",
            "a.b",
            "a b",
            long.as_str(),
        ] {
            let mut report = report("x");
            report.job.report_id = bad.to_string();

            let err = buffer.enqueue(report).await.unwrap_err();

            assert!(
                matches!(err, CalibrationsErr::InvalidReportIdErr(_)),
                "{bad:?}: {err:?}"
            );
        }
        assert!(buffer.is_empty());
        let listing = dirs::files(&fixture.layout().content()).await.unwrap();
        assert!(listing.is_empty(), "{listing:?}");
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
        assert!(!fixture.report_file("b").exists());
        assert_eq!(report_ids(&buffer), ids(&["a"]));
        assert_eq!(buffer.used_bytes(), used);
    }

    #[tokio::test]
    async fn report_file_write_failure_rejects() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        // a directory where the report file would go
        dirs::create(&Dir::new(fixture.report_file("a").path().clone()))
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
        enqueue_all(&mut buffer, &["a", "b"]).await;

        let entry = buffer.next_ready(now()).unwrap();
        let removed = buffer.remove(entry.id).await.unwrap();

        assert_eq!(removed.job.report_id, "clb_rpt_a");
        assert!(!fixture.report_file("a").exists());
        assert!(fixture.report_file("b").exists());
        assert_eq!(queue_on_disk(&fixture).await, ids(&["b"]));
        assert_eq!(buffer.used_bytes(), serialized_len(&report("b")));
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        enqueue_all(&mut buffer, &["a"]).await;

        assert!(buffer.remove(uuid::Uuid::new_v4()).await.is_none());
        assert_eq!(buffer.len(), 1);
    }
}

pub mod requeue {
    use super::*;

    #[tokio::test]
    async fn backoff_survives_a_restart() {
        let fixture = Fixture::new();
        let deadline = now() + TimeDelta::minutes(5);
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b"]).await;
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
        }

        let mut buffer = fixture.open().await;

        assert_eq!(report_ids(&buffer), ids(&["b", "a"]));
        let ready = buffer.next_ready(now()).unwrap();
        assert_eq!(ready.job.report_id, "clb_rpt_b");
        buffer.remove(ready.id).await.unwrap();
        // `a` waits out its backoff, then comes back with its attempt count
        assert!(buffer.next_ready(now()).is_none());
        assert_eq!(buffer.earliest_next_attempt(), Some(deadline));
        let waiting = buffer.next_ready(deadline).unwrap();
        assert_eq!(waiting.job.report_id, "clb_rpt_a");
        assert_eq!(waiting.attempts, 3);
        assert!(buffer.read("clb_rpt_a").await.is_ok());
    }
}

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_the_job_and_content() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        buffer.enqueue(report("a")).await.unwrap();

        let file = buffer.read("clb_rpt_a").await.unwrap();

        assert_eq!(file.job, report("a").job);
        assert_eq!(file.content, report("a").content);
    }

    #[tokio::test]
    async fn errors_for_an_unknown_or_invalid_report() {
        let fixture = Fixture::new();
        let buffer = fixture.open().await;

        let missing = buffer.read("clb_rpt_missing").await.unwrap_err();
        assert!(
            matches!(missing, CalibrationsErr::FileSysErr(_)),
            "{missing:?}"
        );
        let invalid = buffer.read("../queue").await.unwrap_err();
        assert!(
            matches!(invalid, CalibrationsErr::InvalidReportIdErr(_)),
            "{invalid:?}"
        );
    }
}
