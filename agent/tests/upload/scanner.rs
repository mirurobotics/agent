// standard crates
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

// internal crates
use crate::mocks::clock::Clock;
use miru_agent::filesys::{self, File, PathExt};
use miru_agent::models::{UploadRule, UploadRuleSource};
use miru_agent::upload::scanner::{find_stable, Observation, StableFile};
use miru_agent::upload::scanner::{Scanner, ScannerArgs};
use miru_agent::upload::ScannerExt;

// external crates
use chrono::{DateTime, Utc};

// =============================== TEST HELPERS ================================= //

/// A DateTime<Utc> at the given epoch seconds.
fn t(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

/// Build an UploadRule from Default with only the source fields set.
fn rule_with(id: &str, glob: &str, stability_window_secs: i32) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        source: UploadRuleSource {
            glob: glob.to_string(),
            stability_window_secs,
        },
        ..Default::default()
    }
}

/// Build an UploadRule with a pinned upload_collection_id (plus id/glob/stability).
fn rule_in_collection(
    id: &str,
    collection_id: &str,
    glob: &str,
    stability_window_secs: i32,
) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        upload_collection_id: collection_id.to_string(),
        source: UploadRuleSource {
            glob: glob.to_string(),
            stability_window_secs,
        },
        ..Default::default()
    }
}

/// The set of file names (last path component) of the returned stable files.
fn stable_names(stable: &[StableFile]) -> BTreeSet<String> {
    stable
        .iter()
        .map(|sf| sf.path.file_name().unwrap().to_string_lossy().to_string())
        .collect()
}

// ========================== PURE find_stable TESTS ============================ //

mod pure {
    use super::*;

    // T1: glob match/miss (absolute), recursive `**`, and an invalid glob.
    #[tokio::test]
    async fn glob_match_miss_recursive_and_invalid() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::create_dir_all(base.join("data")).unwrap();
        std::fs::create_dir_all(base.join("other")).unwrap();
        std::fs::write(base.join("data/a.mcap"), b"aaa").unwrap();
        std::fs::write(base.join("data/b.txt"), b"bbb").unwrap();
        std::fs::write(base.join("other/c.mcap"), b"ccc").unwrap();

        // --- absolute, non-recursive: only a.mcap matches ---
        let rule = rule_with("r1", &format!("{}/data/*.mcap", base.display()), 0);
        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();
        // first call records the observation (stability window 0 still requires a
        // prior observation to compare against), so nothing is stable yet.
        let first = find_stable(&rule, &mut obs, &mut ledger, t(0));
        assert!(first.is_empty());
        // second call at a later `now`: a.mcap is now newly stable.
        let second = find_stable(&rule, &mut obs, &mut ledger, t(1));
        assert_eq!(
            stable_names(&second),
            BTreeSet::from(["a.mcap".to_string()])
        );

        // --- recursive `**`: a.mcap + c.mcap match ---
        let rule = rule_with("r2", &format!("{}/**/*.mcap", base.display()), 0);
        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(0)).is_empty());
        let stable = find_stable(&rule, &mut obs, &mut ledger, t(1));
        assert_eq!(
            stable_names(&stable),
            BTreeSet::from(["a.mcap".to_string(), "c.mcap".to_string()])
        );

        // --- invalid glob: returns empty, no panic ---
        let rule = rule_with("r3", "[", 0);
        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(0)).is_empty());
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(100)).is_empty());
    }

    // T2: stability state machine.
    #[tokio::test]
    async fn stability_state_machine() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        // ---- change resets the window ----
        let file = base.join("reset.mcap");
        std::fs::write(&file, b"aaa").unwrap();
        let rule = rule_with("reset", &glob, 10);
        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();

        // t0: record the observation.
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(0)).is_empty());
        // rewrite with DIFFERENT bytes (changes size) before t=5 -> resets window.
        std::fs::write(&file, b"bbbbbb").unwrap();
        // t5: change detected, window resets to 5, not stable.
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(5)).is_empty());
        // t12: only 7s since reset (< 10) -> not stable.
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(12)).is_empty());
        // t16: 11s since reset (>= 10) -> stable.
        let stable = find_stable(&rule, &mut obs, &mut ledger, t(16));
        assert_eq!(
            stable_names(&stable),
            BTreeSet::from(["reset.mcap".to_string()])
        );

        // ---- unchanged >= window => stable ----
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        std::fs::write(base.join("stable.mcap"), b"aaa").unwrap();
        let rule = rule_with("stable", &glob, 10);
        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(0)).is_empty());
        // unchanged file, exactly the window has elapsed -> stable.
        let stable = find_stable(&rule, &mut obs, &mut ledger, t(10));
        assert_eq!(
            stable_names(&stable),
            BTreeSet::from(["stable.mcap".to_string()])
        );

        // ---- not-yet-stable ----
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        std::fs::write(base.join("young.mcap"), b"aaa").unwrap();
        let rule = rule_with("young", &glob, 10);
        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(0)).is_empty());
        // 9s < 10s window -> not stable.
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(9)).is_empty());
    }

    // T4: a stable file is returned as newly-stable exactly ONCE.
    #[tokio::test]
    async fn dedupe_reports_once() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        std::fs::write(base.join("once.mcap"), b"aaa").unwrap();
        let rule = rule_with("once", &glob, 0);

        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();

        let mut total_stable = 0;
        for now_secs in [0, 1, 2, 3, 4] {
            total_stable += find_stable(&rule, &mut obs, &mut ledger, t(now_secs)).len();
        }
        // recorded on the first call, reported once on the second, suppressed after.
        assert_eq!(total_stable, 1);
    }

    // T5: a glob matching nothing returns empty, no panic.
    #[tokio::test]
    async fn no_match_is_empty() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let rule = rule_with("none", &format!("{}/nope/*.mcap", base.display()), 0);

        let mut obs: HashMap<File, Observation> = HashMap::new();
        let mut ledger: HashSet<File> = HashSet::new();
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(0)).is_empty());
        assert!(find_stable(&rule, &mut obs, &mut ledger, t(100)).is_empty());
    }
}

