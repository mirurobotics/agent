// standard crates
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use crate::mocks::{clock::Clock, error::SleepController};
use miru_agent::filesys::{self, PathExt};
use miru_agent::models::{UploadRule, UploadRuleSource};
use miru_agent::storage::{self, Layout};
use miru_agent::workers::uploads;
use miru_agent::workers::uploads::{decide_ready, FileObservation, ReadyFile};

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

// ============================= RUN-LOOP TESTS ================================= //

mod run_loop {
    use super::*;

    async fn spawn_rules(layout: &Layout) -> Arc<storage::UploadRules> {
        let (rules, _handle) = storage::UploadRules::spawn(64, layout.upload_rules(), 1000)
            .await
            .unwrap();
        Arc::new(rules)
    }

    fn spawn_worker(
        options: uploads::Options,
        rules: Arc<storage::UploadRules>,
        sleep_ctrl: Arc<SleepController>,
        clock: Clock,
    ) {
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        tokio::spawn(async move {
            uploads::run(
                &options,
                rules.as_ref(),
                sleep_ctrl.sleep_fn(),
                clock.now_fn(),
                shutdown_signal,
            )
            .await;
        });
    }

    // T3: per-rule cadence. Two rules (poll 5 and 30): the loop's wait is always
    // governed by the nearest-due rule (~5s), proving each rule keeps its own
    // cadence (rule A re-due every 5s never lets the 30s rule B shorten the wait).
    #[tokio::test]
    async fn per_rule_cadence_two_rules() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir.clone());
        let base = dir.path().clone();
        let rules = spawn_rules(&layout).await;
        // globs match nothing; cadence is what matters here.
        rules
            .write_if_absent(
                "a".to_string(),
                rule_with("a", &format!("{}/a/*.mcap", base.display()), 5, 0),
                |_, _| false,
            )
            .await
            .unwrap();
        rules
            .write_if_absent(
                "b".to_string(),
                rule_with("b", &format!("{}/b/*.mcap", base.display()), 30, 0),
                |_, _| false,
            )
            .await
            .unwrap();

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            rules.clone(),
            sleep_ctrl.clone(),
            clock.clone(),
        );

        // initial scan of both rules -> nearest due is rule A at +5s.
        sleep_ctrl.await_sleep().await;
        assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 5);

        // advance in 5s steps across 35 simulated seconds (past rule B's 30s mark);
        // rule A re-due each step keeps the computed wait pinned at 5s.
        for _ in 0..7 {
            clock.advance(5);
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 5);
        }
    }

    // T3 (companion): a lone 30s rule sleeps ~30s, demonstrating the per-rule
    // poll_interval_secs is honored independently of the min floor.
    #[tokio::test]
    async fn per_rule_cadence_single_rule() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir.clone());
        let base = dir.path().clone();
        let rules = spawn_rules(&layout).await;
        rules
            .write_if_absent(
                "b".to_string(),
                rule_with("b", &format!("{}/b/*.mcap", base.display()), 30, 0),
                |_, _| false,
            )
            .await
            .unwrap();

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            rules.clone(),
            sleep_ctrl.clone(),
            clock.clone(),
        );

        sleep_ctrl.await_sleep().await;
        assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 30);

        for _ in 0..3 {
            clock.advance(30);
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 30);
        }
    }

    // T5: no rules -> worker idles at min_poll_interval_secs (1s), no panic.
    #[tokio::test]
    async fn empty_rules_idles_at_min_interval() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let rules = spawn_rules(&layout).await;

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            rules.clone(),
            sleep_ctrl.clone(),
            clock.clone(),
        );

        for _ in 0..5 {
            sleep_ctrl.await_sleep().await;
            assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);
            sleep_ctrl.release().await;
        }
    }

    // T5 (cache-error): after the cache actor is shut down, `values()` errors;
    // the worker logs and treats it as empty, continuing to idle at the min
    // interval without panicking.
    #[tokio::test]
    async fn cache_error_is_treated_as_empty_and_idles() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let rules = spawn_rules(&layout).await;

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            rules.clone(),
            sleep_ctrl.clone(),
            clock.clone(),
        );

        // let the worker complete one normal (empty) scan + sleep.
        sleep_ctrl.await_sleep().await;
        assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);

        // shut the cache actor down so subsequent `values()` calls error.
        rules.shutdown().await.unwrap();

        // the worker keeps looping: the cache error is logged + treated as empty,
        // so it continues idling at the min interval with no panic.
        for _ in 0..3 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);
        }
    }
}
