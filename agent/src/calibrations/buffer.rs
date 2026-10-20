// standard crates
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

// internal crates
use crate::calibrations::{
    errors::{BufferFullErr, CalibrationsErr, SerializeReportErr},
    job::{CalibrationJob, Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile},
};
use crate::data_uploads::queue::QueueJob;
use crate::disk::layout::CalibrationsLayout;
use crate::filesys::{dirs, files, Dir, File, Overwrite, PathExt, WriteOptions};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

/// The report files' extension. A report file is named `<entry id>.json`, the
/// queue entry's UUID. Anything else in the content directory (an in-flight
/// atomic write, a set-aside `.corrupt` file) is left alone.
const REPORT_EXT: &str = ".json";

/// Appended to a report file that cannot be parsed while rebuilding the queue,
/// so it is kept for inspection but never read or cleaned up again.
const CORRUPT_EXT: &str = ".corrupt";

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BufferOptions {
    /// The most reports the buffer holds at once.
    pub max_reports: usize,
    /// The most report-file bytes the buffer holds at once.
    pub max_bytes: u64,
}

impl Default for BufferOptions {
    fn default() -> Self {
        Self {
            max_reports: 1000,
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// A calibration as intake hands it to the buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub job: CalibrationJob,
    pub content: serde_json::Value,
}

/// The on-disk shape of `content/<entry id>.json`. The job is stored beside
/// the content so the queue can be rebuilt from the report files alone if
/// `queue.json` is ever lost to corruption.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportFile {
    pub job: CalibrationJob,
    pub content: serde_json::Value,
}

#[derive(Serialize)]
struct ReportFileRef<'a> {
    job: &'a CalibrationJob,
    content: &'a serde_json::Value,
}

/// The durable calibration buffer: a FIFO of accepted reports that survives
/// agent restarts until each one is uploaded.
///
/// The queue (`queue.json`) holds each report's metadata, attempt count and
/// backoff, and each report's content lives in its own file under `content/`,
/// so a queue write never rewrites megabytes of content. A report is accepted
/// only once both are on disk: the report file is written first and the queue
/// entry second, so a crash between the two leaves an orphan file that the next
/// [`Buffer::open`] removes, never a queue entry without content.
///
/// The buffer is single-owner (`&mut self`); callers that share it wrap it.
pub struct Buffer {
    queue: Queue,
    content_dir: Dir,
    max_reports: usize,
    max_bytes: u64,
    /// Report-file size per queue entry id.
    sizes: HashMap<Uuid, u64>,
    used_bytes: u64,
}

impl Buffer {
    /// Load the buffer from `layout`, creating it when absent. An unreadable
    /// `queue.json` is set aside as `queue.json.corrupt` and the queue rebuilt
    /// from the report files. Report files with no queue entry are deleted,
    /// and queue entries with no report file are dropped.
    pub async fn open(
        layout: &CalibrationsLayout,
        options: BufferOptions,
    ) -> Result<Self, CalibrationsErr> {
        let content_dir = layout.content();
        dirs::create_if_absent(&content_dir).await?;
        let snapshot_file = load_snapshot(&layout.queue(), &content_dir).await?;
        let mut buffer = Self {
            queue: Queue::from_snapshot(options.max_reports, snapshot_file),
            content_dir,
            max_reports: options.max_reports,
            max_bytes: options.max_bytes,
            sizes: HashMap::new(),
            used_bytes: 0,
        };
        buffer.reconcile().await;
        info!(
            "calibration buffer loaded: {} report(s), {} byte(s)",
            buffer.len(),
            buffer.used_bytes
        );
        Ok(buffer)
    }

    // ================ Accessors ================ //

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Bytes held by the buffered report files.
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    /// The buffered reports' metadata, oldest first.
    pub fn jobs(&self) -> Vec<CalibrationJob> {
        self.queue.iter().map(|entry| entry.job.clone()).collect()
    }

