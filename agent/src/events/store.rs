// internal crates
use crate::events::{
    errors::{CursorExpiredErr, EventsErr},
    model::{Event, EventArgs},
};
use crate::filesys::{self, AppendOptions, Overwrite, PathExt, WriteOptions};
use crate::trace;

// external crates
use tracing::{error, warn};

pub const DEFAULT_MAX_RETAINED: usize = 2_000;

pub struct EventStore {
    log_file: filesys::File,
    events: Vec<Event>,
    next_event_id: u64,
    max_retained: usize,
}

impl EventStore {
    /// Load an event store from a JSONL log file.
    ///
    /// `next_event_id` is recovered from the highest ID found in the log.
    /// Malformed or empty lines are skipped so the store always comes up,
    /// even after a partial write or crash.
    pub async fn init(log_file: filesys::File, max_retained: usize) -> Result<Self, EventsErr> {
        let (events, next_event_id) = if log_file.exists() {
            Self::load_log(&log_file).await?
        } else {
            (Vec::new(), 1)
        };

        Ok(Self {
            log_file,
            events,
            next_event_id,
            max_retained,
        })
    }

    pub async fn append(&mut self, event_args: EventArgs) -> Result<Event, EventsErr> {
        let event = Event::new(self.next_event_id, event_args);
        self.next_event_id += 1;

        let json = serde_json::to_string(&event)?;
        self.log_file
            .append_bytes(format!("{json}\n").as_bytes(), AppendOptions::SYNC)
            .await?;

        self.events.push(event.clone());

        if self.needs_compaction() {
            self.try_compact().await;
        }

        Ok(event)
    }

    /// Return all events with id > cursor, validating that the cursor has not
    /// expired (i.e. been compacted away).
    ///
    /// IDs are monotonically increasing and sorted, so we binary-search for the
    /// partition point rather than scanning linearly. We cannot index directly
    /// from the cursor because `load_log()` tolerates malformed/partial lines,
    /// which means persisted event IDs may be sparse after reload.
    pub fn replay_after(&self, cursor: u64) -> Result<Vec<Event>, EventsErr> {
        if cursor != 0 {
            if let Some(earliest) = self.earliest_id() {
                if cursor < earliest {
                    return Err(EventsErr::CursorExpiredErr(CursorExpiredErr {
                        earliest_available: earliest,
                        requested: cursor,
                        trace: trace!(),
                    }));
                }
            }
        }
        let start = self.events.partition_point(|e| e.id <= cursor);
        Ok(self.events[start..].to_vec())
    }

    pub fn earliest_id(&self) -> Option<u64> {
        self.events.first().map(|e| e.id)
    }

    pub fn latest_id(&self) -> Option<u64> {
        self.events.last().map(|e| e.id)
    }

    pub fn needs_compaction(&self) -> bool {
        self.events.len() > self.max_retained
    }

    /// Compact the log to 80% of max_retained.
    ///
    /// Writes the compacted log to a temp file first, then atomically renames
    /// it over the original. The in-memory vec is only trimmed *after* the
    /// disk write succeeds, so a failed compaction never loses events.
    async fn compact(&mut self) -> Result<(), EventsErr> {
        let keep_count = (self.max_retained * 80) / 100;
        if self.events.len() <= keep_count {
            return Ok(());
        }
        let drain_count = self.events.len() - keep_count;

        let content = self.get_compacted_content(drain_count)?;
        self.write_compacted_content(&content).await?;

        // only trim in-memory after disk write succeeded
        self.events.drain(..drain_count);
        Ok(())
    }

    fn get_compacted_content(
        &self,
        drain_count: usize,
    ) -> Result<String, EventsErr> {
        let mut buf = String::new();
        for event in &self.events[drain_count..] {
            let json = serde_json::to_string(event)?;
            buf.push_str(&json);
            buf.push('\n');
        }
        Ok(buf)
    }

    async fn write_compacted_content(
        &self,
        content: &str,
    ) -> Result<(), EventsErr> {
        let tmp_file_path = self.log_file.path().with_extension("jsonl.tmp");
        let tmp_file = filesys::File::new(tmp_file_path);
        tmp_file
            .write_string(content, WriteOptions::OVERWRITE_ATOMIC)
            .await?;
        tmp_file.move_to(&self.log_file, Overwrite::Allow).await?;
        Ok(())
    }

    async fn try_compact(&mut self) {
        if let Err(e) = self.compact().await {
            error!("event store compaction failed: {e}");
        }
    }

    /// Read a JSONL log file into `events`, advancing `next_event_id` past
    /// the highest ID found. Malformed or empty lines are skipped.
    async fn load_log(log_file: &filesys::File) -> Result<(Vec<Event>, u64), EventsErr> {
        let mut events = Vec::new();
        let mut next_event_id: u64 = 1;

        let content = log_file.read_string().await?;
        for (line_num, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(line) {
                Ok(event) => {
                    if event.id >= next_event_id {
                        next_event_id = event.id + 1;
                    }
                    events.push(event);
                }
                Err(e) => {
                    warn!("skipping malformed event at line {}: {e}", line_num + 1);
                }
            }
        }
        Ok((events, next_event_id))
    }
}
