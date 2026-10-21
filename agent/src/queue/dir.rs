// standard crates
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;

// internal crates
use crate::filesys::{
    dirs, errors::PathDoesNotExistErr, files, Dir, File, FileSysErr, Overwrite, PathExt,
    WriteOptions,
};
use crate::queue::{
    errors::{DirQueueErr, ItemTooLargeErr, SerializeItemErr},
    item::Item,
    job::QueueJob,
    FileQueue, Persistence, QueueEntry, QueueSnapshot, QueueSnapshotFile,
};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

/// The item files' extension. An item file is named `<entry id>.json`, the
/// queue entry's UUID. Anything else in the content directory (an in-flight
/// atomic write, a set-aside `.corrupt` file) is left alone.
const ITEM_EXT: &str = ".json";

/// Appended to a file that cannot be parsed, so it is kept for inspection but
/// never read or cleaned up again.
const CORRUPT_EXT: &str = ".corrupt";

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DirQueueOptions {
    /// The most items the queue holds at once.
    pub capacity: usize,
    /// The largest item file the queue accepts, in bytes.
    pub max_item_bytes: u64,
}

/// A FIFO of accepted items that survives agent restarts until each one is
/// [`DirQueue::remove`]d.
///
/// The queue (`queue_file`) holds each item's job, attempt count and backoff,
/// and each item's content lives in its own file under `content_dir`, so a
/// queue write never rewrites large content. An item is accepted only once
/// both are on disk: the queue entry is written first (a durable [`FileQueue`])
/// and the item file second, so a crash between the two leaves an entry
/// without content, which the next [`DirQueue::open`] drops. Unlike a
/// cache, the queue never evicts: when full it rejects new items. Disk use is
/// bounded by `capacity` times `max_item_bytes`.
///
/// [`DirQueue::in_memory`] is the fallback for when the queue can't be opened
/// on disk: the same queue, with nothing persisted, so it is empty after a
/// restart.
///
/// The queue is single-owner (`&mut self`); callers that share it wrap it.
pub struct DirQueue<J: QueueJob, C> {
    queue: FileQueue<J>,
    content: Content<J, C>,
    max_item_bytes: u64,
}

/// Where the items' content lives.
enum Content<J, C> {
    /// One item file per queue entry.
    Dir(Dir),
    /// In memory only, lost on restart.
    Memory(HashMap<Uuid, Item<J, C>>),
}