    /// The oldest report that is due at `now`. The entry stays buffered until
    /// [`Self::remove`]d.
    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry> {
        self.queue.next_ready(now)
    }

    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.queue.earliest_next_attempt()
    }

    /// Read the report file of the queue entry `id`.
    pub async fn read(&self, id: Uuid) -> Result<ReportFile, CalibrationsErr> {
        Ok(files::read_json::<ReportFile>(&self.report_file(id)).await?)
    }

    // ================ Mutators ================ //

    /// Buffer `report` durably. On success the report file and its queue entry
    /// are both on disk. On any error nothing is buffered, and a full buffer
    /// returns [`BufferFullErr`] rather than dropping an older report.
    /// Returns the new queue entry's id, which also names its report file.
    pub async fn enqueue(&mut self, report: Report) -> Result<Uuid, CalibrationsErr> {
        let id = Uuid::new_v4();
        let file = self.report_file(id);
        let bytes = serialize_report(id, &report)?;
        let report_bytes = bytes.len() as u64;
        self.verify_capacity(report_bytes)?;

        // content first: a crash before the queue entry lands leaves only an
        // orphan file, which the next open removes
        files::write_bytes(&file, &bytes, WriteOptions::OVERWRITE_ATOMIC).await?;
        if let Err(err) = self.queue.enqueue_durable(id, report.job).await {
            delete_report_file(&file).await;
            return Err(err.into());
        }
        self.track(id, report_bytes);
        Ok(id)
    }

    /// Drop the entry with `id` and its report file, once the report has been
    /// resolved. The queue entry goes first, so a crash between the two leaves
    /// an orphan file rather than an entry without content. Unknown ids are
    /// ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry> {
        let entry = self.queue.remove(id).await?;
        delete_report_file(&self.report_file(id)).await;
        self.untrack(id);
        Some(entry)
    }

    /// Move `entry` to the tail with its updated attempt count and deadline.
    pub async fn requeue(&mut self, entry: QueueEntry) {
        self.queue.requeue(entry).await;
    }

    // ================ Internals ================ //

    fn report_file(&self, id: Uuid) -> File {
        report_file(&self.content_dir, id)
    }

    fn verify_capacity(&self, report_bytes: u64) -> Result<(), CalibrationsErr> {
        let reports = self.queue.len();
        let over_bytes = self.used_bytes.saturating_add(report_bytes) > self.max_bytes;
        if reports < self.max_reports && !over_bytes {
            return Ok(());
        }
        let err = BufferFullErr {
            report_bytes,
            reports,
            max_reports: self.max_reports,
            bytes: self.used_bytes,
            max_bytes: self.max_bytes,
            trace: trace!(),
        };
        warn!("{err}");
        Err(CalibrationsErr::BufferFullErr(err))
    }

    /// Count a report file. Entry ids are unique, so each is tracked once.
    fn track(&mut self, id: Uuid, bytes: u64) {
        self.sizes.insert(id, bytes);
        self.used_bytes = self.used_bytes.saturating_add(bytes);
    }

    fn untrack(&mut self, id: Uuid) {
        if let Some(bytes) = self.sizes.remove(&id) {
            self.used_bytes = self.used_bytes.saturating_sub(bytes);
        }
    }

    /// Make the queue and the content directory agree: size every report file
    /// that has a queue entry, delete the ones that do not, and drop queue
    /// entries whose file is gone. Problems are logged, never fatal: a file
    /// that cannot be cleaned up now is retried on the next open.
    async fn reconcile(&mut self) {
        let queued: HashSet<Uuid> = self.queue.iter().map(|entry| entry.id).collect();
        for (id, file) in report_files(&self.content_dir).await {
            if !queued.contains(&id) {
                warn!("calibration: removing report file {file} with no queue entry");
                delete_report_file(&file).await;
                continue;
            }
            match files::size(&file).await {
                Ok(bytes) => self.track(id, bytes),
                Err(err) => error!("calibration: failed to size report file {file}: {err}"),
            }
        }

        let missing: Vec<QueueEntry> = self
            .queue
            .iter()
            .filter(|entry| !self.sizes.contains_key(&entry.id))
            .cloned()
            .collect();
        for entry in missing {
            error!(
                "calibration: dropping {} whose report file is missing",
                QueueJob::name(&entry.job)
            );
            self.queue.remove(entry.id).await;
        }
    }
}

