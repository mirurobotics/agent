// standard crates
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use crate::mocks::{clock::Clock, error::SleepController};
use miru_agent::filesys::{self, PathExt};
use miru_agent::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};
use miru_agent::storage::{self, Layout};
use miru_agent::workers::uploads;
use miru_agent::workers::uploads::{active_upload_rules, decide_ready, FileObservation, ReadyFile};

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

    #[allow(clippy::type_complexity)]
    async fn spawn_stores(
        layout: &Layout,
    ) -> (
        Arc<storage::Deployments>,
        Arc<storage::Releases>,
        Arc<storage::UploadRules>,
    ) {
        let (deployments, _h1) = storage::Deployments::spawn(64, layout.deployments(), 1000)
            .await
            .unwrap();
        let (releases, _h2) = storage::Releases::spawn(64, layout.releases(), 1000)
            .await
            .unwrap();
        let (upload_rules, _h3) = storage::UploadRules::spawn(64, layout.upload_rules(), 1000)
            .await
            .unwrap();
        (Arc::new(deployments), Arc::new(releases), Arc::new(upload_rules))
    }

    fn spawn_worker(
        options: uploads::Options,
        deployments: Arc<storage::Deployments>,
        releases: Arc<storage::Releases>,
        upload_rules: Arc<storage::UploadRules>,
        sleep_ctrl: Arc<SleepController>,
        clock: Clock,
    ) {
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        tokio::spawn(async move {
            uploads::run(
                &options,
                deployments.as_ref(),
                releases.as_ref(),
                upload_rules.as_ref(),
                sleep_ctrl.sleep_fn(),
                clock.now_fn(),
                shutdown_signal,
            )
            .await;
        });
    }

    /// Seed a Deployed deployment -> release -> upload rule bodies so the
    /// traversal in `active_upload_rules` resolves to the given rules.
    async fn seed_deployed(
        deployments: &storage::Deployments,
        releases: &storage::Releases,
        upload_rules: &storage::UploadRules,
        dpl_id: &str,
        release_id: &str,
        rules: &[UploadRule],
    ) {
        deployments
            .write_if_absent(
                dpl_id.to_string(),
                Deployment {
                    id: dpl_id.to_string(),
                    activity_status: DplActivity::Deployed,
                    release_id: release_id.to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();
        releases
            .write_if_absent(
                release_id.to_string(),
                Release {
                    id: release_id.to_string(),
                    upload_rule_ids: rules.iter().map(|r| r.id.clone()).collect(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();
        for rule in rules {
            upload_rules
                .write_if_absent(rule.id.clone(), rule.clone(), |_, _| false)
                .await
                .unwrap();
        }
    }

    // T3: per-rule cadence. Two rules (poll 5 and 30): the loop's wait is always
    // governed by the nearest-due rule (~5s), proving each rule keeps its own
    // cadence (rule A re-due every 5s never lets the 30s rule B shorten the wait).
    #[tokio::test]
    async fn per_rule_cadence_two_rules() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir.clone());
        let base = dir.path().clone();
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
        // globs match nothing; cadence is what matters here.
        let rule_a = rule_with("a", &format!("{}/a/*.mcap", base.display()), 5, 0);
        let rule_b = rule_with("b", &format!("{}/b/*.mcap", base.display()), 30, 0);
        seed_deployed(
            &deployments,
            &releases,
            &upload_rules,
            "dpl_a",
            "rel_a",
            &[rule_a, rule_b],
        )
        .await;

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
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
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
        let rule_b = rule_with("b", &format!("{}/b/*.mcap", base.display()), 30, 0);
        seed_deployed(
            &deployments,
            &releases,
            &upload_rules,
            "dpl_b",
            "rel_b",
            &[rule_b],
        )
        .await;

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
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
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
            sleep_ctrl.clone(),
            clock.clone(),
        );

        for _ in 0..5 {
            sleep_ctrl.await_sleep().await;
            assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);
            sleep_ctrl.release().await;
        }
    }

    // T5 (cache-error): after the deployments cache actor is shut down,
    // `find_where` (the first cache call in the traversal) errors; the worker
    // logs and treats it as empty, continuing to idle at the min interval
    // without panicking.
    #[tokio::test]
    async fn cache_error_is_treated_as_empty_and_idles() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;

        let sleep_ctrl = Arc::new(SleepController::new());
        let clock = Clock::new(1000);
        spawn_worker(
            uploads::Options::default(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
            sleep_ctrl.clone(),
            clock.clone(),
        );

        // let the worker complete one normal (empty) scan + sleep.
        sleep_ctrl.await_sleep().await;
        assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);

        // shut the deployments cache actor down so subsequent `find_where` calls
        // error, exercising the cache-error-treated-as-empty path.
        deployments.shutdown().await.unwrap();

        // the worker keeps looping: the cache error is logged + treated as empty,
        // so it continues idling at the min interval with no panic.
        for _ in 0..3 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);
        }
    }

    // ====================== active_upload_rules TRAVERSAL ===================== //

    mod active_set {
        use super::*;

        /// The set of resolved rule ids, order-independent.
        fn ids(rules: &[UploadRule]) -> BTreeSet<String> {
            rules.iter().map(|r| r.id.clone()).collect()
        }

        // TS3a: Deployed deployment -> release ids [r1,r2] with both bodies
        // present => both rules resolve.
        #[tokio::test]
        async fn resolves_active_set() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            let r1 = rule_with("r1", "/none/*.mcap", 60, 0);
            let r2 = rule_with("r2", "/none/*.mcap", 60, 0);
            seed_deployed(
                &deployments,
                &releases,
                &upload_rules,
                "dpl",
                "rel",
                &[r1, r2],
            )
            .await;

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string(), "r2".to_string()]));
        }

        // TS3b: a rule body present in the store but NOT referenced by the
        // deployed release is not acted on.
        #[tokio::test]
        async fn stale_rule_not_acted_on() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            let r1 = rule_with("r1", "/none/*.mcap", 60, 0);
            // release references only r1, but r2's body also lives in the store.
            seed_deployed(&deployments, &releases, &upload_rules, "dpl", "rel", &[r1])
                .await;
            upload_rules
                .write_if_absent(
                    "r2".to_string(),
                    rule_with("r2", "/none/*.mcap", 60, 0),
                    |_, _| false,
                )
                .await
                .unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string()]));
        }

        // TS3c: no Deployed deployment (only a Queued one) => empty.
        #[tokio::test]
        async fn no_deployed_is_empty() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            deployments
                .write_if_absent(
                    "dpl".to_string(),
                    Deployment {
                        id: "dpl".to_string(),
                        activity_status: DplActivity::Queued,
                        release_id: "rel".to_string(),
                        ..Default::default()
                    },
                    |_, _| false,
                )
                .await
                .unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert!(rules.is_empty());
        }

        // TS3d: release references [r1,r2] but only r1's body is in the store =>
        // missing rule id is skipped.
        #[tokio::test]
        async fn missing_rule_id_skipped() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            let r1 = rule_with("r1", "/none/*.mcap", 60, 0);
            let r2 = rule_with("r2", "/none/*.mcap", 60, 0);
            // seed release referencing both, but only write r1's body.
            deployments
                .write_if_absent(
                    "dpl".to_string(),
                    Deployment {
                        id: "dpl".to_string(),
                        activity_status: DplActivity::Deployed,
                        release_id: "rel".to_string(),
                        ..Default::default()
                    },
                    |_, _| false,
                )
                .await
                .unwrap();
            releases
                .write_if_absent(
                    "rel".to_string(),
                    Release {
                        id: "rel".to_string(),
                        upload_rule_ids: vec![r1.id.clone(), r2.id.clone()],
                        ..Default::default()
                    },
                    |_, _| false,
                )
                .await
                .unwrap();
            upload_rules
                .write_if_absent(r1.id.clone(), r1, |_, _| false)
                .await
                .unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string()]));
        }

        // TS3d2: Deployed deployment whose release_id is not cached => empty.
        #[tokio::test]
        async fn missing_release_is_empty() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            deployments
                .write_if_absent(
                    "dpl".to_string(),
                    Deployment {
                        id: "dpl".to_string(),
                        activity_status: DplActivity::Deployed,
                        release_id: "absent".to_string(),
                        ..Default::default()
                    },
                    |_, _| false,
                )
                .await
                .unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert!(rules.is_empty());
        }

        // TS3e: two Deployed deployments referencing overlapping rule ids union
        // and dedupe to a single set.
        #[tokio::test]
        async fn union_and_dedupe_across_deployments() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            let r1 = rule_with("r1", "/none/*.mcap", 60, 0);
            let r2 = rule_with("r2", "/none/*.mcap", 60, 0);
            seed_deployed(
                &deployments,
                &releases,
                &upload_rules,
                "dpl_A",
                "rel_A",
                &[r1.clone()],
            )
            .await;
            seed_deployed(
                &deployments,
                &releases,
                &upload_rules,
                "dpl_B",
                "rel_B",
                &[r1, r2],
            )
            .await;

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string(), "r2".to_string()]));
            assert_eq!(rules.len(), 2);
        }

        // TS3-err-deployments: deployments store shut down => find_where errors,
        // treated as empty.
        #[tokio::test]
        async fn deployments_cache_error_is_empty() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            deployments.shutdown().await.unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert!(rules.is_empty());
        }

        // TS3-err-release: releases store shut down => read_optional errors for
        // the deployed release, that deployment is skipped.
        #[tokio::test]
        async fn release_cache_error_is_empty() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            let r1 = rule_with("r1", "/none/*.mcap", 60, 0);
            seed_deployed(&deployments, &releases, &upload_rules, "dpl", "rel", &[r1])
                .await;
            releases.shutdown().await.unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert!(rules.is_empty());
        }

        // TS3-err-rule: upload_rules store shut down => read_optional errors per
        // rule id, all skipped.
        #[tokio::test]
        async fn rule_cache_error_is_empty() {
            let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
            let layout = Layout::new(dir);
            let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
            let r1 = rule_with("r1", "/none/*.mcap", 60, 0);
            seed_deployed(&deployments, &releases, &upload_rules, "dpl", "rel", &[r1])
                .await;
            upload_rules.shutdown().await.unwrap();

            let rules =
                active_upload_rules(deployments.as_ref(), releases.as_ref(), upload_rules.as_ref())
                    .await;
            assert!(rules.is_empty());
        }
    }
}
