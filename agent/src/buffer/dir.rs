// standard crates
use std::collections::HashSet;
use std::marker::PhantomData;
use std::str::FromStr;

// internal crates
use crate::buffer::{
    errors::{BufferErr, BufferFullErr, ItemTooLargeErr, SerializeItemErr},
    job::{BufferJob, Item},
};
use crate::filesys::{dirs, files, Dir, File, FileSysErr, Overwrite, PathExt, WriteOptions};
use crate::queue::{Queue, QueueEntry, QueueJob, QueueSnapshot, QueueSnapshotFile};
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
pub struct BufferOptions {
    /// The most items the buffer holds at once.
    pub max_items: usize,
    /// The largest item file the buffer accepts, in bytes.
    pub max_item_bytes: u64,
}

/// A FIFO of accepted items that survives agent restarts until each one is
/// [`Buffer::remove`]d.
///
/// The queue (`queue_file`) holds each item's job, attempt count and backoff,
/// and each item's content lives in its own file under `content_dir`, so a
/// queue write never rewrites large content. An item is accepted only once
/// both are on disk: the item file is written first and the queue entry
/// second, so a crash between the two leaves an orphan file that the next
/// [`Buffer::open`] removes, never a queue entry without content. Unlike a
/// cache, the buffer never evicts: when full it rejects new items. Disk use is
/// bounded by `max_items` times `max_item_bytes`.
///
/// The buffer is single-owner (`&mut self`); callers that share it wrap it.
pub struct Buffer<J: BufferJob, C> {
    queue: Queue<J>,
    content_dir: Dir,
    max_items: usize,
    max_item_bytes: u64,
    _content: PhantomData<C>,
}

