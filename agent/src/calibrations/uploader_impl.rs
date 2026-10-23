// standard crates
use std::collections::HashSet;
use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;

// internal crates
use crate::calibrations::{
    errors::{CalibrationTooLargeErr, CalibrationsErr, InvalidReportErr},
    report::{Queue, QueueEntry, QueueSnapshotFile, Report, ReportMetadata},
    sender::Sender,
};
use crate::errors::Error;
use crate::filesys::{dirs, files, Dir, File, FileSysErr, PathExt, WriteOptions};
use crate::queue::{self, Persistence, RetryPolicy, Verdict};
use crate::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UploaderOptions {
    /// The most reports queued at once; intake is rejected beyond it.
    pub capacity: usize,
    /// The largest report data accepted, in bytes.
    pub max_report_bytes: u64,
    /// When a failed report is retried and when it is dropped. Its age is
    /// measured from `reported_at`.
    pub retry: RetryPolicy,
}

impl Default for UploaderOptions {
    /// At most 1,000 reports of up to 1 MiB of data each (1 GiB at worst),
    /// the most calibration data Miru accepts, retried as the data uploads
    /// are.
    fn default() -> Self {
        Self {
            capacity: 1000,
            max_report_bytes: 1024 * 1024,
            retry: RetryPolicy::default(),
        }
    }
}

/// What the queue holds for the worker to do next.
pub(crate) type NextAttempt = queue::NextAttempt<ReportMetadata>;

/// Single-threaded calibration uploader state and logic: the queue of report
/// metadata, the directory of report files, and the sender that uploads them.
/// [`Uploader`] runs it inside an actor and does the waiting.
///
/// Each queued report's data is its own file, `<reports>/<entry id>`, so large
/// data is never rewritten with the queue. A failed report is
/// retried at the tail of the queue after its own backoff, as in
/// `data_uploads::upload`, so one failing report never blocks the others.
///
/// [`Uploader`]: crate::calibrations::Uploader
pub struct UploaderImpl<S, N> {
    sender: Arc<S>,
    queue: Queue,
    reports: Dir,
    options: UploaderOptions,
    now_fn: N,
}

