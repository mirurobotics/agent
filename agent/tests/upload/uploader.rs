// standard crates
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use crate::mocks::clock::Clock;
use miru_agent::filesys::{self, PathExt};
use miru_agent::models::{UploadRule, UploadRuleSource};
use miru_agent::upload::uploader::{decide_ready, FileObservation, ReadyFile};
use miru_agent::upload::uploader::{Uploader, UploaderArgs};
use miru_agent::upload::UploaderExt;

// external crates
use chrono::{DateTime, Utc};

// =============================== TEST HELPERS ================================= //

/// A DateTime<Utc> at the given epoch seconds.
fn t(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

/// Build an UploadRule from Default with only the source fields set.
fn rule_with(
    id: &str,
    glob: &str,
    poll_interval_secs: i32,
    stability_window_secs: i32,
) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        source: UploadRuleSource {
            glob: glob.to_string(),
            poll_interval_secs,
            stability_window_secs,
        },
        ..Default::default()
    }
}

/// The set of file names (last path component) of the returned ready files.
fn ready_names(ready: &[ReadyFile]) -> BTreeSet<String> {
    ready
        .iter()
        .map(|rf| rf.path.file_name().unwrap().to_string_lossy().to_string())
        .collect()
}

// =========================== PURE decide_ready TESTS ========================== //

mod pure {
    use super::*;

    // T1: glob match/miss (absolute), recursive `**`, and an invalid glob.
    #[tokio::test]
    async fn glob_match_miss_recursive_and_invalid() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::create_dir_all(base.join("data")).unwrap();
        std::fs::create_dir_all(base.join("other")).unwrap();
        std::fs::write(base.join("data/a.mcap"), b"aaa").unwrap();
        std::fs::write(base.join("data/b.txt"), b"bbb").unwrap();
        std::fs::write(base.join("other/c.mcap"), b"ccc").unwrap();

        // --- absolute, non-recursive: only a.mcap matches ---
        let rule = rule_with("r1", &format!("{}/data/*.mcap", base.display()), 60, 0);
        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();
        // first call records the observation (stability window 0 still requires a
        // prior observation to compare against), so nothing is ready yet.
        let first = decide_ready(&rule, &mut obs, &mut reported, t(0));
        assert!(first.is_empty());
        // second call at a later `now`: a.mcap is now stable and newly ready.
        let second = decide_ready(&rule, &mut obs, &mut reported, t(1));
        assert_eq!(ready_names(&second), BTreeSet::from(["a.mcap".to_string()]));

        // --- recursive `**`: a.mcap + c.mcap match ---
        let rule = rule_with("r2", &format!("{}/**/*.mcap", base.display()), 60, 0);
        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(0)).is_empty());
        let ready = decide_ready(&rule, &mut obs, &mut reported, t(1));
        assert_eq!(
            ready_names(&ready),
            BTreeSet::from(["a.mcap".to_string(), "c.mcap".to_string()])
        );

        // --- invalid glob: returns empty, no panic ---
        let rule = rule_with("r3", "[", 60, 0);
        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(0)).is_empty());
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(100)).is_empty());
    }

    // T2: stability state machine.
    #[tokio::test]
    async fn stability_state_machine() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        // ---- change resets the window ----
        let file = base.join("reset.mcap");
        std::fs::write(&file, b"aaa").unwrap();
        let rule = rule_with("reset", &glob, 60, 10);
        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();

        // t0: record the observation.
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(0)).is_empty());
        // rewrite with DIFFERENT bytes (changes size) before t=5 -> resets window.
        std::fs::write(&file, b"bbbbbb").unwrap();
        // t5: change detected, window resets to 5, not ready.
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(5)).is_empty());
        // t12: only 7s since reset (< 10) -> not ready.
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(12)).is_empty());
        // t16: 11s since reset (>= 10) -> ready.
        let ready = decide_ready(&rule, &mut obs, &mut reported, t(16));
        assert_eq!(
            ready_names(&ready),
            BTreeSet::from(["reset.mcap".to_string()])
        );

        // ---- unchanged >= window => ready ----
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        std::fs::write(base.join("stable.mcap"), b"aaa").unwrap();
        let rule = rule_with("stable", &glob, 60, 10);
        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(0)).is_empty());
        // unchanged file, exactly the window has elapsed -> ready.
        let ready = decide_ready(&rule, &mut obs, &mut reported, t(10));
        assert_eq!(
            ready_names(&ready),
            BTreeSet::from(["stable.mcap".to_string()])
        );

        // ---- not-yet-stable ----
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        std::fs::write(base.join("young.mcap"), b"aaa").unwrap();
        let rule = rule_with("young", &glob, 60, 10);
        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(0)).is_empty());
        // 9s < 10s window -> not ready.
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(9)).is_empty());
    }

    // T4: a stable file is returned as newly-ready exactly ONCE.
    #[tokio::test]
    async fn dedupe_reports_once() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        std::fs::write(base.join("once.mcap"), b"aaa").unwrap();
        let rule = rule_with("once", &glob, 60, 0);

        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();

        let mut total_ready = 0;
        for now_secs in [0, 1, 2, 3, 4] {
            total_ready += decide_ready(&rule, &mut obs, &mut reported, t(now_secs)).len();
        }
        // recorded on the first call, reported once on the second, suppressed after.
        assert_eq!(total_ready, 1);
    }

    // T5: a glob matching nothing returns empty, no panic.
    #[tokio::test]
    async fn no_match_is_empty() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        let rule = rule_with("none", &format!("{}/nope/*.mcap", base.display()), 60, 0);

        let mut obs: HashMap<PathBuf, FileObservation> = HashMap::new();
        let mut reported: HashSet<PathBuf> = HashSet::new();
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(0)).is_empty());
        assert!(decide_ready(&rule, &mut obs, &mut reported, t(100)).is_empty());
    }
}