// =============================== ACTOR TESTS ================================== //

mod actor {
    use super::*;

    /// Spawn a scanner actor with a deterministic injected clock.
    fn spawn_scanner(clock: &Clock, min_poll: i64) -> Scanner {
        let (u, _h) = Scanner::spawn(
            64,
            ScannerArgs {
                min_poll_interval_secs: min_poll,
                now_fn: Arc::new(clock.now_fn()),
            },
        )
        .unwrap();
        u
    }

    /// The set of rule ids currently held by the scanner.
    fn ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
    }

    // update_rules is a state-preserving reconcile at collection granularity: it
    // DROPS collections no longer present in the pushed set. Here each rule sits in
    // its own default collection (rule_with mints a fresh collection id per rule),
    // so pushing [a,b] then [c] drops the a/b collections and leaves only c.
    #[tokio::test]
    async fn update_rules_reconciles_active_set() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);

        scanner
            .update_rules(vec![
                rule_with("a", "/none/*.mcap", 0),
                rule_with("b", "/none/*.mcap", 0),
            ])
            .await
            .unwrap();
        assert_eq!(
            ids(&scanner.get_rules().await.unwrap()),
            BTreeSet::from(["a".to_string(), "b".to_string()])
        );

        scanner
            .update_rules(vec![rule_with("c", "/none/*.mcap", 0)])
            .await
            .unwrap();
        assert_eq!(
            ids(&scanner.get_rules().await.unwrap()),
            BTreeSet::from(["c".to_string()])
        );
    }

    // Per-collection state carries over across a rule-version swap: replacing a
    // collection's rule (new rule id + digest, SAME collection id) preserves that
    // collection's observation/dedupe state, so an already-reported file is not
    // re-reported after the swap.
    #[tokio::test]
    async fn update_rules_carries_state_across_rule_version_swap() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("carry.mcap"), b"ccc").unwrap();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);

        // v1: pin collection "coll-A" with rule id "r1" / digest "d1".
        let mut rule_v1 = rule_in_collection("r1", "coll-A", &glob, 0);
        rule_v1.digest = "d1".to_string();
        scanner.update_rules(vec![rule_v1]).await.unwrap();

        // record only.
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +1s: the file crosses into stable and is reported once.
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        // v2: SAME collection "coll-A", different rule id "r2" / digest "d2".
        let mut rule_v2 = rule_in_collection("r2", "coll-A", &glob, 0);
        rule_v2.digest = "d2".to_string();
        scanner.update_rules(vec![rule_v2]).await.unwrap();

        // +1s and scan again: the dedupe set carried across the swap, so the
        // already-reported file is NOT re-reported -> count stays at 1.
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        // The swap took effect: exactly one rule, and its digest is the v2 digest.
        let rules = scanner.get_rules().await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].digest, "d2".to_string());
    }

    // A single push carrying two rules for the SAME collection is folded to one
    // (last-in-the-vec wins) before any state is touched, collapsing the redeploy
    // double-rule case. The winning rule's glob matches the one real file.
    #[tokio::test]
    async fn update_rules_double_rule_tie_break_last_wins() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("hit.mcap"), b"hhh").unwrap();
        let glob_match = format!("{}/*.mcap", base.display());
        let glob_miss = format!("{}/nope/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);

        // Both rules pin collection "coll-B"; rule_y is last in the vec, so it wins.
        let mut rule_x = rule_in_collection("rx", "coll-B", &glob_miss, 0);
        rule_x.digest = "d1".to_string();
        let mut rule_y = rule_in_collection("ry", "coll-B", &glob_match, 0);
        rule_y.digest = "d2".to_string();
        scanner.update_rules(vec![rule_x, rule_y]).await.unwrap();

        // Exactly one rule survives, and it is the last-in-vec (digest "d2").
        let rules = scanner.get_rules().await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].digest, "d2".to_string());

        // record only.
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +1s: the single (winning) collection's glob matches the one file -> one
        // report, no double-count, no panic.
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // Two DISTINCT collections do not share observation/dedupe state: given two
    // rules in different collections with identical globs matching the same one
    // file, that file crosses into stable once PER collection -> reported twice.
    // Under the old global map this would dedupe to 1.
    #[tokio::test]
    async fn distinct_collections_do_not_share_observation_state() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("shared.mcap"), b"sss").unwrap();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);

        let rule_c1 = rule_in_collection("c1", "coll-C1", &glob, 0);
        let rule_c2 = rule_in_collection("c2", "coll-C2", &glob, 0);
        scanner.update_rules(vec![rule_c1, rule_c2]).await.unwrap();

        // record only (both collections observe the file).
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +1s: each collection independently sees the file cross into stable, so the
        // summed ledger count is 2 (once per collection).
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 2);
    }

    // Each rule keeps its own scan cadence: a 5s rule is scanned at +5s while a
    // 30s rule is skipped until +30s.
    #[tokio::test]
    async fn scan_uses_global_cadence() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::create_dir_all(base.join("a")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::write(base.join("a/x.mcap"), b"xxx").unwrap();
        std::fs::write(base.join("b/y.mcap"), b"yyy").unwrap();

        let rule_a = rule_with("a", &format!("{}/a/*.mcap", base.display()), 0);
        let rule_b = rule_with("b", &format!("{}/b/*.mcap", base.display()), 0);

        let clock = Clock::new(1000);
        // all rules share a single global cadence of 1s.
        let scanner = spawn_scanner(&clock, 1);
        scanner.update_rules(vec![rule_a, rule_b]).await.unwrap();

        // first sighting just records observations; nothing stable yet.
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +1s: the global interval has elapsed, so BOTH rules come due in the same
        // scan and both files are now stable. There is no per-rule
        // interval, so neither rule is skipped.
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 2);
    }

    // A rule scanned within the global cadence window is skipped on the next scan:
    // the file would otherwise be stable, but the rule is not yet due so it is not
    // re-scanned and nothing is reported until the interval elapses.
    #[tokio::test]
    async fn scan_skips_rule_until_cadence_elapses() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("w.mcap"), b"www").unwrap();

        let rule = rule_with("w", &format!("{}/*.mcap", base.display()), 0);

        let clock = Clock::new(1000);
        // global cadence of 10s: after the first scan the rule is not due again
        // until t=1010.
        let scanner = spawn_scanner(&clock, 10);
        scanner.update_rules(vec![rule]).await.unwrap();

        // record only: stable_since=1000, next_scan_at=1010.
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +1s (t=1001 < 1010): the file is stable, but the rule
        // is not yet due, so the scan skips it and nothing is reported.
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +9s (t=1010 >= 1010): the cadence has elapsed, the rule is due, and the
        // now-stable file is reported.
        clock.advance(9);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // The stability window is honored through the actor's scan path.
    #[tokio::test]
    async fn stability_window_through_scan() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("z.mcap"), b"zzz").unwrap();

        let rule = rule_with("z", &format!("{}/*.mcap", base.display()), 10);

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);
        scanner.update_rules(vec![rule]).await.unwrap();

        // record only: stable_since=1000, next_scan_at=1001.
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // +10s: due (1010 >= 1001) and unchanged for 10s >= window 10 -> stable.
        clock.advance(10);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // A stable file is counted exactly once across repeated scans.
    #[tokio::test]
    async fn dedupe_reports_once_across_scans() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        std::fs::write(base.join("once.mcap"), b"ooo").unwrap();

        let rule = rule_with("once", &format!("{}/*.mcap", base.display()), 0);

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);
        scanner.update_rules(vec![rule]).await.unwrap();

        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // With no rules the scan is a no-op: no panic, nothing reported.
    #[tokio::test]
    async fn empty_set_scan_is_noop() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock, 1);

        for _ in 0..5 {
            scanner.scan().await.unwrap();
        }
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // shutdown stops the actor loop; the actor task completes and any subsequent
    // command fails with a send error (the receiver is gone).
    #[tokio::test]
    async fn shutdown_stops_actor_and_later_commands_error() {
        let clock = Clock::new(1000);
        let (scanner, handle) = Scanner::spawn(
            64,
            ScannerArgs {
                min_poll_interval_secs: 1,
                now_fn: Arc::new(clock.now_fn()),
            },
        )
        .unwrap();

        scanner.shutdown().await.unwrap();
        // the run loop broke out of the receive loop, so the task finishes.
        handle.await.unwrap();

        let err = scanner.scan().await.unwrap_err();
        assert!(matches!(
            err,
            miru_agent::upload::UploadErr::SendActorMessageErr(_)
        ));
    }
}