impl<S, N> UploaderImpl<S, N>
where
    S: Sender + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    /// Without a `snapshot_file` the queue lives in memory, so reports still
    /// queued at a restart are lost.
    pub(crate) fn new(
        sender: Arc<S>,
        snapshot_file: Option<QueueSnapshotFile>,
        reports: Dir,
        options: UploaderOptions,
        now_fn: N,
    ) -> Self {
        let queue = match snapshot_file {
            Some(file) => Queue::from_snapshot(options.capacity, file, Persistence::Durable),
            None => Queue::new(options.capacity),
        };
        Self {
            sender,
            queue,
            reports,
            options,
            now_fn,
        }
    }

    /// Delete report files with no queue entry, left behind by a crash
    /// between dequeuing a report and deleting its file. Failures are logged
    /// and retried on the next boot.
    pub(crate) async fn prune_orphans(&self) {
        if !self.reports.exists() {
            return;
        }
        let listing = match dirs::files(&self.reports).await {
            Ok(listing) => listing,
            Err(e) => {
                warn!("calibrations: failed to list report files: {e}");
                return;
            }
        };
        let queued: HashSet<Uuid> = self.queue.iter().map(|entry| entry.id).collect();
        for file in listing {
            if report_id(&file).is_some_and(|id| queued.contains(&id)) {
                continue;
            }
            warn!("calibrations: deleting report file {file}, which is not queued");
            delete(&file).await;
        }
    }

    /// Queue `report` and return its id. The queue entry is written before
    /// the report file, so a crash between the two leaves an entry without a
    /// file, which [`Self::attempt`] fails for good. On error nothing is queued.
    pub(crate) async fn enqueue(&mut self, report: Report) -> Result<Uuid, CalibrationsErr> {
        let bytes = report.data.into_bytes();
        self.verify_size(bytes.len() as u64)?;
        let id = self.queue.enqueue(report.metadata).await?;
        let file = self.report_file(id);
        if let Err(e) = files::write_bytes(&file, &bytes, WriteOptions::OVERWRITE_ATOMIC).await {
            self.queue.ack(id).await;
            return Err(e.into());
        }
        Ok(id)
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// The report to attempt now, or how long until one is due.
    pub(crate) async fn next_attempt(&mut self) -> NextAttempt {
        let now = (self.now_fn)();
        let horizon = self.options.retry.horizon(now);
        self.queue.next_attempt(now, horizon).await
    }

    /// Upload the report queued as `entry`: read its file, then send it. The
    /// future owns what it needs and borrows nothing from `self`, so the
    /// worker can serve commands while it runs. The metadata is cloned into
    /// the future, which is cheap: the data is read from disk inside it.
    pub(crate) fn attempt(
        &self,
        entry: &QueueEntry,
    ) -> impl Future<Output = Result<(), CalibrationsErr>> {
        let sender = self.sender.clone();
        let file = self.report_file(entry.id);
        let metadata = entry.job.clone();
        async move {
            let data = read(&file).await?;
            sender.send(Report { metadata, data }).await
        }
    }

    /// Settle `entry` once its attempt ended in `result`: remove the report
    /// when sent, otherwise retry or drop it as the retry policy says.
    pub(crate) async fn finish_attempt(
        &mut self,
        entry: QueueEntry,
        result: Result<(), CalibrationsErr>,
    ) {
        match result {
            Ok(()) => {
                Self::log_success(&entry);
                self.remove(entry.id).await;
            }
            Err(err) => self.handle_failure(entry, err).await,
        }
    }

    /// Settle a failed attempt as the retry policy says. Network connection
    /// errors are expected and don't count toward the attempt budget; any
    /// other failure does.
    async fn handle_failure(&mut self, mut entry: QueueEntry, err: CalibrationsErr) {
        let age = (self.now_fn)() - entry.job.reported_at;
        match self.options.retry.on_failure(&err, entry.attempts, age) {
            Verdict::Retry {
                attempts,
                wait_secs,
            } => {
                entry.attempts = attempts;
                Self::log_failure(&entry, &err);
                self.retry_after(entry.id, attempts, wait_secs).await;
            }
            Verdict::Terminal { .. } => {
                Self::log_terminal_drop(&entry, &err);
                self.remove(entry.id).await;
            }
            Verdict::OutOfAttempts { attempts } => {
                entry.attempts = attempts;
                Self::log_dropped(&entry, &err);
                self.remove(entry.id).await;
            }
            Verdict::TooOld { age, .. } => {
                Self::log_age_drop(&entry, age, &err);
                self.remove(entry.id).await;
            }
        }
    }

    /// Move report `id` to the tail, due again in `wait_secs`.
    async fn retry_after(&mut self, id: Uuid, attempts: u32, wait_secs: i64) {
        let next_attempt_at = (self.now_fn)() + TimeDelta::seconds(wait_secs.max(0));
        self.queue.retry(id, attempts, next_attempt_at).await;
    }

    /// Dequeue report `id`, then delete its file. The entry goes first, so a
    /// crash between the two leaves a file that [`Self::prune_orphans`] deletes.
    async fn remove(&mut self, id: Uuid) {
        self.queue.ack(id).await;
        delete(&self.report_file(id)).await;
    }

    fn verify_size(&self, report_bytes: u64) -> Result<(), CalibrationTooLargeErr> {
        if report_bytes <= self.options.max_report_bytes {
            return Ok(());
        }
        let err = CalibrationTooLargeErr {
            report_bytes,
            max_bytes: self.options.max_report_bytes,
            trace: trace!(),
        };
        warn!("calibrations: rejecting report: {err}");
        Err(err)
    }

    fn report_file(&self, id: Uuid) -> File {
        self.reports.file(&id.hyphenated().to_string())
    }

    /// A network error logs at debug, since it repeats every base backoff
    /// while the device is offline; any other failure counts and logs at warn.
    fn log_failure(entry: &QueueEntry, err: &CalibrationsErr) {
        if err.is_network_conn_err() {
            Self::log_network_conn_err(entry, err);
        } else {
            Self::log_counted_failure(entry, err);
        }
    }

    fn log_network_conn_err(entry: &QueueEntry, err: &CalibrationsErr) {
        let id = entry.id;
        debug!("calibrations: failed to reach Miru with report {id}; will retry: {err}");
    }

    fn log_counted_failure(entry: &QueueEntry, err: &CalibrationsErr) {
        let (id, attempts) = (entry.id, entry.attempts);
        warn!("calibrations: attempt {attempts} to upload report {id} failed; will retry: {err}");
    }

    fn log_success(entry: &QueueEntry) {
        info!("calibrations: uploaded report {}", entry.id);
    }

    fn log_dropped(entry: &QueueEntry, err: &CalibrationsErr) {
        let (id, attempts) = (entry.id, entry.attempts);
        error!("calibrations: dropping report {id} after {attempts} attempts: {err}");
    }

    fn log_age_drop(entry: &QueueEntry, age: TimeDelta, err: &CalibrationsErr) {
        let (id, days) = (entry.id, age.num_days());
        error!("calibrations: dropping report {id}, still unsent after {days} days: {err}");
    }

    fn log_terminal_drop(entry: &QueueEntry, err: &CalibrationsErr) {
        let id = entry.id;
        error!("calibrations: dropping report {id}, which can never be uploaded: {err}");
    }
}