// =============================== ACTOR TESTS ================================== //

mod actor {
    use super::*;

    /// Spawn an uploader actor with a deterministic injected clock.
    fn spawn_uploader(clock: &Clock, min_poll: i64) -> Uploader {
        let (u, _h) = Uploader::spawn(
            64,
            UploaderArgs {
                min_poll_interval_secs: min_poll,
                now_fn: Arc::new(clock.now_fn()),
            },
        )
        .unwrap();
        u
    }

    /// The set of rule ids currently held by the uploader.
    fn ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
    }

    // update_rules replaces the active set wholesale (not a merge).
    #[tokio::test]
    async fn update_rules_replaces_set() {
        let clock = Clock::new(1000);
        let uploader = spawn_uploader(&clock, 1);

        uploader
            .update_rules(vec![
                rule_with("a", "/none/*.mcap", 60, 0),
                rule_with("b", "/none/*.mcap", 60, 0),
            ])
            .await
            .unwrap();
        assert_eq!(
            ids(&uploader.get_rules().await.unwrap()),
            BTreeSet::from(["a".to_string(), "b".to_string()])
        );

        uploader
            .update_rules(vec![rule_with("c", "/none/*.mcap", 60, 0)])
            .await
            .unwrap();
        assert_eq!(
            ids(&uploader.get_rules().await.unwrap()),
            BTreeSet::from(["c".to_string()])
        );
    }

    // Each rule keeps its own scan cadence: a 5s rule is scanned at +5s while a
    // 30s rule is skipped until +30s.
    #[tokio::test]
    async fn scan_honors_per_rule_cadence() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::create_dir_all(base.join("a")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::write(base.join("a/x.mcap"), b"xxx").unwrap();
        std::fs::write(base.join("b/y.mcap"), b"yyy").unwrap();

        let rule_a = rule_with("a", &format!("{}/a/*.mcap", base.display()), 5, 0);
        let rule_b = rule_with("b", &format!("{}/b/*.mcap", base.display()), 30, 0);

        let clock = Clock::new(1000);
        let uploader = spawn_uploader(&clock, 1);
        uploader.update_rules(vec![rule_a, rule_b]).await.unwrap();

        // first sighting just records observations; nothing ready yet.
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 0);

        // +5s: rule_a is due (and its file is now stable->ready); rule_b is NOT
        // due until +30s so it is skipped. Proves per-rule cadence.
        clock.advance(5);
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 1);

        // +30s: rule_b now due and its file is ready; rule_a's file was already
        // reported so it dedupes.
        clock.advance(25);
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 2);
    }

    // The readiness stability window is honored through the actor's scan path.
    #[tokio::test]
    async fn readiness_window_through_scan() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("z.mcap"), b"zzz").unwrap();

        let rule = rule_with("z", &format!("{}/*.mcap", base.display()), 1, 10);

        let clock = Clock::new(1000);
        let uploader = spawn_uploader(&clock, 1);
        uploader.update_rules(vec![rule]).await.unwrap();

        // record only: stable_since=1000, next_scan_at=1001.
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 0);

        // +10s: due (1010 >= 1001) and unchanged for 10s >= window 10 -> ready.
        clock.advance(10);
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 1);
    }

    // A ready file is counted exactly once across repeated scans.
    #[tokio::test]
    async fn dedupe_reports_once_across_scans() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("once.mcap"), b"ooo").unwrap();

        let rule = rule_with("once", &format!("{}/*.mcap", base.display()), 1, 0);

        let clock = Clock::new(1000);
        let uploader = spawn_uploader(&clock, 1);
        uploader.update_rules(vec![rule]).await.unwrap();

        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 0);

        clock.advance(1);
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 1);

        clock.advance(1);
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 1);

        clock.advance(1);
        uploader.scan().await.unwrap();
        assert_eq!(uploader.get_reported_count().await.unwrap(), 1);
    }

    // With no rules the scan is a no-op: no panic, nothing reported.
    #[tokio::test]
    async fn empty_set_scan_is_noop() {
        let clock = Clock::new(1000);
        let uploader = spawn_uploader(&clock, 1);

        for _ in 0..5 {
            uploader.scan().await.unwrap();
        }
        assert_eq!(uploader.get_reported_count().await.unwrap(), 0);
    }
}
