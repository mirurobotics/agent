// internal crates
use crate::filesys::File;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A persisted delete job: "this exact file (path, size, mtime, digest)
/// became deletable at `eligible_at`; delete it once `ttl_secs` have
/// passed". Deliberately event-agnostic — it names *when* the file became
/// deletable, not *why* — so each retention trigger (upload confirmation
/// today; stability for retention-only rules in PR 3b) adds a producer of
/// this job, not fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// The exact file that became deletable. The worker only ever deletes
    /// paths carried by these jobs (path-safety invariant).
    pub file: File,
    /// Size/mtime/digest as recorded when the file became deletable.
    /// The sweep re-stats and deletes on a size+mtime match; when only the
    /// mtime differs it re-hashes and deletes only on a digest match (a
    /// touched-but-unchanged file is absorbed by the scanner as an mtime
    /// alias and never re-uploaded, so no replacement job would ever
    /// arrive).
    pub size: u64,
    pub mtime: DateTime<Utc>,
    pub digest: String,
    /// When the file became deletable (today: upload confirmation).
    pub eligible_at: DateTime<Utc>,
    /// The rule's `retention.ttl_secs`: how long past `eligible_at` the file
    /// must survive before the sweep may delete it.
    pub ttl_secs: u64,
    // for logging only
    pub file_rule_id: String,
    pub deployment_id: String,
}

impl Job {
    /// The instant this job becomes actionable: `eligible_at` plus the TTL.
    /// A TTL too large to represent saturates to the far future — the job is
    /// simply never due (the backend bounds `ttl_secs` on the wire, so this
    /// is defense in depth, not a reachable policy).
    pub fn due_at(&self) -> DateTime<Utc> {
        i64::try_from(self.ttl_secs)
            .ok()
            .and_then(Duration::try_seconds)
            .and_then(|ttl| self.eligible_at.checked_add_signed(ttl))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }
}
