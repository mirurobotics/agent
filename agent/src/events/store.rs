// standard crates
use std::fs;
use std::io::{BufRead, BufWriter, Write};

// internal crates
use crate::events::{
    errors::{CursorExpiredErr, EventsErr},
    model::{Envelope, NewEvent},
};
use crate::filesys::{self, PathExt};
use crate::trace;

// external crates
use tracing::{error, warn};

pub const DEFAULT_MAX_RETAINED: usize = 2_000;

pub struct EventStore {
    log_file: filesys::File,
    events: Vec<Envelope>,
    next_event_id: u64,
    max_retained: usize,
}

impl EventStore {
    /// Load an event store from a JSONL log file.
    ///
    /// `next_event_id` is recovered from the highest ID found in the log.
    /// Malformed or empty lines are skipped so the store always comes up,
    /// even after a partial write or crash.
    pub fn init(log_file: filesys::File, max_retained: usize) -> Result<Self, EventsErr> {
        let mut events = Vec::new();
        let mut next_event_id: u64 = 1;

        if log_file.exists() {
            Self::load_log(&log_file, &mut events, &mut next_event_id)?;
        }

        Ok(Self {
            log_file,
            events,
            next_event_id,
            max_retained,
        })
    }

    pub fn append(&mut self, event: NewEvent) -> Result<Envelope, EventsErr> {
        let envelope = Envelope::from_new_event(self.next_event_id, event);
        self.next_event_id += 1;

        let json = serde_json::to_string(&envelope)?;

        self.ensure_parent_dir()?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_file.path())?;
        writeln!(file, "{json}")?;
        file.flush()?;

        self.events.push(envelope.clone());

        if self.events.len() > self.max_retained {
            if let Err(e) = self.compact() {
                error!("event store compaction failed: {e}");
            }
        }

        Ok(envelope)
    }

    /// Return all events with id > cursor, validating that the cursor has not
    /// expired (i.e. been compacted away).
    ///
    /// IDs are monotonically increasing and sorted, so we binary-search for the
    /// partition point rather than scanning linearly.
    pub fn replay_after(&self, cursor: u64) -> Result<Vec<Envelope>, EventsErr> {
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

    /// Compact the log to 90% of max_retained.
    ///
    /// Writes the compacted log to a temp file first, then atomically renames
    /// it over the original. The in-memory vec is only trimmed *after* the
    /// disk write succeeds, so a failed compaction never loses events.
    fn compact(&mut self) -> Result<(), EventsErr> {
        let keep_count = self.max_retained / 2;
        if self.events.len() <= keep_count {
            return Ok(());
        }
        let drain_count = self.events.len() - keep_count;

        // write compacted log to temp file before touching in-memory state
        let tmp_path = self.log_file.path().with_extension("jsonl.tmp");
        let file = fs::File::create(&tmp_path)?;
        let mut writer = BufWriter::new(file);
        for event in &self.events[drain_count..] {
            let json = serde_json::to_string(event)?;
            writeln!(writer, "{json}")?;
        }
        writer.flush()?;
        fs::rename(&tmp_path, self.log_file.path())?;

        // only trim in-memory after disk write succeeded
        self.events.drain(..drain_count);
        Ok(())
    }

    /// Read a JSONL log file into `events`, advancing `next_event_id` past
    /// the highest ID found. Malformed or empty lines are skipped.
    fn load_log(
        log_file: &filesys::File,
        events: &mut Vec<Envelope>,
        next_event_id: &mut u64,
    ) -> Result<(), EventsErr> {
        let file = fs::File::open(log_file.path())?;
        for (line_num, line) in std::io::BufReader::new(file).lines().enumerate() {
            let line = match line {
                Ok(l) if l.trim().is_empty() => continue,
                Ok(l) => l,
                Err(e) => {
                    warn!("skipping unreadable event at line {}: {e}", line_num + 1);
                    continue;
                }
            };
            match serde_json::from_str::<Envelope>(&line) {
                Ok(envelope) => {
                    if envelope.id >= *next_event_id {
                        *next_event_id = envelope.id + 1;
                    }
                    events.push(envelope);
                }
                Err(e) => {
                    warn!("skipping malformed event at line {}: {e}", line_num + 1);
                }
            }
        }
        Ok(())
    }

    fn ensure_parent_dir(&self) -> Result<(), EventsErr> {
        if let Some(parent) = self.log_file.path().parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        Ok(())
    }
}
