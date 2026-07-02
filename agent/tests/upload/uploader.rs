// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use crate::upload::discovery::make_rule;
use miru_agent::filesys::{self, PathExt, WriteOptions};
use miru_agent::models::UploadRule;
use miru_agent::upload::{
    uploader::{Options, ReadyFile, SingleThreadUploader, Uploader, Worker},
    ScanOutcome, UploaderExt,
};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

// ========================= FIXTURE ========================= //

/// A hand-advanced clock injected into the uploader so cadence and stability
/// windows are tested deterministically, without sleeping.
#[derive(Clone)]
struct TestClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl TestClock {
    fn new() -> Self {
        Self {
            now: Arc::new(Mutex::new(Utc::now())),
        }
    }

    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }

    fn advance(&self, delta: TimeDelta) {
        *self.now.lock().unwrap() += delta;
    }
}

async fn spawn_uploader(poll_interval_secs: i64, clock: &TestClock) -> (Uploader, JoinHandle<()>) {
    let (uploader, handle) = Uploader::spawn(16, Options { poll_interval_secs }).unwrap();
    let clock = clock.clone();
    uploader.set_now_fn(move || clock.now()).await.unwrap();
    (uploader, handle)
}

async fn write_file(dir: &filesys::Dir, name: &str, content: &str) -> filesys::File {
    let file = dir.file(name);
    file.write_string(content, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    file
}

fn glob_for(dir: &filesys::Dir, pattern: &str) -> String {
    format!("{}/{}", dir.path().display(), pattern)
}

fn assert_scanned(outcome: ScanOutcome, expected: Vec<ReadyFile>) {
    assert_eq!(outcome, ScanOutcome::Completed(expected));
}

async fn ready_file(file: &filesys::File) -> ReadyFile {
    let modified_at =
        DateTime::<Utc>::from(std::fs::metadata(file.path()).unwrap().modified().unwrap());
    ReadyFile {
        path: file.path().clone(),
        modified_at,
    }
}

pub mod cadence {
    use super::*;

    #[tokio::test]
    async fn scans_are_gated_by_the_global_poll_interval() {
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(60, &clock).await;

        // the first scan is always due
        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        // not yet due: skipped without scanning
        assert_eq!(uploader.scan().await.unwrap(), ScanOutcome::NotDue);
        clock.advance(TimeDelta::seconds(59));
        assert_eq!(uploader.scan().await.unwrap(), ScanOutcome::NotDue);

        // due once the full interval has elapsed
        clock.advance(TimeDelta::seconds(1));
        assert_scanned(uploader.scan().await.unwrap(), vec![]);
    }
}

pub mod scan {
    use super::*;

    #[tokio::test]
    async fn empty_rule_set_idles() {
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(0, &clock).await;

        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        uploader.update_rules(Vec::new()).await.unwrap();
        assert_scanned(uploader.scan().await.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn stability_window_gates_readiness() {
        let dir = filesys::Dir::create_temp_dir("uploader-stability")
            .await
            .unwrap();
        let file = write_file(&dir, "a.log", "hello").await;
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(0, &clock).await;
        uploader
            .update_rules(vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 60)])
            .await
            .unwrap();

        // first sighting starts the stability window
        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        // still within the window
        clock.advance(TimeDelta::seconds(59));
        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        // unchanged for the full window: ready
        clock.advance(TimeDelta::seconds(1));
        assert_scanned(
            uploader.scan().await.unwrap(),
            vec![ready_file(&file).await],
        );
    }

    #[tokio::test]
    async fn file_change_resets_the_stability_window() {
        let dir = filesys::Dir::create_temp_dir("uploader-reset")
            .await
            .unwrap();
        write_file(&dir, "a.log", "hello").await;
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(0, &clock).await;
        uploader
            .update_rules(vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 60)])
            .await
            .unwrap();

        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        // the file changes mid-window: the watermark restarts
        clock.advance(TimeDelta::seconds(30));
        let file = write_file(&dir, "a.log", "hello, but longer").await;
        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        // 60s after the first sighting but only 30s after the change
        clock.advance(TimeDelta::seconds(30));
        assert_scanned(uploader.scan().await.unwrap(), vec![]);

        // 60s after the change: ready
        clock.advance(TimeDelta::seconds(30));
        assert_scanned(
            uploader.scan().await.unwrap(),
            vec![ready_file(&file).await],
        );
    }

    #[tokio::test]
    async fn ready_files_are_reported_once() {
        let dir = filesys::Dir::create_temp_dir("uploader-dedupe")
            .await
            .unwrap();
        let file = write_file(&dir, "a.log", "hello").await;
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(0, &clock).await;
        uploader
            .update_rules(vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 0)])
            .await
            .unwrap();

        assert_scanned(
            uploader.scan().await.unwrap(),
            vec![ready_file(&file).await],
        );

        // subsequent scans dedupe the already-reported file
        clock.advance(TimeDelta::seconds(60));
        assert_scanned(uploader.scan().await.unwrap(), vec![]);
        clock.advance(TimeDelta::seconds(60));
        assert_scanned(uploader.scan().await.unwrap(), vec![]);
    }
}

pub mod update_rules {
    use super::*;

    #[tokio::test]
    async fn replaces_the_active_rule_set() {
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(0, &clock).await;

        let first = vec![make_rule("rule_1", "/data/*.log", 60)];
        uploader.update_rules(first.clone()).await.unwrap();
        assert_eq!(uploader.get_rules().await.unwrap(), first);

        let second = vec![make_rule("rule_2", "/data/*.mcap", 30)];
        uploader.update_rules(second.clone()).await.unwrap();
        assert_eq!(uploader.get_rules().await.unwrap(), second);
    }

    #[tokio::test]
    async fn replaced_rules_stop_matching() {
        let dir = filesys::Dir::create_temp_dir("uploader-replace")
            .await
            .unwrap();
        let log_file = write_file(&dir, "a.log", "hello").await;
        let txt_file = write_file(&dir, "b.txt", "world").await;
        let clock = TestClock::new();
        let (uploader, _) = spawn_uploader(0, &clock).await;

        uploader
            .update_rules(vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 0)])
            .await
            .unwrap();
        assert_scanned(
            uploader.scan().await.unwrap(),
            vec![ready_file(&log_file).await],
        );

        // replace the rule set; only the new rule's matches are reported
        uploader
            .update_rules(vec![make_rule("rule_2", &glob_for(&dir, "*.txt"), 0)])
            .await
            .unwrap();
        write_file(&dir, "c.log", "no longer matched").await;
        clock.advance(TimeDelta::seconds(60));
        assert_scanned(
            uploader.scan().await.unwrap(),
            vec![ready_file(&txt_file).await],
        );
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn shutdown() {
        let (uploader, handle) = Uploader::spawn(16, Options::default()).unwrap();
        uploader.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_a_manually_spawned_worker() {
        let (sender, receiver) = mpsc::channel(16);
        let worker = Worker::new(SingleThreadUploader::new(Options::default()), receiver);
        let handle = tokio::spawn(worker.run());
        let uploader = Uploader::new(sender);

        uploader.shutdown().await.unwrap();
        handle.await.unwrap();
    }
}

pub mod options {
    use super::*;

    #[test]
    fn default_poll_interval() {
        let options = Options::default();
        assert_eq!(options.poll_interval_secs, 60);
    }
}

pub mod get_rules {
    use super::*;

    #[tokio::test]
    async fn starts_empty() {
        let (uploader, _) = Uploader::spawn(16, Options::default()).unwrap();
        assert_eq!(
            uploader.get_rules().await.unwrap(),
            Vec::<UploadRule>::new()
        );
    }
}
