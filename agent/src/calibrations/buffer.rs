// standard crates
use std::collections::{HashMap, HashSet};

// internal crates
use crate::calibrations::{
    errors::{
        BufferFullErr, CalibrationsErr, DuplicateReportErr, InvalidReportIdErr, SerializeReportErr,
    },
    job::{CalibrationJob, Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile},
};
use crate::disk::layout::CalibrationsLayout;
use crate::filesys::{dirs, files, Dir, File, Overwrite, PathExt, WriteOptions};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

/// The report files' extension. Anything else in the content directory (an
/// in-flight atomic write, a set-aside `.corrupt` file) is left alone.
const REPORT_EXT: &str = ".json";

/// Appended to a report file that cannot be parsed while rebuilding the queue,
/// so it is kept for inspection but never read or cleaned up again.
const CORRUPT_EXT: &str = ".corrupt";

const MAX_REPORT_ID_LEN: usize = 128;

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

/// The on-disk shape of `content/<report_id>.json`. The job is stored beside
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
    /// Report-file size per buffered report id.
    sizes: HashMap<String, u64>,
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

    /// Read a buffered report's file.
    pub async fn read(&self, report_id: &str) -> Result<ReportFile, CalibrationsErr> {
        let file = self.report_file(report_id)?;
        Ok(files::read_json::<ReportFile>(&file).await?)
    }

    // ================ Mutators ================ //

    /// Buffer `report` durably. On success the report file and its queue entry
    /// are both on disk. On any error nothing is buffered, and a full buffer
    /// returns [`BufferFullErr`] rather than dropping an older report.
    pub async fn enqueue(&mut self, report: Report) -> Result<(), CalibrationsErr> {
        let report_id = report.job.report_id.clone();
        let file = self.report_file(&report_id)?;
        if self.sizes.contains_key(&report_id) {
            return Err(CalibrationsErr::DuplicateReportErr(DuplicateReportErr {
                report_id,
                trace: trace!(),
            }));
        }
        let bytes = serialize_report(&report)?;
        let report_bytes = bytes.len() as u64;
        self.verify_capacity(&report_id, report_bytes)?;

        // content first: a crash before the queue entry lands leaves only an
        // orphan file, which the next open removes
        files::write_bytes(&file, &bytes, WriteOptions::OVERWRITE_ATOMIC).await?;
        if let Err(err) = self.queue.enqueue_durable(report.job).await {
            delete_report_file(&file).await;
            return Err(err.into());
        }
        self.track(report_id, report_bytes);
        Ok(())
    }

    /// Drop the entry with `id` and its report file, once the report has been
    /// resolved. The queue entry goes first, so a crash between the two leaves
    /// an orphan file rather than an entry without content. Unknown ids are
    /// ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry> {
        let entry = self.queue.remove(id).await?;
        let report_id = &entry.job.report_id;
        if let Ok(file) = self.report_file(report_id) {
            delete_report_file(&file).await;
        }
        self.untrack(report_id);
        Some(entry)
    }

    /// Move `entry` to the tail with its updated attempt count and deadline.
    pub async fn requeue(&mut self, entry: QueueEntry) {
        self.queue.requeue(entry).await;
    }

    // ================ Internals ================ //

    fn report_file(&self, report_id: &str) -> Result<File, CalibrationsErr> {
        validate_report_id(report_id)?;
        Ok(self.content_dir.file(&format!("{report_id}{REPORT_EXT}")))
    }

    fn verify_capacity(&self, report_id: &str, report_bytes: u64) -> Result<(), CalibrationsErr> {
        let reports = self.queue.len();
        let over_bytes = self.used_bytes.saturating_add(report_bytes) > self.max_bytes;
        if reports < self.max_reports && !over_bytes {
            return Ok(());
        }
        let err = BufferFullErr {
            report_id: report_id.to_string(),
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

    fn track(&mut self, report_id: String, bytes: u64) {
        if let Some(old) = self.sizes.insert(report_id, bytes) {
            self.used_bytes = self.used_bytes.saturating_sub(old);
        }
        self.used_bytes = self.used_bytes.saturating_add(bytes);
    }

    fn untrack(&mut self, report_id: &str) {
        if let Some(bytes) = self.sizes.remove(report_id) {
            self.used_bytes = self.used_bytes.saturating_sub(bytes);
        }
    }

    /// Make the queue and the content directory agree: size every report file
    /// that has a queue entry, delete the ones that do not, and drop queue
    /// entries whose file is gone. Problems are logged, never fatal: a file
    /// that cannot be cleaned up now is retried on the next open.
    async fn reconcile(&mut self) {
        let queued: HashSet<String> = self
            .queue
            .iter()
            .map(|entry| entry.job.report_id.clone())
            .collect();
        for (report_id, file) in report_files(&self.content_dir).await {
            if !queued.contains(&report_id) {
                warn!("calibration: removing report file {file} with no queue entry");
                delete_report_file(&file).await;
                continue;
            }
            match files::size(&file).await {
                Ok(bytes) => self.track(report_id, bytes),
                Err(err) => error!("calibration: failed to size report file {file}: {err}"),
            }
        }

        let missing: Vec<QueueEntry> = self
            .queue
            .iter()
            .filter(|entry| !self.sizes.contains_key(&entry.job.report_id))
            .cloned()
            .collect();
        for entry in missing {
            error!(
                "calibration: dropping report {} whose report file is missing",
                entry.job.report_id
            );
            self.queue.remove(entry.id).await;
        }
    }
}

/// Report ids name files, so they are restricted to characters that are safe
/// in a file name on every platform.
fn validate_report_id(report_id: &str) -> Result<(), CalibrationsErr> {
    let valid = !report_id.is_empty()
        && report_id.len() <= MAX_REPORT_ID_LEN
        && report_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        return Ok(());
    }
    Err(CalibrationsErr::InvalidReportIdErr(InvalidReportIdErr {
        report_id: report_id.to_string(),
        trace: trace!(),
    }))
}