fn report_file(content_dir: &Dir, id: Uuid) -> File {
    content_dir.file(&format!("{}{REPORT_EXT}", id.hyphenated()))
}

fn serialize_report(id: Uuid, report: &Report) -> Result<Vec<u8>, CalibrationsErr> {
    let record = ReportFileRef {
        job: &report.job,
        content: &report.content,
    };
    serde_json::to_vec(&record).map_err(|source| {
        CalibrationsErr::SerializeReportErr(SerializeReportErr {
            id,
            source,
            trace: trace!(),
        })
    })
}

async fn delete_report_file(file: &File) {
    if let Err(err) = files::delete(file).await {
        warn!("calibration: failed to delete report file {file}: {err}");
    }
}

/// The report files in `content_dir` with their queue entry ids. Only names in
/// the exact form [`report_file`] writes count. A listing error is logged and
/// treated as an empty directory.
async fn report_files(content_dir: &Dir) -> Vec<(Uuid, File)> {
    let listing = match dirs::files(content_dir).await {
        Ok(listing) => listing,
        Err(err) => {
            error!("calibration: failed to list report files in {content_dir}: {err}");
            return Vec::new();
        }
    };
    listing
        .into_iter()
        .filter_map(|file| {
            let name = file.name().ok()?;
            let stem = name.strip_suffix(REPORT_EXT)?;
            let id = Uuid::from_str(stem).ok()?;
            (id.hyphenated().to_string() == stem).then_some((id, file))
        })
        .collect()
}

/// Open `queue.json`, creating it when absent. An existing snapshot that
/// cannot be read is never silently replaced: it is set aside and the queue
/// is rebuilt from the report files.
async fn load_snapshot(
    file: &File,
    content_dir: &Dir,
) -> Result<QueueSnapshotFile, CalibrationsErr> {
    if !file.exists() {
        let empty = QueueSnapshot::default();
        return Ok(QueueSnapshotFile::create(file.clone(), &empty, Overwrite::Allow).await?);
    }
    let err = match QueueSnapshotFile::new(file.clone()).await {
        Ok(snapshot_file) => return Ok(snapshot_file),
        Err(err) => err,
    };
    let aside = corrupt_path(file);
    error!("calibration: queue snapshot {file} is unreadable; moving it to {aside} and rebuilding from the report files: {err}");
    files::move_to(file, &aside, Overwrite::Allow).await?;
    let snapshot = rebuild_snapshot(content_dir).await;
    Ok(QueueSnapshotFile::create(file.clone(), &snapshot, Overwrite::Allow).await?)
}

/// A queue holding every readable report file under its own entry id, oldest
/// calibration first. Attempt counts and backoff are lost, so every report is
/// due at once. An unreadable report file is renamed with [`CORRUPT_EXT`] and
/// skipped.
async fn rebuild_snapshot(content_dir: &Dir) -> QueueSnapshot {
    let mut entries = Vec::new();
    for (id, file) in report_files(content_dir).await {
        match files::read_json::<ReportFile>(&file).await {
            Ok(record) => entries.push(QueueEntry {
                id,
                job: record.job,
                attempts: 0,
                next_attempt_at: None,
            }),
            Err(err) => {
                error!("calibration: report file {file} is unreadable; setting it aside: {err}");
                set_aside(&file).await;
            }
        }
    }
    entries.sort_by_key(|entry| (entry.job.calibrated_at, entry.id));
    QueueSnapshot { entries }
}

/// `file` with [`CORRUPT_EXT`] appended to its name.
fn corrupt_path(file: &File) -> File {
    let mut path = file.path().clone().into_os_string();
    path.push(CORRUPT_EXT);
    File::new(path)
}

async fn set_aside(file: &File) {
    if let Err(err) = files::move_to(file, &corrupt_path(file), Overwrite::Allow).await {
        warn!("calibration: failed to set aside report file {file}: {err}");
    }
}
