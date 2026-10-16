// standard crates
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes moved by a transfer of a fixed `total` size. Counts describe what the
/// transfer code has handed off, not what the remote end has acknowledged.
#[derive(Debug)]
pub struct Progress {
    total: u64,
    sent: AtomicU64,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            sent: AtomicU64::new(0),
        }
    }

    pub fn add(&self, bytes: u64) {
        self.sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Moves the position to `offset`, e.g. when the transfer resends from an
    /// earlier byte, and returns the previous position.
    pub fn seek_to(&self, offset: u64) -> u64 {
        self.sent.swap(offset, Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    #[test]
    fn counts_bytes_and_seeks() {
        let progress = Progress::new(10);
        progress.add(4);
        progress.add(3);
        assert_eq!(progress.seek_to(2), 7);
        assert_eq!((progress.total(), progress.sent()), (10, 2));
    }
}