fn serialize_report(report: &Report) -> Result<Vec<u8>, CalibrationsErr> {
    let record = ReportFileRef {
        job: &report.job,
        content: &report.content,
    };
    serde_json::to_vec(&record).map_err(|source| {
        CalibrationsErr::SerializeReportErr(SerializeReportErr {
            report_id: report.job.report_id.clone(),
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

/// The report files in `content_dir` with their report ids. A listing error is
/// logged and treated as an empty directory.
async fn report_files(content_dir: &Dir) -> Vec<(String, File)> {
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
            let report_id = name.strip_suffix(REPORT_EXT)?;
            validate_report_id(report_id).ok()?;
            Some((report_id.to_string(), file))
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
    let aside = file
        .parent()?
        .file(&format!("{}{CORRUPT_EXT}", file.name()?));
    error!("calibration: queue snapshot {file} is unreadable; moving it to {aside} and rebuilding from the report files: {err}");
    files::move_to(file, &aside, Overwrite::Allow).await?;
    let snapshot = rebuild_snapshot(content_dir).await;
    Ok(QueueSnapshotFile::create(file.clone(), &snapshot, Overwrite::Allow).await?)
}

/// A queue holding every readable report file, oldest calibration first.
/// Attempt counts and backoff are lost, so every report is due at once. An
/// unreadable report file is renamed with [`CORRUPT_EXT`] and skipped.
async fn rebuild_snapshot(content_dir: &Dir) -> QueueSnapshot {
    let mut jobs = Vec::new();
    for (report_id, file) in report_files(content_dir).await {
        match files::read_json::<ReportFile>(&file).await {
            Ok(record) if record.job.report_id == report_id => jobs.push(record.job),
            Ok(record) => {
                error!(
                    "calibration: report file {file} holds report {}; setting it aside",
                    record.job.report_id
                );
                set_aside(&file).await;
            }
            Err(err) => {
                error!("calibration: report file {file} is unreadable; setting it aside: {err}");
                set_aside(&file).await;
            }
        }
    }
    jobs.sort_by(|a, b| (a.calibrated_at, &a.report_id).cmp(&(b.calibrated_at, &b.report_id)));
    QueueSnapshot {
        entries: jobs
            .into_iter()
            .map(|job| QueueEntry {
                id: Uuid::new_v4(),
                job,
                attempts: 0,
                next_attempt_at: None,
            })
            .collect(),
    }
}

async fn set_aside(file: &File) {
    let aside = match (file.parent(), file.name()) {
        (Ok(dir), Ok(name)) => dir.file(&format!("{name}{CORRUPT_EXT}")),
        _ => return,
    };
    if let Err(err) = files::move_to(file, &aside, Overwrite::Allow).await {
        warn!("calibration: failed to set aside report file {file}: {err}");
    }
}
