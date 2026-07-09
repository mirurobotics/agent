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

    pub(crate) async fn discover_candidates(&mut self, now: DateTime<Utc>) -> Result<(), ScanErr> {
        let candidates = discover_candidates(&self.state, now).await?;
        self.state
            .candidates
            .extend(candidates.into_iter().map(|c| (c.file.clone(), c)));
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
                EvalAction::AlreadyInLedger(observation) => {
                    self.state.candidates.remove(&candidate.file);
                    self.state.add_mtime_alias_to_latest_ledger_entry(
                        &candidate.file,
                        observation.mtime.into(),
                    )?;
                }
                EvalAction::Stable(stable_file) => {
                    self.state.candidates.remove(&candidate.file);
                    stable_files.push(stable_file.clone());
                    self.state
                        .ledger
                        .entry(candidate.file.clone())
                        .or_default()
                        .push(stable_file);
                }
            }
        }
        Ok(stable_files)
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
        if state.is_latest_ledger_entry(&observation) {
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
    AlreadyInLedger(Observation),
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

    match outcome {
        StabilityOutcome::Stable(digest) => {
            let first = candidate.first_observation()?;
            let last = candidate.latest_observation()?;
            debug_assert!(first.equal_metadata(&last));
            Ok(EvalAction::Stable(StableFile {
                file: candidate.file.clone(),
                size: observation.size,
                digest,
                mtime: last.mtime.into(),
                mtime_aliases: vec![],
                first_observed_at: first.timestamp,
                last_observed_at: last.timestamp,
                deployment_id: first.deployment_id.clone(),
                upload_rule_id: first.upload_rule_id.clone(),
            }))
        }
        StabilityOutcome::AlreadyInLedger => Ok(EvalAction::AlreadyInLedger(observation)),
        StabilityOutcome::Unstable => Ok(EvalAction::Unstable),
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

pub type Digest = String;

pub enum StabilityOutcome {
    AlreadyInLedger,
    Stable(Digest),
    Unstable,
}

async fn determine_stability(
    state: &State,
    candidate: &Candidate,
    observation: &Observation,
) -> Result<StabilityOutcome, ScanErr> {
    if !is_metadata_stable(candidate, observation)? {
        return Ok(StabilityOutcome::Unstable);
    }
    // TODO: add file footer verification for various file types
    differs_from_previous(state, candidate).await
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
) -> Result<StabilityOutcome, ScanErr> {
    // check if there is a previous stable file
    let previous = if let Some(previous) = find_previous_stable_file(state, candidate) {
        previous
    } else {
        return Ok(StabilityOutcome::Stable(
            files::hash(&candidate.file).await?,
        ));
    };

    // check if stable file digest has changed; any digest change is a new version
    let digest = files::hash(&candidate.file).await?;
    if digest != previous.digest {
        return Ok(StabilityOutcome::Stable(digest));
    }
    Ok(StabilityOutcome::AlreadyInLedger)
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
    use crate::filesys::{dirs, Dir, PathExt, WriteOptions};
    use crate::models::{Deployment, UploadRule, UploadRuleSource};
    use crate::scan::state::{Candidate, Config, Observation, StableFile, State};

    // external crates
    use std::time::SystemTime;

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

    /// Atomically overwrite `file` with `bytes`.
    async fn write_file(file: &File, bytes: &[u8]) {
        files::write_bytes(file, bytes, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }

    /// Write `bytes` to `dir/name` and return the corresponding `File`.
    async fn write(dir: &Dir, name: &str, bytes: &[u8]) -> File {
        let file = dir.file(name);
        write_file(&file, bytes).await;
        file
    }

    /// The `*.mcap` glob for a directory.
    fn glob_for(dir: &Dir) -> String {
        format!("{}/*.mcap", dir.path().display())
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
            mtime_aliases: vec![],
            first_observed_at,
            last_observed_at: first_observed_at,
            deployment_id: "d".to_string(),
            upload_rule_id: "coll".to_string(),
        }
    }

    // ============================ M1: CONSTRUCTION ============================== //

    mod construction {
        use super::*;

        #[tokio::test]
        async fn new_snapshots_present_file_as_preexisting() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "a.mcap", b"aaaa").await;
            let cfg = config("d", "coll", &glob_for(&dir), 10);
            let mut scanner = CollectionScanner::new(cfg.clone(), ts(1000)).await.unwrap();

            let mut expected_state = State::new(cfg);
            expected_state.preexisting.insert(
                file.clone(),
                observation(&expected_state, file.clone(), ts(1000)).await,
            );
            assert_eq!(scanner.state, expected_state);

            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
            assert_eq!(scanner.state, expected_state);
        }
    }

    // ============================ M2: update_config ============================== //

    mod update_config {
        use super::*;

        // update_config propagates the InvalidRule error on a collection change.
        #[tokio::test]
        async fn update_config_different_colls_err() {
            let dir = dirs::temp("testing").unwrap();
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

        // update_config re-snapshots newly present files as preexisting and swaps the rule.
        #[tokio::test]
        async fn update_config_resnapshots_preexisting() {
            let dir = dirs::temp("testing").unwrap();
            let glob = glob_for(&dir);
            let mut scanner = CollectionScanner::new(config("d", "coll", &glob, 0), ts(1000))
                .await
                .unwrap();

            let late_file = write(&dir, "late.mcap", b"aaaa").await;
            let mut v2 = rule("coll", &glob, 0);
            v2.id = "r2".to_string();
            let cfg = Config {
                deployment: deployment("d"),
                rule: v2,
            };
            scanner.update_config(cfg.clone(), ts(1001)).await.unwrap();

            let mut expected_state = State::new(cfg);
            expected_state.preexisting.insert(
                late_file.clone(),
                observation(&expected_state, late_file, ts(1001)).await,
            );
            assert_eq!(scanner.state, expected_state);

            scanner.discover_candidates(ts(1002)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
            assert_eq!(scanner.state, expected_state);
        }

        // update_config leaves tracked candidates in place; they are not moved into preexisting.
        #[tokio::test]
        async fn update_config_preserves_candidates() {
            let dir = dirs::temp("testing").unwrap();
            let glob = glob_for(&dir);
            let candidate_file = write(&dir, "pre.mcap", b"aaaa").await;
            let mut scanner = CollectionScanner::new(config("d", "coll", &glob, 0), ts(1000))
                .await
                .unwrap();

            write_file(&candidate_file, b"bbbbbbbb").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            let expected_candidate = scanner
                .state
                .candidates
                .get(&candidate_file)
                .cloned()
                .expect("changed preexisting file promoted to candidate");

            let mut v2 = rule("coll", &glob, 0);
            v2.id = "r2".to_string();
            let cfg = Config {
                deployment: deployment("d"),
                rule: v2,
            };
            scanner.update_config(cfg.clone(), ts(1002)).await.unwrap();

            let mut expected_state = State::new(cfg);
            expected_state
                .candidates
                .insert(candidate_file, expected_candidate);
            assert_eq!(scanner.state, expected_state);
        }
    }

    // ============================== M3: DISCOVERY ============================== //

    mod discovery {
        use super::*;

        // A preexisting file whose metadata is unchanged is never promoted.
        #[tokio::test]
        async fn unchanged_preexisting_not_promoted() {
            let dir = dirs::temp("testing").unwrap();
            write(&dir, "pre.mcap", b"aaaa").await;
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 0), ts(1000))
                    .await
                    .unwrap();

            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
        }

        // A preexisting file whose size changed is promoted to a candidate.
        #[tokio::test]
        async fn changed_preexisting_promoted() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "pre.mcap", b"aaaa").await;
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 0), ts(1000))
                    .await
                    .unwrap();

            write_file(&file, b"bbbbbbbb").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert_eq!(scanner.state.candidates.len(), 1);
        }

        // A file that appears after scanner creation is discovered as a candidate.
        #[tokio::test]
        async fn new_file_adds_to_candidates() {
            let dir = dirs::temp("testing").unwrap();
            let cfg = config("d", "coll", &glob_for(&dir), 0);
            let mut scanner = CollectionScanner::new(cfg.clone(), ts(1000)).await.unwrap();
            assert_eq!(scanner.state, State::new(cfg.clone()));

            let file = write(&dir, "new.mcap", b"aaaa").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert_eq!(scanner.state.candidates.len(), 1);

            let mut expected_state = State::new(cfg);
            expected_state.candidates.insert(
                file.clone(),
                candidate(
                    file.clone(),
                    observation(&expected_state, file, ts(1001)).await,
                ),
            );
            assert_eq!(scanner.state, expected_state);
        }

        // An already-tracked candidate is not re-added on a second discover pass.
        #[tokio::test]
        async fn tracked_candidate_not_readded() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "pre.mcap", b"aaaa").await;
            let mut scanner =
                CollectionScanner::new(config("d", "coll", &glob_for(&dir), 0), ts(1000))
                    .await
                    .unwrap();

            write_file(&file, b"bbbbbbbb").await;
            // first pass: the file is promoted to a candidate.
            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert_eq!(scanner.state.candidates.len(), 1);

            // second pass: the file is already a candidate => not added again.
            scanner.discover_candidates(ts(1002)).await.unwrap();
            assert_eq!(scanner.state.candidates.len(), 1);
        }

        // A file whose on-disk size+mtime match the latest ledger entry is skipped
        // at discovery (already reported, untouched).
        #[tokio::test]
        async fn discovery_skips_latest_ledger_entry() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "led.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            // build a ledger entry from the file's actual observed metadata.
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let mut entry = stable_file(file.clone(), ts(900));
            entry.size = obs.size;
            entry.mtime = DateTime::<Utc>::from(obs.mtime);
            state.ledger.insert(file.clone(), vec![entry]);
            let mut scanner = CollectionScanner::from_state(state);

            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
        }
    }

    // ========================= M4: STABILITY WINDOWS ========================== //

    mod stability_window {
        use super::*;

        #[tokio::test]
        async fn elapsed_at_boundary() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "w.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);

            assert!(has_stability_window_elapsed(&state, &cand, ts(1010)).unwrap());
        }

        #[tokio::test]
        async fn not_elapsed_one_second_short() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "w.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);

            assert!(!has_stability_window_elapsed(&state, &cand, ts(1009)).unwrap());
        }

        #[tokio::test]
        async fn window_zero_at_first_timestamp() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "w.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);
            assert!(has_stability_window_elapsed(&state, &cand, ts(1000)).unwrap());
        }
    }

    mod is_metadata_stable {
        use super::*;

        // external crates
        use std::time::Duration;

        fn base_observation(file: File) -> Observation {
            Observation {
                file,
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                upload_rule_id: "coll".to_string(),
            }
        }

        #[test]
        fn unchanged() {
            let file = File::new("/none/x.mcap");
            let base = base_observation(file.clone());
            let cand = candidate(file, base.clone());
            assert!(is_metadata_stable(&cand, &base).unwrap());
        }

        #[test]
        fn size_differs() {
            let file = File::new("/none/x.mcap");
            let base = base_observation(file.clone());
            let cand = candidate(file, base.clone());
            let mut diff_size = base;
            diff_size.size = 5;
            assert!(!is_metadata_stable(&cand, &diff_size).unwrap());
        }

        #[test]
        fn mtime_differs() {
            let file = File::new("/none/x.mcap");
            let base = base_observation(file.clone());
            let cand = candidate(file, base.clone());
            let mut diff_mtime = base;
            diff_mtime.mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
            assert!(!is_metadata_stable(&cand, &diff_mtime).unwrap());
        }
    }

    // ===================== M5: find_previous_stable_file ====================== //

    mod find_previous_stable_file {
        use super::*;

        #[test]
        fn absent() {
            let file = File::new("/none/s.mcap");
            let state = State::new(config("d", "coll", "/none/*.mcap", 0));
            let cand = Candidate {
                file: file.clone(),
                observations: vec![],
            };
            assert_eq!(super::find_previous_stable_file(&state, &cand), None);
        }

        #[test]
        fn returns_last_ledger_entry() {
            let file = File::new("/none/s.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            let first = stable_file(file.clone(), ts(900));
            let last = stable_file(file.clone(), ts(1000));
            state.ledger.insert(file.clone(), vec![first, last.clone()]);
            let cand = Candidate {
                file,
                observations: vec![],
            };
            assert_eq!(super::find_previous_stable_file(&state, &cand), Some(last));
        }
    }

    // ======================= M6: differs_from_previous ======================== //
    mod differs_from_previous {
        use super::*;

        #[tokio::test]
        async fn no_previous() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs.clone());

            let outcome = super::differs_from_previous(&state, &cand).await.unwrap();
            assert!(matches!(outcome, StabilityOutcome::Stable(digest) if digest == HASH_AAAA));
        }

        // Same size + same digest as the latest ledger entry => already reported.
        #[tokio::test]
        async fn previous_same_size_dedup() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(900))]);

            let outcome = super::differs_from_previous(&state, &cand).await.unwrap();
            assert!(matches!(outcome, StabilityOutcome::AlreadyInLedger));
        }

        // A different-size new version is now reported (the HIGH-bug regression):
        // size no longer gates, the digest change decides.
        #[tokio::test]
        async fn previous_size_changed_reports_new_version() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            let mut prev = stable_file(file.clone(), ts(900));
            prev.size = 99;
            prev.digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string();
            state.ledger.insert(file, vec![prev]);

            let outcome = super::differs_from_previous(&state, &cand).await.unwrap();
            assert!(matches!(outcome, StabilityOutcome::Stable(digest) if digest == HASH_AAAA));
        }

        // Same digest as the latest ledger entry => already reported.
        #[tokio::test]
        async fn previous_same_digest_dedup() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            let mut prev = stable_file(file.clone(), ts(900));
            prev.digest = HASH_AAAA.to_string();
            state.ledger.insert(file, vec![prev]);

            let outcome = super::differs_from_previous(&state, &cand).await.unwrap();
            assert!(matches!(outcome, StabilityOutcome::AlreadyInLedger));
        }

        // A different digest than the latest ledger entry => new version reported.
        #[tokio::test]
        async fn previous_different_digest_reports_new_version() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            let mut prev = stable_file(file.clone(), ts(900));
            prev.digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string();
            state.ledger.insert(file, vec![prev]);

            let outcome = super::differs_from_previous(&state, &cand).await.unwrap();
            assert!(matches!(outcome, StabilityOutcome::Stable(digest) if digest == HASH_AAAA));
        }
    }

    // =================== M7: determine_stability / outcome =================== //

    mod determine_stability {
        use super::*;

        // external crates
        use std::time::Duration;

        #[tokio::test]
        async fn short_circuits_on_size_change() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs.clone());

            let mut changed = obs;
            changed.size += 1;
            let outcome = super::determine_stability(&state, &cand, &changed)
                .await
                .unwrap();
            assert!(matches!(outcome, StabilityOutcome::Unstable));
        }

        #[tokio::test]
        async fn short_circuits_on_mtime_change() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs.clone());

            let mut changed = obs;
            changed.mtime += Duration::from_secs(1);
            let outcome = super::determine_stability(&state, &cand, &changed)
                .await
                .unwrap();
            assert!(matches!(outcome, StabilityOutcome::Unstable));
        }

        #[tokio::test]
        async fn no_previous_stable() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs.clone());

            let outcome = super::determine_stability(&state, &cand, &obs)
                .await
                .unwrap();
            assert!(matches!(outcome, StabilityOutcome::Stable(digest) if digest == HASH_AAAA));
        }

        #[tokio::test]
        async fn previous_matching_digest_is_already_in_ledger() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(900))]);

            let outcome = super::determine_stability(&state, &cand, &obs)
                .await
                .unwrap();
            assert!(matches!(outcome, StabilityOutcome::AlreadyInLedger));
        }

        #[tokio::test]
        async fn previous_different_digest_reports_new_version() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "s.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file.clone(), obs.clone());
            let mut prev = stable_file(file.clone(), ts(900));
            prev.digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string();
            state.ledger.insert(file, vec![prev]);

            let outcome = super::determine_stability(&state, &cand, &obs)
                .await
                .unwrap();
            assert!(matches!(outcome, StabilityOutcome::Stable(digest) if digest == HASH_AAAA));
        }
    }

    // ============== M8: eval_candidate / evaluate_candidates =================== //

    mod eval_candidate_fn {
        use super::*;

        // A stable candidate produces a StableFile with the expected payload.
        #[tokio::test]
        async fn stable_payload_fields() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "e.mcap", b"aaaa").await;
            let state = State::new(config("dpl-1", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let expected = StableFile {
                file: file.clone(),
                size: 4,
                digest: HASH_AAAA.to_string(),
                mtime: DateTime::<Utc>::from(obs.mtime),
                mtime_aliases: vec![],
                first_observed_at: ts(1000),
                last_observed_at: ts(1000),
                deployment_id: "dpl-1".to_string(),
                upload_rule_id: "coll".to_string(),
            };
            let cand = candidate(file, obs);

            let action = eval_candidate(&state, &cand, ts(1010)).await.unwrap();
            let EvalAction::Stable(sf) = action else {
                panic!("expected Stable");
            };
            assert_eq!(sf, expected);
        }

        // A candidate whose file was deleted hits the Unstable (deleted) branch —
        // reachable directly but awkward through the actor.
        #[tokio::test]
        async fn deleted_file_is_unstable() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "gone.mcap", b"aaaa").await;
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
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "wait.mcap", b"aaaa").await;
            let state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let cand = candidate(file, obs);

            let action = eval_candidate(&state, &cand, ts(1009)).await.unwrap();
            assert!(matches!(action, EvalAction::WaitForStabilityWindow));
        }

        // Re-evaluating a file already in the ledger with identical content emits
        // nothing, leaves the ledger history unchanged, retires the candidate, and
        // records the fresh mtime as an alias on the latest ledger entry.
        #[tokio::test]
        async fn evaluate_dedups_unchanged_ledgered_file() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "ins.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            // prior ledger entry: same size + digest as on-disk `b"aaaa"`.
            let prior = stable_file(file.clone(), ts(900));
            state.ledger.insert(file.clone(), vec![prior]);
            state
                .candidates
                .insert(file.clone(), candidate(file.clone(), obs.clone()));
            let mut scanner = CollectionScanner::from_state(state);

            let stable = scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert!(stable.is_empty());
            assert_eq!(scanner.ledger_count(), 1);
            assert_eq!(scanner.state.ledger.get(&file).unwrap().len(), 1);
            assert!(!scanner.state.candidates.contains_key(&file));
            let latest = scanner.state.ledger.get(&file).unwrap().last().unwrap();
            assert!(latest
                .mtime_aliases
                .contains(&DateTime::<Utc>::from(obs.mtime)));
        }

        // evaluate_candidates drops an Unstable (deleted) candidate: nothing
        // reported, the candidate removed, the ledger untouched.
        #[tokio::test]
        async fn evaluate_drops_unstable() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "drop.mcap", b"aaaa").await;
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

    // =============================== M9: LEDGER =============================== //

    mod ledger {
        use super::*;

        // Two distinct stable files yield a ledger count of two.
        #[tokio::test]
        async fn ledger_count_two_distinct() {
            let dir = dirs::temp("testing").unwrap();
            let f1 = write(&dir, "one.mcap", b"aaaa").await;
            let f2 = write(&dir, "two.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let o1 = observation(&state, f1.clone(), ts(1000)).await;
            let o2 = observation(&state, f2.clone(), ts(1000)).await;
            state.candidates.insert(f1.clone(), candidate(f1, o1));
            state.candidates.insert(f2.clone(), candidate(f2, o2));
            let mut scanner = CollectionScanner::from_state(state);

            scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert_eq!(scanner.ledger_count(), 2);
        }

        // A ledgered file is reported once: the first tick emits it and retires the
        // candidate; re-promoting the same content and ticking again emits nothing
        // and does not grow the ledger history.
        #[tokio::test]
        async fn reevaluate_dedups_unchanged_file() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "dedup.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            state
                .candidates
                .insert(file.clone(), candidate(file.clone(), obs.clone()));
            let mut scanner = CollectionScanner::from_state(state);

            let first = scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert_eq!(first.len(), 1);
            assert_eq!(scanner.state.ledger.get(&file).unwrap().len(), 1);
            // reported-once: the candidate is retired after the first emit.
            assert!(!scanner.state.candidates.contains_key(&file));

            // re-promote the same (unchanged) content and tick again.
            scanner
                .state
                .candidates
                .insert(file.clone(), candidate(file.clone(), obs));
            let second = scanner.evaluate_candidates(ts(1020)).await.unwrap();
            assert!(second.is_empty());
            assert_eq!(scanner.ledger_count(), 1);
            assert_eq!(scanner.state.ledger.get(&file).unwrap().len(), 1);
            assert!(!scanner.state.candidates.contains_key(&file));
        }
    }

    // ===================== M10: DEDUP / CHANGED VERSIONS ====================== //

    mod dedup {
        use super::*;

        // A file reported once via the full discover -> evaluate cycle is not
        // re-discovered or re-emitted on later ticks; its ledger stays length 1.
        #[tokio::test]
        async fn static_reported_file_goes_quiet_across_ticks() {
            let dir = dirs::temp("testing").unwrap();
            write(&dir, "static.mcap", b"aaaa").await;
            // window 0 so the appearing file is immediately stable, but note it is
            // preexisting at creation — create the scanner first, then the file.
            let cfg = config("d", "coll", &glob_for(&dir), 0);
            let mut scanner = CollectionScanner::new(cfg, ts(1000)).await.unwrap();

            let file = write(&dir, "new.mcap", b"aaaa").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            let first = scanner.evaluate_candidates(ts(1001)).await.unwrap();
            assert_eq!(first.len(), 1);
            assert_eq!(scanner.ledger_count(), 1);
            assert!(!scanner.state.candidates.contains_key(&file));

            // later tick: the untouched file is skipped at discovery, emits nothing.
            scanner.discover_candidates(ts(1002)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
            let second = scanner.evaluate_candidates(ts(1002)).await.unwrap();
            assert!(second.is_empty());
            assert_eq!(scanner.state.ledger.get(&file).unwrap().len(), 1);
        }

        // A touched-but-unchanged file (mtime moved, size+digest identical) is
        // evaluated once, retired, and its new mtime recorded as an alias on the
        // latest ledger entry; a later discovery with that aliased mtime is skipped.
        #[tokio::test]
        async fn touched_unchanged_file_records_mtime_alias_then_quiet() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "touch.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            // The candidate is observed from disk (real, non-deterministic mtime);
            // the ledger entry deliberately carries an OLD mtime so the file's real
            // mtime differs from it. Content (size + digest) is identical, so the
            // outcome is AlreadyInLedger and the real mtime is recorded as an alias.
            let base = observation(&state, file.clone(), ts(1000)).await;
            let mut entry = stable_file(file.clone(), ts(900));
            entry.size = base.size;
            entry.mtime = ts(500);
            state.ledger.insert(file.clone(), vec![entry]);
            state
                .candidates
                .insert(file.clone(), candidate(file.clone(), base.clone()));
            let mut scanner = CollectionScanner::from_state(state);

            let stable = scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert!(stable.is_empty());
            assert!(!scanner.state.candidates.contains_key(&file));
            let latest = scanner.state.ledger.get(&file).unwrap().last().unwrap();
            assert!(latest
                .mtime_aliases
                .contains(&DateTime::<Utc>::from(base.mtime)));

            // a subsequent discovery observing the aliased mtime is skipped.
            assert!(scanner.state.is_latest_ledger_entry(&base));
        }

        // Changed content at a DIFFERENT size is reported as a new version — the
        // HIGH-bug regression that previously looped Unstable forever.
        #[tokio::test]
        async fn changed_content_different_size_reports_new_version() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "grow.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            // ledger entry for the original 4-byte content.
            let orig = observation(&state, file.clone(), ts(1000)).await;
            let mut entry = stable_file(file.clone(), ts(900));
            entry.size = orig.size;
            entry.mtime = DateTime::<Utc>::from(orig.mtime);
            state.ledger.insert(file.clone(), vec![entry]);

            // rewrite to a larger, different-digest content and re-promote.
            write_file(&file, b"bbbbbbbb").await;
            let changed = observation(&state, file.clone(), ts(1001)).await;
            state
                .candidates
                .insert(file.clone(), candidate(file.clone(), changed));
            let mut scanner = CollectionScanner::from_state(state);

            let stable = scanner.evaluate_candidates(ts(1011)).await.unwrap();
            assert_eq!(stable.len(), 1);
            assert_eq!(scanner.state.ledger.get(&file).unwrap().len(), 2);
        }

        // Changed content at the SAME size (different digest) is reported as a new
        // version.
        #[tokio::test]
        async fn changed_content_same_size_reports_new_version() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "same.mcap", b"aaaa").await;
            let mut state = State::new(config("d", "coll", &glob_for(&dir), 0));
            let orig = observation(&state, file.clone(), ts(1000)).await;
            let mut entry = stable_file(file.clone(), ts(900));
            entry.size = orig.size;
            entry.mtime = DateTime::<Utc>::from(orig.mtime);
            state.ledger.insert(file.clone(), vec![entry]);

            // rewrite to same-size but different-digest content and re-promote.
            write_file(&file, b"bbbb").await;
            let changed = observation(&state, file.clone(), ts(1001)).await;
            assert_eq!(changed.size, orig.size);
            state
                .candidates
                .insert(file.clone(), candidate(file.clone(), changed));
            let mut scanner = CollectionScanner::from_state(state);

            let stable = scanner.evaluate_candidates(ts(1011)).await.unwrap();
            assert_eq!(stable.len(), 1);
            assert_eq!(scanner.state.ledger.get(&file).unwrap().len(), 2);
        }
    }

    // CollectionScanner::prune_ledger is a pass-through to State::prune_ledger
    // (behavior covered in state tests). These only assert the hook is wired.
    mod prune_ledger {
        use super::*;

        #[test]
        fn retains_at_cutoff() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            let mut scanner = CollectionScanner::from_state(state);

            scanner.prune_ledger(ts(1000)).unwrap();
            assert_eq!(scanner.ledger_count(), 1);
        }

        #[test]
        fn drops_after_cutoff() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            let mut scanner = CollectionScanner::from_state(state);

            scanner.prune_ledger(ts(1001)).unwrap();
            assert_eq!(scanner.ledger_count(), 0);
        }
    }
}