impl<J, C> DirQueue<J, C>
where
    J: QueueJob,
    C: Clone + Serialize + DeserializeOwned,
{
    /// Load the queue, creating it when absent. A missing `queue_file` is
    /// rebuilt from the item files, and an unparseable one is set aside with
    /// [`CORRUPT_EXT`] and rebuilt the same way. Item files with no queue
    /// entry are deleted, and queue entries with no item file are dropped.
    pub async fn open(
        queue_file: File,
        content_dir: Dir,
        options: DirQueueOptions,
    ) -> Result<Self, DirQueueErr> {
        dirs::create_if_absent(&content_dir).await?;
        let snapshot_file = load_snapshot::<J, C>(&queue_file, &content_dir).await?;
        let mut queue = Self {
            queue: FileQueue::from_snapshot(options.capacity, snapshot_file, Persistence::Durable),
            content: Content::Dir(content_dir.clone()),
            max_item_bytes: options.max_item_bytes,
        };
        queue.reconcile(&content_dir).await?;
        info!("{} queue loaded: {} item(s)", J::LABEL, queue.len());
        Ok(queue)
    }

    /// An empty queue that keeps everything in memory, with the same caps.
    pub fn in_memory(options: DirQueueOptions) -> Self {
        Self {
            queue: FileQueue::new(options.capacity),
            content: Content::Memory(HashMap::new()),
            max_item_bytes: options.max_item_bytes,
        }
    }

    // ================ Accessors ================ //

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The queued items' entries, oldest first.
    pub fn entries(&self) -> Vec<QueueEntry<J>> {
        self.queue.iter().cloned().collect()
    }

    /// The oldest item that is due at `now`. The entry stays queued until
    /// [`Self::remove`]d.
    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry<J>> {
        self.queue.next_ready(now)
    }

    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.queue.earliest_next_attempt()
    }

    /// Read the item of the queue entry `id`. An unknown id is
    /// [`FileSysErr::PathDoesNotExistErr`], in memory as on disk.
    pub async fn read(&self, id: Uuid) -> Result<Item<J, C>, DirQueueErr> {
        match &self.content {
            Content::Dir(dir) => Ok(files::read_json(&item_file(dir, id)).await?),
            Content::Memory(items) => items.get(&id).cloned().ok_or_else(|| not_found(id)),
        }
    }

    // ================ Mutators ================ //

    /// Queue `item` durably. On success its queue entry and item file are
    /// both on disk (or both in memory, for [`Self::in_memory`]). On any
    /// error nothing is queued: a full queue returns [`QueueFullErr`] rather
    /// than dropping an older item, an item file over `max_item_bytes`
    /// returns [`ItemTooLargeErr`], and a failed item file write removes the
    /// entry again. Returns the new queue entry's id, which also names its
    /// item file.
    pub async fn enqueue(&mut self, item: Item<J, C>) -> Result<Uuid, DirQueueErr> {
        let bytes = serialize_item(&item)?;
        self.verify_item_size(bytes.len() as u64)?;
        let id = self.queue.enqueue(item.job.clone()).await?;
        match &mut self.content {
            Content::Dir(dir) => {
                let written =
                    files::write_bytes(&item_file(dir, id), &bytes, WriteOptions::OVERWRITE_ATOMIC)
                        .await;
                if let Err(err) = written {
                    self.queue.remove(id).await;
                    return Err(err.into());
                }
            }
            Content::Memory(items) => drop(items.insert(id, item)),
        }
        Ok(id)
    }

    /// Drop the entry with `id` and its item file, once the item has been
    /// resolved. The queue entry goes first, so a crash between the two leaves
    /// an orphan file rather than an entry without content. Unknown ids are
    /// ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry<J>> {
        let entry = self.queue.remove(id).await?;
        match &mut self.content {
            Content::Dir(dir) => delete_item_file::<J>(&item_file(dir, id)).await,
            Content::Memory(items) => drop(items.remove(&id)),
        }
        Some(entry)
    }

    /// Move the item file of the entry `id` into `dest_dir` (as
    /// `<entry id>.json`) and drop its queue entry, for an item that can never
    /// be resolved but should be kept for inspection. The file moves first, so
    /// a crash between the two steps leaves an entry without a file, which the
    /// next [`Self::open`] drops, while the parked copy survives; removing the
    /// entry first would leave an orphan that `open` deletes. `open` never
    /// looks inside `dest_dir`. An in-memory item is written there instead.
    /// Unknown ids return `Ok(None)` and touch nothing. On an error the entry
    /// and its item are unchanged.
    pub async fn park(
        &mut self,
        id: Uuid,
        dest_dir: &Dir,
    ) -> Result<Option<QueueEntry<J>>, DirQueueErr> {
        if !self.is_queued(id) {
            return Ok(None);
        }
        dirs::create_if_absent(dest_dir).await?;
        let dest = item_file(dest_dir, id);
        match &mut self.content {
            Content::Dir(dir) => {
                files::move_to(&item_file(dir, id), &dest, Overwrite::Allow).await?
            }
            Content::Memory(items) => {
                if let Some(item) = items.get(&id) {
                    let bytes = serialize_item(item)?;
                    files::write_bytes(&dest, &bytes, WriteOptions::OVERWRITE_ATOMIC).await?;
                    items.remove(&id);
                }
            }
        }
        Ok(self.queue.remove(id).await)
    }

    /// Move `entry` to the tail with its updated attempt count and deadline.
    /// An entry that is no longer queued (already removed, or stale) is
    /// ignored, so it can't come back without its item file.
    pub async fn requeue(&mut self, entry: QueueEntry<J>) {
        if !self.is_queued(entry.id) {
            warn!(
                "{}: ignoring requeue of {}, which is not queued",
                J::LABEL,
                entry.job.name()
            );
            return;
        }
        self.queue.requeue(entry).await;
    }

    // ================ Internals ================ //

    fn is_queued(&self, id: Uuid) -> bool {
        self.queue.iter().any(|entry| entry.id == id)
    }

    fn verify_item_size(&self, item_bytes: u64) -> Result<(), DirQueueErr> {
        if item_bytes <= self.max_item_bytes {
            return Ok(());
        }
        let err = ItemTooLargeErr {
            label: J::LABEL,
            item_bytes,
            max_item_bytes: self.max_item_bytes,
            trace: trace!(),
        };
        warn!("{err}");
        Err(DirQueueErr::ItemTooLargeErr(err))
    }

    /// Make the queue and the content directory agree: delete item files that
    /// have no queue entry, and drop queue entries whose file is gone. Files
    /// already queued are kept even if they exceed a lowered size cap. A
    /// failed listing fails the open, since acting on it would drop every
    /// entry. Problems with single files are logged, never fatal: a file that
    /// cannot be cleaned up now is retried on the next open.
    async fn reconcile(&mut self, content_dir: &Dir) -> Result<(), DirQueueErr> {
        let queued: HashSet<Uuid> = self.queue.iter().map(|entry| entry.id).collect();
        let mut found = HashSet::new();
        for (id, file) in item_files(content_dir).await? {
            if !queued.contains(&id) {
                warn!(
                    "{}: removing item file {file} with no queue entry",
                    J::LABEL
                );
                delete_item_file::<J>(&file).await;
                continue;
            }
            found.insert(id);
        }

        let missing: Vec<QueueEntry<J>> = self
            .queue
            .iter()
            .filter(|entry| !found.contains(&entry.id))
            .cloned()
            .collect();
        for entry in missing {
            error!(
                "{}: dropping {} whose item file is missing",
                J::LABEL,
                entry.job.name()
            );
            self.queue.remove(entry.id).await;
        }
        Ok(())
    }
}

