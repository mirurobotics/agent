// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::{errors::*, files, File, PathExt};
use crate::models::UploadRule;
use crate::scan::{
    errors::*,
    state::{Candidate, Config, Observation, StableFile, State},
};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};

/// Owned (non-actor) sub-scanner for a single upload collection.
pub(crate) struct CollectionScanner {
    state: State,
}

impl CollectionScanner {
    pub(crate) async fn new(config: Config, now: DateTime<Utc>) -> Result<Self, ScanErr> {
        let mut state = State::new(config);
        state.preexisting = discover_preexisting(&state, now).await?;
        Ok(Self::from_state(state))
    }

    pub(crate) fn from_state(state: State) -> Self {
        Self { state }
    }

    pub(crate) fn rule(&self) -> &UploadRule {
        self.state.rule()
    }

    pub(crate) fn ledger_count(&self) -> usize {
        self.state.ledger_count()
    }

    pub(crate) fn has_candidates(&self) -> bool {
        self.state.has_candidates()
    }

    /// Replace only the active rule, carrying over observation/dedupe/cadence state.
    pub(crate) async fn update_config(
        &mut self,
        config: Config,
        now: DateTime<Utc>,
    ) -> Result<(), ScanErr> {
        self.state.set_config(config)?;

        // rediscover preexisting files
        let preexisting = discover_preexisting(&self.state, now).await?;
        self.state.preexisting = preexisting;

        Ok(())
    }

