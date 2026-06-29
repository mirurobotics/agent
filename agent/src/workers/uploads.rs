// standard crates
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::models::{UploadRule, UploadRuleID};
use crate::storage;

// external crates
use chrono::{DateTime, Utc};
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Options {
    /// Floor for the worker's sleep between scan passes and a fallback when a
    /// rule's poll_interval_secs is <= 0. Guards against a 0/negative interval
    /// busy-looping the worker.
    pub min_poll_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_poll_interval_secs: 1,
        }
    }
}

/// Per-file stability state used by the readiness state machine. A file is
/// "ready" once its (size, mtime) have been unchanged for at least
/// `stability_window_secs` since `stable_since`.
struct FileObservation {
    size: u64,
    mtime: std::time::SystemTime,
    stable_since: DateTime<Utc>,
}

/// A file that has been determined ready for upload. Fields are public so unit
/// tests can assert against the pure `decide_ready` seam directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyFile {
    pub path: PathBuf,
    pub modified_at: DateTime<Utc>,
}

pub async fn run<SleepF, SleepFut, NowF>(
    options: &Options,
    upload_rules: &storage::UploadRules,
    sleep_fn: SleepF,
    now_fn: NowF,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    SleepF: Fn(Duration) -> SleepFut,
    SleepFut: Future<Output = ()> + Send,
    NowF: Fn() -> DateTime<Utc>,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Uploads worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(options, upload_rules, sleep_fn, now_fn) => {}
    }
}

async fn run_impl<SleepF, SleepFut, NowF>(
    options: &Options,
    upload_rules: &storage::UploadRules,
    sleep_fn: SleepF,
    now_fn: NowF,
) where
    SleepF: Fn(Duration) -> SleepFut,
    SleepFut: Future<Output = ()> + Send,
    NowF: Fn() -> DateTime<Utc>,
{
    info!("Running uploads worker");

    // in-memory state: re-derived from the cache + filesystem on restart.
    let mut next_scan_at: HashMap<UploadRuleID, DateTime<Utc>> = HashMap::new();
    let mut observations: HashMap<PathBuf, FileObservation> = HashMap::new();
    let mut already_reported: HashSet<PathBuf> = HashSet::new();

    loop {
        let now = now_fn();

        // load the current rules; on a cache error log and treat as empty so the
        // worker keeps running rather than crashing.
        let rules = match upload_rules.values().await {
            Ok(rules) => rules,
            Err(e) => {
                error!("error reading cached upload rules: {e:?}");
                Vec::new()
            }
        };

        for rule in &rules {
            let due = next_scan_at.get(&rule.id).is_none_or(|next| *next <= now);
            if !due {
                continue;
            }

            // decide_ready does blocking fs I/O (glob enumeration + stat). Per the
            // Decision Log, run it under spawn_blocking to keep the runtime
            // responsive. The observations map is keyed by PathBuf across all
            // rules; we move it into the blocking task and take it back out
            // afterwards (alongside the newly-ready files) so decide_ready itself
            // stays a plain, unit-testable sync fn.
            let rule_clone = rule.clone();
            let moved_observations = std::mem::take(&mut observations);
            let (returned_observations, ready) = match tokio::task::spawn_blocking(move || {
                let mut obs = moved_observations;
                let ready = decide_ready(&rule_clone, &mut obs, now);
                (obs, ready)
            })
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    error!("uploads readiness task panicked: {e:?}");
                    (HashMap::new(), Vec::new())
                }
            };
            observations = returned_observations;

            // placeholder sink: emit a log line per newly-ready file exactly once.
            // M3 replaces this with the digest + POST /uploads pipeline.
            for rf in ready {
                if already_reported.contains(&rf.path) {
                    continue;
                }
                info!(
                    file_path = %rf.path.display(),
                    file_modified_at = %rf.modified_at,
                    "upload candidate ready (M2 placeholder sink)"
                );
                already_reported.insert(rf.path.clone());
            }

            let interval_secs = std::cmp::max(
                rule.source.poll_interval_secs as i64,
                options.min_poll_interval_secs,
            );
            next_scan_at.insert(
                rule.id.clone(),
                now + chrono::TimeDelta::seconds(interval_secs),
            );
        }

        // prune next_scan_at entries for rules that are no longer present.
        let present_ids: HashSet<&UploadRuleID> = rules.iter().map(|r| &r.id).collect();
        next_scan_at.retain(|id, _| present_ids.contains(id));

        // compute the sleep: the minimum remaining time until any rule is due,
        // clamped to >= min_poll_interval_secs; idle at min_poll_interval_secs
        // when there are no rules.
        let wait_secs = next_scan_at
            .values()
            .map(|next| next.signed_duration_since(now).num_seconds())
            .min()
            .map(|secs| std::cmp::max(secs, options.min_poll_interval_secs))
            .unwrap_or(options.min_poll_interval_secs);
        let wait = Duration::from_secs(wait_secs.max(0) as u64);

        debug!("uploads worker sleeping {wait:?} until next scan");
        sleep_fn(wait).await;
    }
}

/// Pure readiness decision for a single rule. Enumerates `rule.source.glob`,
/// stats each matched file, updates the per-file stability state in
/// `observations`, and returns the files that have been size/mtime-stable for
/// at least `stability_window_secs`.
///
/// This does BLOCKING filesystem I/O (glob walk + metadata) and is intended to
/// be called inside `tokio::task::spawn_blocking`. It is kept sync and pure
/// (state in/out via the `observations` argument) so it can be unit-tested
/// directly without a runtime.
fn decide_ready(
    rule: &UploadRule,
    observations: &mut HashMap<PathBuf, FileObservation>,
    now: DateTime<Utc>,
) -> Vec<ReadyFile> {
    let mut ready = Vec::new();

    let paths = match glob::glob(&rule.source.glob) {
        Ok(paths) => paths,
        Err(e) => {
            error!("invalid glob {:?}: {e}", rule.source.glob);
            return ready;
        }
    };

    for entry in paths {
        let path = match entry {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !path.is_file() {
            continue;
        }

        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue, // e.g. file deleted mid-scan
        };
        let size = meta.len();
        let mtime = match meta.modified() {
            Ok(mtime) => mtime,
            Err(_) => continue,
        };

        match observations.get(&path) {
            // new file, or size/mtime changed since last observation: (re)start
            // the stability window and do NOT report.
            Some(obs) if obs.size == size && obs.mtime == mtime => {
                // HOOK (M3): finalization-marker detection (MCAP footer / parquet
                // magic bytes) would gate readiness here in addition to size+mtime
                // stability. NOT implemented in M2.
                if now.signed_duration_since(obs.stable_since).num_seconds()
                    >= rule.source.stability_window_secs as i64
                {
                    ready.push(ReadyFile {
                        path: path.clone(),
                        modified_at: mtime.into(),
                    });
                }
            }
            _ => {
                observations.insert(
                    path,
                    FileObservation {
                        size,
                        mtime,
                        stable_since: now,
                    },
                );
            }
        }
    }

    ready
}