fn item_file(content_dir: &Dir, id: Uuid) -> File {
    content_dir.file(&item_name(id))
}

fn item_name(id: Uuid) -> String {
    format!("{}{ITEM_EXT}", id.hyphenated())
}

/// The error for an id with no item, matching a missing item file.
fn not_found(id: Uuid) -> DirQueueErr {
    DirQueueErr::FileSysErr(FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
        path: PathBuf::from(item_name(id)),
        trace: trace!(),
    }))
}

fn serialize_item<J: QueueJob, C: Serialize>(item: &Item<J, C>) -> Result<Vec<u8>, DirQueueErr> {
    serde_json::to_vec(item).map_err(|source| {
        DirQueueErr::SerializeItemErr(SerializeItemErr {
            label: J::LABEL,
            name: item.job.name(),
            source,
            trace: trace!(),
        })
    })
}

async fn delete_item_file<J: QueueJob>(file: &File) {
    if let Err(err) = files::delete(file).await {
        warn!("{}: failed to delete item file {file}: {err}", J::LABEL);
    }
}

/// The item files in `content_dir` with their queue entry ids. Only names in
/// the exact form [`item_file`] writes count. A listing error is returned,
/// never treated as an empty directory.
async fn item_files(content_dir: &Dir) -> Result<Vec<(Uuid, File)>, DirQueueErr> {
    let listing = dirs::files(content_dir).await?;
    Ok(listing
        .into_iter()
        .filter_map(|file| {
            let name = file.name().ok()?;
            let stem = name.strip_suffix(ITEM_EXT)?;
            let id = Uuid::from_str(stem).ok()?;
            (id.hyphenated().to_string() == stem).then_some((id, file))
        })
        .collect())
}