    pub(crate) async fn evaluate_candidates(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Vec<StableFile>, ScanErr> {
        let mut stable_files = Vec::new();
        let candidates = self.state.candidates.clone();
        for candidate in candidates.values() {
            let action = eval_candidate(&self.state, candidate, now).await?;
            match action {
                EvalAction::WaitForStabilityWindow => continue,
                EvalAction::Unstable => {
                    self.state.candidates.remove(&candidate.file);
                }
                EvalAction::Stable(stable_file) => {
                    stable_files.push(stable_file.clone());
                    self.state
                        .ledger
                        .insert(candidate.file.clone(), vec![stable_file]);
                }
            }
        }
        Ok(stable_files)
    }

    pub(crate) async fn discover_candidates(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Vec<Candidate>, ScanErr> {
        let candidates = discover_candidates(&self.state, now).await?;
        for candidate in candidates.iter() {
            self.state
                .candidates
                .insert(candidate.file.clone(), candidate.clone());
        }
        Ok(candidates)
    }

    pub(crate) fn prune_ledger(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.state.prune_ledger(before)
    }
}

pub async fn discover_preexisting(
    state: &State,
    now: DateTime<Utc>,
) -> Result<HashMap<File, Observation>, ScanErr> {
    let mut preexisting = HashMap::new();
    for file in files::glob(&state.cfg.rule.source.glob)? {
        if state.is_candidate(&file) {
            continue;
        }
        let observation = observe_file(state, file.clone(), now).await?;
        preexisting.insert(file, observation);
    }
    Ok(preexisting)
}

pub async fn discover_candidates(
    state: &State,
    now: DateTime<Utc>,
) -> Result<Vec<Candidate>, ScanErr> {
    let mut candidates = Vec::new();
    for file in files::glob(&state.cfg.rule.source.glob)? {
        if state.is_candidate(&file) {
            continue;
        }
        let observation = observe_file(state, file.clone(), now).await?;
        if state.is_preexisting(&observation) {
            continue;
        }
        candidates.push(Candidate {
            file,
            observations: vec![observation],
        });
    }
    Ok(candidates)
}

pub async fn observe_file(
    state: &State,
    file: File,
    timestamp: DateTime<Utc>,
) -> Result<Observation, ScanErr> {
    let meta = files::metadata(&file).await?;
    let mtime = meta.modified().map_err(|source| {
        ScanErr::FileSysErr(FileSysErr::FileMetadataErr(FileMetadataErr {
            file: file.clone(),
            source: Box::new(source),
            trace: trace!(),
        }))
    })?;
    Ok(Observation {
        file,
        timestamp,
        size: meta.len(),
        mtime,
        deployment_id: state.cfg.deployment.id.clone(),
        upload_rule_id: state.cfg.rule.upload_collection_id.clone(),
    })
}

pub enum EvalAction {
    WaitForStabilityWindow,
    Unstable,
    Stable(StableFile),
}

async fn eval_candidate(
    state: &State,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Result<EvalAction, ScanErr> {
    if !has_stability_window_elapsed(state, candidate, now)? {
        return Ok(EvalAction::WaitForStabilityWindow);
    }

    if !candidate.file.exists() {
        // file was deleted
        return Ok(EvalAction::Unstable);
    }

    let observation = observe_file(state, candidate.file.clone(), now).await?;
    let outcome = determine_stability(state, candidate, &observation).await?;

    if outcome.is_stable {
        let first = candidate.first_observation()?;
        let last = candidate.latest_observation()?;
        debug_assert!(first.equal_metadata(&last));
        Ok(EvalAction::Stable(StableFile {
            file: candidate.file.clone(),
            size: observation.size,
            digest: outcome.digest()?,
            mtime: last.mtime.into(),
            first_observed_at: first.timestamp,
            last_observed_at: last.timestamp,
            deployment_id: first.deployment_id.clone(),
            upload_rule_id: first.upload_rule_id.clone(),
        }))
    } else {
        Ok(EvalAction::Unstable)
    }
}

fn has_stability_window_elapsed(
    state: &State,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Result<bool, ScanErr> {
    let window = state.cfg.rule.source.stability_window_secs as i64;
    let stable_since = candidate.first_observation()?.timestamp;
    Ok(now.signed_duration_since(stable_since).num_seconds() >= window)
}

pub struct StabilityOutcome {
    pub is_stable: bool,
    pub digest: Option<String>,
}

impl StabilityOutcome {
    pub fn digest(&self) -> Result<String, ScanErr> {
        self.digest
            .clone()
            .ok_or(ScanErr::InternalError(InternalError {
                message: "Digest is not available".to_string(),
                trace: trace!(),
            }))
    }
}

async fn determine_stability(
    state: &State,
    candidate: &Candidate,
    observation: &Observation,
) -> Result<StabilityOutcome, ScanErr> {
    if !is_metadata_stable(candidate, observation)? {
        return Ok(StabilityOutcome {
            is_stable: false,
            digest: None,
        });
    }
    // TODO: add file footer verification for various file types
    differs_from_previous(state, candidate, observation).await
}

fn is_metadata_stable(candidate: &Candidate, observation: &Observation) -> Result<bool, ScanErr> {
    let latest = candidate.latest_observation()?;
    if latest.size != observation.size {
        return Ok(false);
    }
    if latest.mtime != observation.mtime {
        return Ok(false);
    }
    Ok(true)
}

async fn differs_from_previous(
    state: &State,
    candidate: &Candidate,
    observation: &Observation,
) -> Result<StabilityOutcome, ScanErr> {
    // check if there is a previous stable file
    let previous = if let Some(previous) = find_previous_stable_file(state, candidate) {
        previous
    } else {
        return Ok(StabilityOutcome {
            is_stable: true,
            digest: Some(files::hash(&candidate.file).await?),
        });
    };

    // check if stable file size has changed
    if previous.size != observation.size {
        return Ok(StabilityOutcome {
            is_stable: false,
            digest: None,
        });
    }

    // check if stable file digest has changed
    let digest = files::hash(&candidate.file).await?;
    Ok(StabilityOutcome {
        is_stable: true,
        digest: Some(digest),
    })
}

fn find_previous_stable_file(state: &State, candidate: &Candidate) -> Option<StableFile> {
    state
        .ledger
        .get(&candidate.file)
        .and_then(|stable_files| stable_files.last().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // internal crates
    use crate::filesys::PathExt;
    use crate::models::{Deployment, UploadRule, UploadRuleSource};
    use crate::scan::state::{Candidate, Config, Observation, StableFile, State};

    // external crates
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    // ================================ FIXTURES =================================== //

    /// SHA-256 of `b"aaaa"` — the digest `files::hash` returns for that content.
    const HASH_AAAA: &str =
        "sha256:61be55a8e2f6b4e172338bddf184d6dbee29c98853e0a0485ecee7f27b9af0b4";

    /// A deployment with the given id.
    fn deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            ..Default::default()
        }
    }

    /// An UploadRule pinned to a collection id, glob, and stability window.
    fn rule(collection_id: &str, glob: &str, window: i32) -> UploadRule {
        UploadRule {
            upload_collection_id: collection_id.to_string(),
            source: UploadRuleSource {
                glob: glob.to_string(),
                stability_window_secs: window,
            },
            ..Default::default()
        }
    }

    /// A Config wiring the deployment id and a rule together.
    fn config(dpl_id: &str, collection_id: &str, glob: &str, window: i32) -> Config {
        Config {
            deployment: deployment(dpl_id),
            rule: rule(collection_id, glob, window),
        }
    }

    /// A deterministic UTC timestamp at `secs` past the epoch.
    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    /// A persisted temp dir (matches the integration tests' `create_temp`).
    async fn temp_dir() -> std::path::PathBuf {
        crate::filesys::dirs::create_temp("testing")
            .await
            .unwrap()
            .path()
            .clone()
    }

    /// Write `bytes` to `dir/name` and return the corresponding `File`.
    fn write(dir: &Path, name: &str, bytes: &[u8]) -> File {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        File::new(path)
    }

    /// The `*.mcap` glob for a directory.
    fn glob_for(dir: &Path) -> String {
        format!("{}/*.mcap", dir.display())
    }

    /// Observe `file` at `timestamp` through the production `observe_file`.
    async fn observation(state: &State, file: File, timestamp: DateTime<Utc>) -> Observation {
        observe_file(state, file, timestamp).await.unwrap()
    }

    /// A candidate wrapping a single observation.
    fn candidate(file: File, obs: Observation) -> Candidate {
        Candidate {
            file,
            observations: vec![obs],
        }
    }

    /// A StableFile with a given `first_observed_at` (other fields fixed).
    fn stable_file(file: File, first_observed_at: DateTime<Utc>) -> StableFile {
        StableFile {
            file,
            size: 4,
            digest: HASH_AAAA.to_string(),
            mtime: ts(0),
            first_observed_at,
            last_observed_at: first_observed_at,
            deployment_id: "d".to_string(),
            upload_rule_id: "coll".to_string(),
        }
    }

    // ============================ M1: CONSTRUCTION ============================== //

    mod construction {
        use super::*;

        // A file present before the collection is created is snapshotted as
        // preexisting: it is not a candidate and a discover pass finds nothing.
        #[tokio::test]
        async fn new_snapshots_present_file_as_preexisting() {
            let dir = temp_dir().await;
            write(&dir, "a.mcap", b"aaaa");
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 10), ts(1000))
                    .await
                    .unwrap();

            assert!(!scanner.has_candidates());
            let discovered = scanner.discover_candidates(ts(1001)).await.unwrap();
            assert!(discovered.is_empty());
            assert!(!scanner.has_candidates());
        }
    }

    // ===================== M2: update_config ====================== //

    mod update_config {
        use super::*;

        // update_config re-snapshots preexisting and reflects the new rule.
        #[tokio::test]
        async fn update_config_same_collection_resnapshots() {
            let dir = temp_dir().await;
            let glob = glob_for(&dir);
            let mut scanner = CollectionScanner::new(config("d", "coll", &glob, 0), ts(1000))
                .await
                .unwrap();

            // a file appears after creation; update_config re-runs discover_preexisting
            // so the now-present file is snapshotted as preexisting (not a candidate).
            write(&dir, "late.mcap", b"aaaa");
            let mut v2 = rule("coll", &glob, 0);
            v2.id = "r2".to_string();
            scanner
                .update_config(
                    Config {
                        deployment: deployment("d"),
                        rule: v2,
                    },
                    ts(1001),
                )
                .await
                .unwrap();

            assert_eq!(scanner.rule().id, "r2".to_string());
            let discovered = scanner.discover_candidates(ts(1002)).await.unwrap();
            assert!(discovered.is_empty());
        }

        // update_config propagates the InvalidRule error on a collection change.
        #[tokio::test]
        async fn update_config_collection_change_errors() {
            let dir = temp_dir().await;
            let glob = glob_for(&dir);
            let mut scanner = CollectionScanner::new(config("d", "coll", &glob, 0), ts(1000))
                .await
                .unwrap();
            let err = scanner
                .update_config(config("d", "other", &glob, 0), ts(1001))
                .await
                .unwrap_err();
            assert!(matches!(err, ScanErr::InvalidRule(_)));
        }
    }

    // ============================== M3: DISCOVERY ============================== //

    mod discovery {
        use super::*;

        // A preexisting file whose metadata is unchanged is never promoted.
        #[tokio::test]
        async fn unchanged_preexisting_not_promoted() {
            let dir = temp_dir().await;
            write(&dir, "pre.mcap", b"aaaa");
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 0), ts(1000))
                    .await
                    .unwrap();

            let discovered = scanner.discover_candidates(ts(1001)).await.unwrap();
            assert!(discovered.is_empty());
            assert!(!scanner.has_candidates());
        }