/// The report id a report file is named after, or `None` for any other file.
fn report_id(file: &File) -> Option<Uuid> {
    file.name().ok().and_then(|name| Uuid::from_str(name).ok())
}

/// Delete `file`, logging a failure. A file that is already gone is fine.
async fn delete(file: &File) {
    if let Err(e) = files::delete(file).await {
        warn!("calibrations: failed to delete report file {file}: {e}");
    }
}

/// The data in report `file`. A missing file or one that isn't UTF-8 can
/// never be sent, so either is a terminal [`InvalidReportErr`]; any other
/// read error is retried.
async fn read(file: &File) -> Result<String, CalibrationsErr> {
    match files::read_string(file).await {
        Ok(data) => Ok(data),
        Err(FileSysErr::PathDoesNotExistErr(_)) => Err(invalid("the report file is missing")),
        Err(FileSysErr::ConvertUTF8Err(e)) => {
            Err(invalid(&format!("the report file is not UTF-8: {e}")))
        }
        Err(e) => Err(e.into()),
    }
}

fn invalid(reason: &str) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: reason.to_string(),
        trace: trace!(),
    })
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // internal crates
    use super::{NextAttempt, QueueEntry, UploaderImpl, UploaderOptions};
    use crate::test_utils::calibration_sender::{MockCalibrationSender, Step};
    use crate::test_utils::calibrations::{layout, received_at, report};
    use crate::test_utils::filesys::dirs as test_dirs;
    use miru_agent::calibrations::{CalibrationsErr, Queue, QueueSnapshotFile};
    use miru_agent::disk::layout::CalibrationsLayout;
    use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};
    use miru_agent::queue::{Persistence, RetryPolicy};

    // external crates
    use chrono::{DateTime, TimeDelta, Utc};
    use uuid::Uuid;

    type Clock = Box<dyn Fn() -> DateTime<Utc> + Send + Sync>;
    type TestUploader = UploaderImpl<MockCalibrationSender, Clock>;

    struct Fixture {
        _dir: test_dirs::TempDir,
        layout: CalibrationsLayout,
        sender: Arc<MockCalibrationSender>,
        /// The uploader's clock. It starts when the fixture reports were
        /// received and moves only when a test advances it.
        clock: Arc<Mutex<DateTime<Utc>>>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = test_dirs::temp("calibration_uploader_impl_test").unwrap();
            let layout = layout(&dir);
            let (sender, _) = MockCalibrationSender::new();
            Self {
                _dir: dir,
                layout,
                sender,
                clock: Arc::new(Mutex::new(received_at())),
            }
        }

        async fn snapshot_file(&self) -> QueueSnapshotFile {
            QueueSnapshotFile::new_with_default(self.layout.queue(), Default::default())
                .await
                .unwrap()
        }

        /// The queue file's entries, as the next boot would load them.
        async fn queued_on_disk(&self) -> Vec<QueueEntry> {
            let snapshot_file = self.snapshot_file().await;
            let queue = Queue::from_snapshot(1000, snapshot_file, Persistence::Durable);
            queue.iter().cloned().collect()
        }

        async fn uploader(&self) -> TestUploader {
            self.uploader_with(UploaderOptions::default()).await
        }

        async fn uploader_with(&self, options: UploaderOptions) -> TestUploader {
            let snapshot_file = Some(self.snapshot_file().await);
            self.uploader_on(snapshot_file, options)
        }

        fn uploader_on(
            &self,
            snapshot_file: Option<QueueSnapshotFile>,
            options: UploaderOptions,
        ) -> TestUploader {
            let clock = self.clock.clone();
            let now_fn: Clock = Box::new(move || *clock.lock().unwrap());
            UploaderImpl::new(
                self.sender.clone(),
                snapshot_file,
                self.layout.reports(),
                options,
                now_fn,
            )
        }

        /// Move the uploader's clock forward by `by`.
        fn advance(&self, by: TimeDelta) {
            *self.clock.lock().unwrap() += by;
        }

        fn advance_secs(&self, secs: u64) {
            self.advance(TimeDelta::seconds(secs as i64));
        }

        fn report_file(&self, id: Uuid) -> File {
            self.layout.reports().file(&id.to_string())
        }

        fn sent(&self) -> Vec<String> {
            self.sender.sent()
        }
    }

    /// Queue a report per slug at `f`'s layout, oldest first, as a previous
    /// run would have left them: metadata in `queue.json`, data in `reports/`.
    async fn preloaded(f: &Fixture, slugs: &[&str]) -> Vec<Uuid> {
        let mut queue = Queue::from_snapshot(1000, f.snapshot_file().await, Persistence::Durable);
        let mut ids = Vec::with_capacity(slugs.len());
        for slug in slugs {
            let report = report(slug);
            let id = queue.enqueue(report.metadata).await.unwrap();
            files::write_string(
                &f.report_file(id),
                &report.data,
                WriteOptions::OVERWRITE_ATOMIC,
            )
            .await
            .unwrap();
            ids.push(id);
        }
        ids
    }

    /// Upload the report that `next_attempt` says is due, then settle it, as
    /// the worker's `run_attempt` does. Panics if no report is due.
    async fn run_next_attempt(u: &mut TestUploader) {
        let next = u.next_attempt().await;
        let NextAttempt::Ready(entry) = next else {
            panic!("no report is due: {next:?}");
        };
        let result = u.attempt(&entry).await;
        u.finish_attempt(entry, result).await;
    }

    /// Assert that every report is backing off and the earliest is due in `secs`.
    async fn assert_backoff(u: &mut TestUploader, secs: u64) {
        let wait = Duration::from_secs(secs);
        assert_eq!(u.next_attempt().await, NextAttempt::Backoff(wait));
    }

    async fn assert_empty(u: &mut TestUploader) {
        assert_eq!(u.next_attempt().await, NextAttempt::Empty);
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    // =============================== OPTIONS ================================ //
    #[test]
    fn default_options_are_1000_reports_of_1_mib() {
        let options = UploaderOptions::default();
        assert_eq!(options.capacity, 1000);
        assert_eq!(options.max_report_bytes, 1024 * 1024);
    }

    // ================================= NEW ================================== //
    #[tokio::test]
    async fn without_a_queue_file_the_queue_lives_in_memory() {
        let f = Fixture::new();
        let mut u = f.uploader_on(None, UploaderOptions::default());

        u.enqueue(report("a")).await.unwrap();

        assert_eq!(u.len(), 1);
        assert!(!f.layout.queue().exists());
    }

    // ============================ PRUNE ORPHANS ============================= //
    #[tokio::test]
    async fn a_missing_reports_directory_is_not_created() {
        let f = Fixture::new();
        let u = f.uploader().await;

        u.prune_orphans().await;

        assert!(!f.layout.reports().exists());
    }

    #[tokio::test]
    async fn an_unlistable_reports_directory_is_left_alone() {
        let f = Fixture::new();
        // a regular file where the reports directory should be
        let blocker = File::new(f.layout.reports().path().clone());
        files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let u = f.uploader().await;

        u.prune_orphans().await;

        assert!(blocker.exists());
    }

    #[tokio::test]
    async fn report_files_with_no_queue_entry_are_pruned() {
        let f = Fixture::new();
        let ids = preloaded(&f, &["a"]).await;
        let stray = f.report_file(Uuid::new_v4());
        let other = f.layout.reports().file("notes.txt");
        for file in [&stray, &other] {
            files::write_string(file, "{}", WriteOptions::OVERWRITE_ATOMIC)
                .await
                .unwrap();
        }
        let u = f.uploader().await;

        u.prune_orphans().await;

        let left = dirs::files(&f.layout.reports()).await.unwrap();
        assert_eq!(left, vec![f.report_file(ids[0])]);
    }

    // =============================== ENQUEUE ================================ //
    #[tokio::test]
    async fn stores_metadata_in_the_queue_and_data_in_a_report_file() {
        let f = Fixture::new();
        let mut u = f.uploader().await;

        let id = u.enqueue(report("a")).await.unwrap();

        let entry = QueueEntry {
            id,
            job: report("a").metadata,
            attempts: 0,
            next_attempt_at: None,
        };
        assert_eq!(f.queued_on_disk().await, [entry]);
        let data = files::read_string(&f.report_file(id)).await.unwrap();
        assert_eq!(data, report("a").data);
    }

    #[tokio::test]
    async fn an_oversized_report_is_rejected_and_writes_nothing() {
        let f = Fixture::new();
        let options = UploaderOptions {
            max_report_bytes: 100,
            ..UploaderOptions::default()
        };
        let mut u = f.uploader_with(options).await;
        let mut big = report("arm");
        big.data = "x".repeat(200);

        let err = u.enqueue(big).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::TooLargeErr(_)), "{err:?}");
        assert_eq!(u.len(), 0);
        assert!(!f.layout.reports().exists());
    }

    #[tokio::test]
    async fn a_full_queue_rejects_a_report_and_keeps_what_it_holds() {
        let f = Fixture::new();
        let options = UploaderOptions {
            capacity: 1,
            ..UploaderOptions::default()
        };
        let mut u = f.uploader_with(options).await;
        u.enqueue(report("a")).await.unwrap();

        let err = u.enqueue(report("b")).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::QueueFullErr(_)), "{err:?}");
        assert_eq!(u.len(), 1);
    }

    #[tokio::test]
    async fn a_queue_write_failure_queues_nothing() {
        let f = Fixture::new();
        let mut u = f.uploader().await;
        // a non-empty directory where the queue file should be can't be replaced
        files::delete(&f.layout.queue()).await.unwrap();
        let blocker = f.layout.root.subdir("queue.json");
        files::write_string(&blocker.file("x"), "x", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let err = u.enqueue(report("a")).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
        assert_eq!(u.len(), 0);
        assert!(!f.layout.reports().exists());
    }

    #[tokio::test]
    async fn a_report_file_write_failure_queues_nothing() {
        let f = Fixture::new();
        // a regular file where the reports directory should be
        let blocker = File::new(f.layout.reports().path().clone());
        files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let mut u = f.uploader().await;

        let err = u.enqueue(report("a")).await.unwrap_err();

        assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
        assert_eq!(u.len(), 0);
    }

    // ============================= NEXT ATTEMPT ============================= //
    #[tokio::test]
    async fn a_deadline_past_the_longest_backoff_is_pulled_in() {
        let f = Fixture::new();
        let ids = preloaded(&f, &["a"]).await;
        // a deadline a clock jump left far beyond the 1 h cap
        let mut queue = Queue::from_snapshot(1000, f.snapshot_file().await, Persistence::Durable);
        let far = received_at() + TimeDelta::days(30);
        assert!(queue.retry(ids[0], 1, far).await);
        let mut u = f.uploader().await;

        assert_backoff(&mut u, 3600).await;
    }

    // =============================== ATTEMPT ================================ //
    #[tokio::test]
    async fn a_report_whose_file_is_missing_is_dropped() {
        let f = Fixture::new();
        let ids = preloaded(&f, &["a"]).await;
        files::delete(&f.report_file(ids[0])).await.unwrap();
        let mut u = f.uploader().await;

        run_next_attempt(&mut u).await;

        assert!(f.sent().is_empty());
        assert_empty(&mut u).await;
    }

    #[tokio::test]
    async fn an_unreadable_report_is_dropped_without_sending() {
        let f = Fixture::new();
        let ids = preloaded(&f, &["a"]).await;
        files::write_bytes(
            &f.report_file(ids[0]),
            &[0xff, 0xfe],
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
        let mut u = f.uploader().await;

        run_next_attempt(&mut u).await;

        assert!(f.sent().is_empty());
        assert_empty(&mut u).await;
        assert!(!f.report_file(ids[0]).exists());
    }

    #[tokio::test]
    async fn a_read_error_is_retried() {
        let f = Fixture::new();
        let ids = preloaded(&f, &["a"]).await;
        // a directory where the report file should be cannot be read
        files::delete(&f.report_file(ids[0])).await.unwrap();
        let dir = f.layout.reports().subdir(ids[0].to_string());
        dirs::create(&dir).await.unwrap();
        let mut u = f.uploader().await;

        run_next_attempt(&mut u).await;

        assert!(f.sent().is_empty());
        assert_backoff(&mut u, 10).await;
    }

    // ============================ FINISH ATTEMPT ============================ //
    #[tokio::test]
    async fn a_sent_report_is_dequeued_and_its_file_deleted() {
        let f = Fixture::new();
        let mut u = f.uploader().await;
        let id = u.enqueue(report("a")).await.unwrap();

        run_next_attempt(&mut u).await;

        assert!(f.queued_on_disk().await.is_empty());
        assert!(!f.report_file(id).exists());
    }

    /// A report file that can't be deleted is left for `prune_orphans` on the
    /// next boot; the report itself is still dequeued.
    #[tokio::test]
    async fn a_report_file_that_cannot_be_deleted_still_dequeues_the_report() {
        let f = Fixture::new();
        let ids = preloaded(&f, &["a"]).await;
        let mut u = f.uploader().await;
        let entry = QueueEntry {
            id: ids[0],
            job: report("a").metadata,
            attempts: 0,
            next_attempt_at: None,
        };
        // a directory where the report file should be can't be deleted as a file
        files::delete(&f.report_file(ids[0])).await.unwrap();
        dirs::create(&f.layout.reports().subdir(ids[0].to_string()))
            .await
            .unwrap();

        u.finish_attempt(entry, Ok(())).await;

        assert!(f.queued_on_disk().await.is_empty());
    }

    #[tokio::test]
    async fn retryable_errors_back_off_within_bounds() {
        let f = Fixture::new();
        for _ in 0..11 {
            f.sender.push(Step::Server);
        }
        let mut u = f.uploader().await;
        u.enqueue(report("a")).await.unwrap();

        for expected in [10, 20, 40, 80, 160, 320, 640, 1280, 2560, 3600, 3600] {
            run_next_attempt(&mut u).await;
            assert_backoff(&mut u, expected).await;
            f.advance_secs(expected);
        }
    }

    #[tokio::test]
    async fn network_errors_wait_a_flat_base() {
        let f = Fixture::new();
        for _ in 0..3 {
            f.sender.push(Step::Network);
        }
        let mut u = f.uploader().await;
        u.enqueue(report("a")).await.unwrap();

        for _ in 0..3 {
            run_next_attempt(&mut u).await;
            assert_backoff(&mut u, 10).await;
            f.advance_secs(10);
        }
    }

    #[tokio::test]
    async fn network_errors_do_not_use_up_attempts() {
        let f = Fixture::new();
        for _ in 0..3 {
            f.sender.push(Step::Network);
        }
        let options = UploaderOptions {
            retry: RetryPolicy {
                attempts: 1,
                ..RetryPolicy::default()
            },
            ..UploaderOptions::default()
        };
        let mut u = f.uploader_with(options).await;
        u.enqueue(report("a")).await.unwrap();

        for _ in 0..3 {
            run_next_attempt(&mut u).await;
            f.advance_secs(10);
        }
        run_next_attempt(&mut u).await;

        assert_eq!(f.sent(), strings(&["a"; 4]));
    }

    #[tokio::test]
    async fn each_report_backs_off_on_its_own() {
        let f = Fixture::new();
        f.sender.push(Step::Server);
        f.sender.push(Step::Ok);
        f.sender.push(Step::Server);
        preloaded(&f, &["a", "b"]).await;
        let mut u = f.uploader().await;

        // `a` fails and waits at the tail while `b` is sent
        run_next_attempt(&mut u).await;
        run_next_attempt(&mut u).await;
        assert_backoff(&mut u, 10).await;
        f.advance_secs(10);
        run_next_attempt(&mut u).await;
        assert_backoff(&mut u, 20).await;
        f.advance_secs(20);
        run_next_attempt(&mut u).await;

        assert_eq!(f.sent(), strings(&["a", "b", "a", "a"]));
    }

    #[tokio::test]
    async fn terminal_rejection_is_dropped_not_retried() {
        let f = Fixture::new();
        f.sender.push(Step::Terminal);
        let mut u = f.uploader().await;
        let id = u.enqueue(report("a")).await.unwrap();

        run_next_attempt(&mut u).await;

        assert_empty(&mut u).await;
        assert!(!f.report_file(id).exists());
    }

    #[tokio::test]
    async fn a_report_out_of_attempts_is_dropped() {
        let f = Fixture::new();
        for _ in 0..3 {
            f.sender.push(Step::Server);
        }
        let options = UploaderOptions {
            retry: RetryPolicy {
                attempts: 3,
                ..RetryPolicy::default()
            },
            ..UploaderOptions::default()
        };
        let mut u = f.uploader_with(options).await;
        u.enqueue(report("a")).await.unwrap();

        for wait in [10, 20] {
            run_next_attempt(&mut u).await;
            f.advance_secs(wait);
        }
        run_next_attempt(&mut u).await;

        assert_eq!(f.sent(), strings(&["a"; 3]));
        assert_empty(&mut u).await;
    }

    #[tokio::test]
    async fn a_report_unsent_past_the_age_backstop_is_dropped() {
        let f = Fixture::new();
        f.sender.push(Step::Network);
        f.advance(TimeDelta::days(7));
        let mut u = f.uploader().await;
        u.enqueue(report("a")).await.unwrap();

        run_next_attempt(&mut u).await;

        assert_empty(&mut u).await;
    }

    #[tokio::test]
    async fn a_failing_report_past_the_age_cap_is_dropped() {
        let f = Fixture::new();
        f.sender.push(Step::Server);
        f.advance(TimeDelta::days(7));
        let mut u = f.uploader().await;
        u.enqueue(report("a")).await.unwrap();

        run_next_attempt(&mut u).await;

        assert_empty(&mut u).await;
    }
}