/// Open the queue file (entry order, attempt counts and backoff). A missing
/// one is rebuilt from the item files (empty on first boot), so item files are
/// never deleted for want of a queue. An existing queue file that cannot be
/// parsed is never silently replaced: it is set aside and the queue is rebuilt
/// the same way. Any other read error fails the open and leaves the file in
/// place.
async fn load_snapshot<J, C>(
    queue_file: &File,
    content_dir: &Dir,
) -> Result<QueueSnapshotFile<J>, DirQueueErr>
where
    J: QueueJob,
    C: DeserializeOwned,
{
    if !queue_file.exists() {
        let snapshot = rebuild_snapshot::<J, C>(content_dir).await?;
        return Ok(
            QueueSnapshotFile::create(queue_file.clone(), &snapshot, Overwrite::Allow).await?,
        );
    }
    let err = match QueueSnapshotFile::<J>::new(queue_file.clone()).await {
        Ok(snapshot_file) => return Ok(snapshot_file),
        Err(FileSysErr::ParseJSONErr(err)) => err,
        // a read error may be transient, so keep the snapshot and its attempt
        // counts and order for the next open instead of rebuilding
        Err(err) => return Err(err.into()),
    };
    let aside = corrupt_path(queue_file);
    error!(
        "{}: queue file {queue_file} is unreadable; moving it to {aside} and rebuilding from the item files: {err}",
        J::LABEL
    );
    // rebuild before moving the snapshot aside: if the rebuild fails, the next
    // open must still find the unreadable snapshot, not an absent one it would
    // replace with an empty queue
    let snapshot = rebuild_snapshot::<J, C>(content_dir).await?;
    files::move_to(queue_file, &aside, Overwrite::Allow).await?;
    Ok(QueueSnapshotFile::create(queue_file.clone(), &snapshot, Overwrite::Allow).await?)
}

/// A queue holding every readable item file under its own entry id, in id
/// order, which is enqueue order since ids are UUIDv7. Attempt counts and backoff are lost, so
/// every item is due at once. An item file that cannot be parsed is renamed
/// with [`CORRUPT_EXT`] and skipped. A failed listing or read is returned,
/// never rebuilt as a smaller queue.
async fn rebuild_snapshot<J, C>(content_dir: &Dir) -> Result<QueueSnapshot<J>, DirQueueErr>
where
    J: QueueJob,
    C: DeserializeOwned,
{
    let mut entries = Vec::new();
    for (id, file) in item_files(content_dir).await? {
        match files::read_json::<Item<J, C>>(&file).await {
            Ok(item) => entries.push(QueueEntry {
                id,
                job: item.job,
                attempts: 0,
                next_attempt_at: None,
            }),
            Err(FileSysErr::ParseJSONErr(err)) => {
                error!(
                    "{}: item file {file} is unreadable; setting it aside: {err}",
                    J::LABEL
                );
                set_aside::<J>(&file).await;
            }
            // a read error may be transient; fail rather than set aside an
            // item that may be intact
            Err(err) => return Err(err.into()),
        }
    }
    entries.sort_by_key(|entry| entry.id);
    Ok(QueueSnapshot { entries })
}

/// `file` with [`CORRUPT_EXT`] appended to its name.
fn corrupt_path(file: &File) -> File {
    let mut path = file.path().clone().into_os_string();
    path.push(CORRUPT_EXT);
    File::new(path)
}

async fn set_aside<J: QueueJob>(file: &File) {
    if let Err(err) = files::move_to(file, &corrupt_path(file), Overwrite::Allow).await {
        warn!("{}: failed to set aside item file {file}: {err}", J::LABEL);
    }
}
