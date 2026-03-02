// standard library
use std::io::Write;
use std::path::{Path, PathBuf};

// internal crates
use super::errors::{AppendErr, InitErr, ReadErr};
use super::model::Envelope;

// external crates
use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};

/// Metadata persisted alongside the event log.
#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    next_event_id: u64,
}

/// Durable NDJSON event store with monotonic ID assignment and bounded retention.
pub struct Store {
    log_path: PathBuf,
    meta_path: PathBuf,
    next_id: u64,
    earliest_id: Option<u64>,
    retained_count: usize,
    max_retained: usize,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("log_path", &self.log_path)
            .field("next_id", &self.next_id)
            .field("earliest_id", &self.earliest_id)
            .field("retained_count", &self.retained_count)
            .finish()
    }
}

impl Store {
    /// Initialize the event store. Creates the parent directory for the log and
    /// meta files, then scans the existing NDJSON log (if present) to recover
    /// next_id and earliest_id.
    pub fn init(log_path: &Path, meta_path: &Path, max_retained: usize) -> Result<Self, InitErr> {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| InitErr { source: e })?;
        }

        let log_path = log_path.to_path_buf();
        let meta_path = meta_path.to_path_buf();

        let mut next_id: u64 = 1;
        let mut earliest_id: Option<u64> = None;
        let mut retained_count: usize = 0;

        // try to recover state from meta file first
        if meta_path.exists() {
            match std::fs::read_to_string(&meta_path) {
                Ok(contents) => {
                    if let Ok(meta) = serde_json::from_str::<Meta>(&contents) {
                        next_id = meta.next_event_id;
                    }
                }
                Err(e) => {
                    warn!("failed to read event meta file, will scan log: {e}");
                }
            }
        }

        // scan log to find actual bounds and validate next_id
        if log_path.exists() {
            match std::fs::read_to_string(&log_path) {
                Ok(contents) => {
                    let mut max_seen: u64 = 0;
                    let mut min_seen: Option<u64> = None;
                    for line in contents.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        retained_count += 1;
                        match serde_json::from_str::<Envelope>(line) {
                            Ok(env) => {
                                if env.id > max_seen {
                                    max_seen = env.id;
                                }
                                min_seen = Some(match min_seen {
                                    Some(m) => m.min(env.id),
                                    None => env.id,
                                });
                            }
                            Err(e) => {
                                // tolerate trailing malformed line
                                warn!("skipping malformed event log line: {e}");
                            }
                        }
                    }
                    if max_seen >= next_id {
                        next_id = max_seen + 1;
                    }
                    earliest_id = min_seen;
                }
                Err(e) => {
                    warn!("failed to read event log, starting fresh: {e}");
                }
            }
        }

        // persist recovered meta
        let store = Self {
            log_path,
            meta_path,
            next_id,
            earliest_id,
            retained_count,
            max_retained,
        };
        store.write_meta();

        debug!(
            "event store initialized: next_id={}, earliest_id={:?}, retained_count={}",
            store.next_id, store.earliest_id, store.retained_count
        );

        Ok(store)
    }

    /// Assign the next monotonic ID to the envelope, append it to the NDJSON
    /// log, flush, and return the assigned envelope.
    pub fn append(&mut self, mut envelope: Envelope) -> Result<Envelope, AppendErr> {
        envelope.id = self.next_id;
        self.next_id += 1;

        let mut line = serde_json::to_string(&envelope).map_err(|e| AppendErr {
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        })?;
        line.push('\n');

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| AppendErr { source: e })?;

        file.write_all(line.as_bytes())
            .map_err(|e| AppendErr { source: e })?;
        file.flush().map_err(|e| AppendErr { source: e })?;

        if self.earliest_id.is_none() {
            self.earliest_id = Some(envelope.id);
        }
        self.retained_count += 1;

        self.write_meta();
        self.compact_if_needed();

        Ok(envelope)
    }

    /// Replay events with id > `after`, up to `limit`. Returns envelopes in
    /// ascending ID order.
    pub fn replay_after(&self, after: u64, limit: usize) -> Result<Vec<Envelope>, ReadErr> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let contents =
            std::fs::read_to_string(&self.log_path).map_err(|e| ReadErr { source: e })?;

        let mut result = Vec::new();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Envelope>(line) {
                Ok(env) if env.id > after => {
                    result.push(env);
                    if result.len() >= limit {
                        break;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("skipping malformed event log line during replay: {e}");
                }
            }
        }

        Ok(result)
    }

    /// The ID that will be assigned to the next event.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// The earliest retained event ID, if any events exist.
    pub fn earliest_id(&self) -> Option<u64> {
        self.earliest_id
    }

    fn write_meta(&self) {
        let meta = Meta {
            next_event_id: self.next_id,
        };
        if let Ok(json) = serde_json::to_string_pretty(&meta) {
            if let Err(e) = std::fs::write(&self.meta_path, json) {
                error!("failed to write event meta: {e}");
            }
        }
    }

    fn compact_if_needed(&mut self) {
        if self.retained_count <= self.max_retained {
            return;
        }

        let retain_target = self.retain_target();
        debug!(
            "compacting event log: {} events, retaining {}",
            self.retained_count, retain_target
        );

        let contents = match std::fs::read_to_string(&self.log_path) {
            Ok(c) => c,
            Err(e) => {
                error!("failed to read event log for compaction: {e}");
                return;
            }
        };

        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        self.retained_count = lines.len();
        if self.retained_count <= self.max_retained {
            return;
        }

        if retain_target == 0 {
            match std::fs::write(&self.log_path, "") {
                Ok(()) => {
                    self.retained_count = 0;
                    self.earliest_id = None;
                }
                Err(e) => {
                    error!("failed to truncate event log: {e}");
                }
            }
            return;
        }

        let keep = &lines[lines.len() - retain_target..];

        let tmp_path = self.log_path.with_extension("ndjson.tmp");
        match std::fs::write(&tmp_path, keep.join("\n") + "\n") {
            Ok(()) => {
                if let Err(e) = std::fs::rename(&tmp_path, &self.log_path) {
                    error!("failed to rename compacted event log: {e}");
                    return;
                }
                self.retained_count = keep.len();
                self.earliest_id = keep.iter().find_map(|line| {
                    serde_json::from_str::<Envelope>(line)
                        .ok()
                        .map(|env| env.id)
                });
                debug!("event log compacted to {} events", self.retained_count);
            }
            Err(e) => {
                error!("failed to write compacted event log: {e}");
            }
        }
    }

    fn retain_target(&self) -> usize {
        // Reclaim 10% headroom (for large windows) so we don't compact after
        // every append once retention is crossed.
        self.max_retained.saturating_sub(self.max_retained / 10)
    }
}