        // A preexisting file whose size changed is promoted to a candidate.
        #[tokio::test]
        async fn changed_preexisting_promoted() {
            let dir = temp_dir().await;
            let file = write(&dir, "pre.mcap", b"aaaa");
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 0), ts(1000))
                    .await
                    .unwrap();

            std::fs::write(file.path(), b"bbbbbbbb").unwrap();
            let discovered = scanner.discover_candidates(ts(1001)).await.unwrap();
            assert_eq!(discovered.len(), 1);
            assert!(scanner.has_candidates());
        }

        // An already-tracked candidate is not re-added on a second discover pass.
        #[tokio::test]
        async fn tracked_candidate_not_readded() {
            let dir = temp_dir().await;
            let file = write(&dir, "pre.mcap", b"aaaa");
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 0), ts(1000))
                    .await
                    .unwrap();

            std::fs::write(file.path(), b"bbbbbbbb").unwrap();
            assert_eq!(
                scanner.discover_candidates(ts(1001)).await.unwrap().len(),
                1
            );
            // second pass: the file is already a candidate => skipped.
            assert!(scanner
                .discover_candidates(ts(1002))
                .await
                .unwrap()
                .is_empty());
            assert_eq!(scanner.state.candidates.len(), 1);
        }
    }

    // ========================= M4: STABILITY WINDOWS ========================== //

    mod stability {
        use super::*;

        // has_stability_window_elapsed at the >= boundary and one second short.
        #[tokio::test]
        async fn window_boundary_and_short() {
            let dir = temp_dir().await;
            let file = write(&dir, "w.mcap", b"aaaa");
            let state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);

            // exactly the window (>=) => elapsed.
            assert!(has_stability_window_elapsed(&state, &cand, ts(1010)).unwrap());
            // one second short (N-1) => not elapsed.
            assert!(!has_stability_window_elapsed(&state, &cand, ts(1009)).unwrap());
        }

        // window 0: now == first observation timestamp already satisfies >= 0.
        #[tokio::test]
        async fn window_zero_at_first_timestamp() {
            let dir = temp_dir().await;
            let file = write(&dir, "w.mcap", b"aaaa");
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);
            assert!(has_stability_window_elapsed(&state, &cand, ts(1000)).unwrap());
        }

        // is_metadata_stable: stable iff both size AND mtime match the latest
        // observation. Size and mtime deltas are exercised independently.
        #[test]
        fn is_metadata_stable_size_and_mtime() {
            let file = File::new("/none/x.mcap");
            let base = Observation {
                file: file.clone(),
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                upload_rule_id: "coll".to_string(),
            };
            let cand = candidate(file, base.clone());

            // identical metadata => stable.
            assert!(is_metadata_stable(&cand, &base).unwrap());

            // size differs => unstable.
            let mut diff_size = base.clone();
            diff_size.size = 5;
            assert!(!is_metadata_stable(&cand, &diff_size).unwrap());

            // mtime differs (same size) => unstable.
            let mut diff_mtime = base.clone();
            diff_mtime.mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
            assert!(!is_metadata_stable(&cand, &diff_mtime).unwrap());
        }
    }

    // ============ M5: differs_from_previous / StabilityOutcome ================= //

    mod stability_outcome {
        use super::*;

        // No previous stable file => stable with the real hash of the content.
        #[tokio::test]
        async fn no_previous_hashes_content() {
            let dir = temp_dir().await;
            let file = write(&dir, "s.mcap", b"aaaa");
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs.clone());

            let outcome = differs_from_previous(&state, &cand, &obs).await.unwrap();
            assert!(outcome.is_stable);
            assert_eq!(outcome.digest, Some(HASH_AAAA.to_string()));
        }

        // Previous stable file with the same size => stable, re-hashed.
        #[tokio::test]
        async fn previous_same_size_dedup() {
            let dir = temp_dir().await;
            let file = write(&dir, "s.mcap", b"aaaa");
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(900))]);

            let outcome = differs_from_previous(&state, &cand, &obs).await.unwrap();
            assert!(outcome.is_stable);
            assert_eq!(outcome.digest, Some(HASH_AAAA.to_string()));
        }

        // Previous stable file with a different size => unstable, no digest.
        #[tokio::test]
        async fn previous_size_changed_unstable() {
            let dir = temp_dir().await;
            let file = write(&dir, "s.mcap", b"aaaa");
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            let mut prev = stable_file(file.clone(), ts(900));
            prev.size = 99; // different from the current size of 4
            state.ledger.insert(file, vec![prev]);

            let outcome = differs_from_previous(&state, &cand, &obs).await.unwrap();
            assert!(!outcome.is_stable);
            assert_eq!(outcome.digest, None);
        }

        // determine_stability short-circuits on unstable metadata: no hash taken.
        #[tokio::test]
        async fn determine_stability_short_circuits_on_metadata() {
            let dir = temp_dir().await;
            let file = write(&dir, "s.mcap", b"aaaa");
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs.clone());

            // a fresh observation with a different size => metadata unstable.
            let mut changed = obs;
            changed.size += 1;
            let outcome = determine_stability(&state, &cand, &changed).await.unwrap();
            assert!(!outcome.is_stable);
            assert_eq!(outcome.digest, None);
        }

        // StabilityOutcome::digest maps Some => Ok, None => Err(InternalError).
        #[test]
        fn digest_ok_and_err() {
            let some = StabilityOutcome {
                is_stable: true,
                digest: Some(HASH_AAAA.to_string()),
            };
            assert_eq!(some.digest().unwrap(), HASH_AAAA.to_string());

            let none = StabilityOutcome {
                is_stable: false,
                digest: None,
            };
            assert!(matches!(none.digest(), Err(ScanErr::InternalError(_))));
        }
    }

    // ============== M6: eval_candidate / evaluate_candidates =================== //

    mod eval_candidate_fn {
        use super::*;

        // A stable candidate produces a StableFile with the expected payload.
        #[tokio::test]
        async fn stable_payload_fields() {
            let dir = temp_dir().await;
            let file = write(&dir, "e.mcap", b"aaaa");
            let state = State::new(config("dpl-1", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let mtime = obs.mtime;
            let cand = candidate(file, obs);

            let action = eval_candidate(&state, &cand, ts(1010)).await.unwrap();
            let EvalAction::Stable(sf) = action else {
                panic!("expected Stable");
            };
            // lint:allow(field-by-field-assert)
            assert_eq!(sf.file.name().unwrap(), "e.mcap");
            assert_eq!(sf.size, 4);
            assert_eq!(sf.digest, HASH_AAAA.to_string());
            assert_eq!(sf.mtime, DateTime::<Utc>::from(mtime));
            assert_eq!(sf.first_observed_at, ts(1000));
            assert_eq!(sf.last_observed_at, ts(1000));
            assert_eq!(sf.deployment_id, "dpl-1".to_string());
            assert_eq!(sf.upload_rule_id, "coll".to_string());
        }

        // A candidate whose file was deleted hits the Unstable (deleted) branch —
        // reachable directly but awkward through the actor.
        #[tokio::test]
        async fn deleted_file_is_unstable() {
            let dir = temp_dir().await;
            let file = write(&dir, "gone.mcap", b"aaaa");
            let state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs);

            std::fs::remove_file(file.path()).unwrap();
            let action = eval_candidate(&state, &cand, ts(1010)).await.unwrap();
            assert!(matches!(action, EvalAction::Unstable));
        }

        // Before the window elapses, evaluation waits.
        #[tokio::test]
        async fn not_yet_elapsed_waits() {
            let dir = temp_dir().await;
            let file = write(&dir, "wait.mcap", b"aaaa");
            let state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);

            let action = eval_candidate(&state, &cand, ts(1009)).await.unwrap();
            assert!(matches!(action, EvalAction::WaitForStabilityWindow));
        }

        // evaluate_candidates inserts a stable file into the ledger and returns it.
        #[tokio::test]
        async fn evaluate_inserts_stable() {
            let dir = temp_dir().await;
            let file = write(&dir, "ins.mcap", b"aaaa");
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            state.candidates.insert(file.clone(), candidate(file, obs));
            let mut scanner = CollectionScanner::from_state(state);

            let stable = scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert_eq!(stable.len(), 1);
            assert_eq!(scanner.ledger_count(), 1);
        }

        // evaluate_candidates drops an Unstable (deleted) candidate: nothing
        // reported, the candidate removed, the ledger untouched.
        #[tokio::test]
        async fn evaluate_drops_unstable() {
            let dir = temp_dir().await;
            let file = write(&dir, "drop.mcap", b"aaaa");
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            state
                .candidates
                .insert(file.clone(), candidate(file.clone(), obs));
            std::fs::remove_file(file.path()).unwrap();
            let mut scanner = CollectionScanner::from_state(state);

            let stable = scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert!(stable.is_empty());
            assert!(!scanner.state.candidates.contains_key(&file));
            assert_eq!(scanner.ledger_count(), 0);
        }
    }

    // =============================== M7: LEDGER =============================== //

    mod ledger {
        use super::*;

        // Two distinct stable files yield a ledger count of two.
        #[tokio::test]
        async fn ledger_count_two_distinct() {
            let dir = temp_dir().await;
            let f1 = write(&dir, "one.mcap", b"aaaa");
            let f2 = write(&dir, "two.mcap", b"aaaa");
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let o1 = observation(&state, f1.clone(), ts(1000)).await;
            let o2 = observation(&state, f2.clone(), ts(1000)).await;
            state.candidates.insert(f1.clone(), candidate(f1, o1));
            state.candidates.insert(f2.clone(), candidate(f2, o2));
            let mut scanner = CollectionScanner::from_state(state);

            scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert_eq!(scanner.ledger_count(), 2);
        }

        // A tracked candidate re-evaluated does not re-report: dedup keeps count 1.
        #[tokio::test]
        async fn report_once_dedup() {
            let dir = temp_dir().await;
            let file = write(&dir, "dedup.mcap", b"aaaa");
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            state.candidates.insert(file.clone(), candidate(file, obs));
            let mut scanner = CollectionScanner::from_state(state);

            let first = scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert_eq!(first.len(), 1);
            // the candidate is still tracked; re-evaluating re-inserts the SAME ledger
            // entry (keyed by file), so the count does not grow.
            let second = scanner.evaluate_candidates(ts(1020)).await.unwrap();
            assert_eq!(second.len(), 1);
            assert_eq!(scanner.ledger_count(), 1);
        }
    }
}