impl<J, C> Buffer<J, C>
where
    J: BufferJob,
    C: Serialize + DeserializeOwned,
{
    /// Load the buffer, creating it when absent. A missing `queue_file` is
    /// rebuilt from the item files, and an unparseable one is set aside with
    /// [`CORRUPT_EXT`] and rebuilt the same way. Item files with no queue
    /// entry are deleted, and queue entries with no item file are dropped.
    pub async fn open(
        queue_file: File,
        content_dir: Dir,
        options: BufferOptions,
    ) -> Result<Self, BufferErr> {
        dirs::create_if_absent(&content_dir).await?;
        let snapshot_file = load_snapshot::<J, C>(&queue_file, &content_dir).await?;
        let mut buffer = Self {
            queue: Queue::from_snapshot(options.max_items, snapshot_file),
            content_dir,
            max_items: options.max_items,
            max_item_bytes: options.max_item_bytes,
            _content: PhantomData,
        };
        buffer.reconcile().await?;
        info!("{} buffer loaded: {} item(s)", J::LABEL, buffer.len());
        Ok(buffer)
    }

    // ================ Accessors ================ //

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The buffered items' jobs, oldest first.
    pub fn jobs(&self) -> Vec<J> {
        self.queue.iter().map(|entry| entry.job.clone()).collect()
    }

    /// The oldest item that is due at `now`. The entry stays buffered until
    /// [`Self::remove`]d.
    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry<J>> {
        self.queue.next_ready(now)
    }

    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.queue.earliest_next_attempt()
    }

    /// Read the item file of the queue entry `id`.
    pub async fn read(&self, id: Uuid) -> Result<Item<J, C>, BufferErr> {
        Ok(files::read_json::<Item<J, C>>(&self.item_file(id)).await?)
    }

    // ================ Mutators ================ //

    /// Buffer `item` durably. On success the item file and its queue entry are
    /// both on disk. On any error nothing is buffered: a full buffer returns
    /// [`BufferFullErr`] rather than dropping an older item, and an item file
    /// over `max_item_bytes` returns [`ItemTooLargeErr`]. Returns
    /// the new queue entry's id, which also names its item file.
    pub async fn enqueue(&mut self, item: Item<J, C>) -> Result<Uuid, BufferErr> {
        let id = Uuid::new_v4();
        let file = self.item_file(id);
        self.verify_capacity()?;
        let bytes = serialize_item(id, &item)?;
        self.verify_item_size(bytes.len() as u64)?;

        // content first: a crash before the queue entry lands leaves only an
        // orphan file, which the next open removes
        files::write_bytes(&file, &bytes, WriteOptions::OVERWRITE_ATOMIC).await?;
        if let Err(err) = self.queue.enqueue_durable(id, item.job).await {
            delete_item_file::<J>(&file).await;
            return Err(err.into());
        }
        Ok(id)
    }

    /// Drop the entry with `id` and its item file, once the item has been
    /// resolved. The queue entry goes first, so a crash between the two leaves
    /// an orphan file rather than an entry without content. Unknown ids are
    /// ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry<J>> {
        let entry = self.queue.remove(id).await?;
        delete_item_file::<J>(&self.item_file(id)).await;
        Some(entry)
    }

    /// Move the item file of the entry `id` into `dest_dir` (as
    /// `<entry id>.json`) and drop its queue entry, for an item that can never
    /// be resolved but should be kept for inspection. The file moves first, so
    /// a crash between the two steps leaves an entry without a file, which the
    /// next [`Self::open`] drops, while the parked copy survives; removing the
    /// entry first would leave an orphan that `open` deletes. `open` never
    /// looks inside `dest_dir`. Unknown ids return `Ok(None)` and touch
    /// nothing. On an error the entry and its item file are unchanged.
    pub async fn park(
        &mut self,
        id: Uuid,
        dest_dir: &Dir,
    ) -> Result<Option<QueueEntry<J>>, BufferErr> {
        if !self.is_buffered(id) {
            return Ok(None);
        }
        dirs::create_if_absent(dest_dir).await?;
        files::move_to(
            &self.item_file(id),
            &item_file(dest_dir, id),
            Overwrite::Allow,
        )
        .await?;
        Ok(self.queue.remove(id).await)
    }

    /// Move `entry` to the tail with its updated attempt count and deadline.
    /// An entry that is no longer buffered (already removed, or stale) is
    /// ignored, so it can't come back without its item file.
    pub async fn requeue(&mut self, entry: QueueEntry<J>) {
        if !self.is_buffered(entry.id) {
            warn!(
                "{}: ignoring requeue of {}, which is not buffered",
                J::LABEL,
                entry.job.name()
            );
            return;
        }
        self.queue.requeue(entry).await;
    }

    // ================ Internals ================ //

    fn item_file(&self, id: Uuid) -> File {
        item_file(&self.content_dir, id)
    }

    fn is_buffered(&self, id: Uuid) -> bool {
        self.queue.iter().any(|entry| entry.id == id)
    }

    fn verify_capacity(&self) -> Result<(), BufferErr> {
        let items = self.queue.len();
        if items < self.max_items {
            return Ok(());
        }
        let err = BufferFullErr {
            label: J::LABEL,
            items,
            max_items: self.max_items,
            trace: trace!(),
        };
        warn!("{err}");
        Err(BufferErr::BufferFullErr(err))
    }

    fn verify_item_size(&self, item_bytes: u64) -> Result<(), BufferErr> {
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
        Err(BufferErr::ItemTooLargeErr(err))
    }

    /// Make the queue and the content directory agree: delete item files that
    /// have no queue entry, and drop queue entries whose file is gone. Files
    /// already buffered are kept even if they exceed a lowered size cap. A
    /// failed listing fails the open, since acting on it would drop every
    /// entry. Problems with single files are logged, never fatal: a file that
    /// cannot be cleaned up now is retried on the next open.
    async fn reconcile(&mut self) -> Result<(), BufferErr> {
        let queued: HashSet<Uuid> = self.queue.iter().map(|entry| entry.id).collect();
        let mut found = HashSet::new();
        for (id, file) in item_files(&self.content_dir).await? {
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
    content_dir.file(&format!("{}{ITEM_EXT}", id.hyphenated()))
}

fn serialize_item<J: QueueJob, C: Serialize>(
    id: Uuid,
    item: &Item<J, C>,
) -> Result<Vec<u8>, BufferErr> {
    serde_json::to_vec(item).map_err(|source| {
        BufferErr::SerializeItemErr(SerializeItemErr {
            label: J::LABEL,
            id,
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
async fn item_files(content_dir: &Dir) -> Result<Vec<(Uuid, File)>, BufferErr> {
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
) -> Result<QueueSnapshotFile<J>, BufferErr>
where
    J: BufferJob,
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

/// A queue holding every readable item file under its own entry id, oldest
/// [`BufferJob::received_at`] first. Attempt counts and backoff are lost, so
/// every item is due at once. An item file that cannot be parsed is renamed
/// with [`CORRUPT_EXT`] and skipped. A failed listing or read is returned,
/// never rebuilt as a smaller queue.
async fn rebuild_snapshot<J, C>(content_dir: &Dir) -> Result<QueueSnapshot<J>, BufferErr>
where
    J: BufferJob,
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
    entries.sort_by_key(|entry| (entry.job.received_at(), entry.id));
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
