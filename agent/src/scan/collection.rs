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
    use crate::filesys::{dirs, dirs::TempDir, Dir, PathExt, WriteOptions};
    use crate::models::{Deployment, UploadRule, UploadRuleSource};
    use crate::scan::state::{Candidate, Config, Observation, StableFile, State};

    // external crates
    use std::time::SystemTime;

    // ================================ FIXTURES =================================== //

    /// SHA-256 of `b"aaaa"` — the digest `files::hash` returns for that content.
    const HASH_AAAA: &str =
        "sha256:61be55a8e2f6b4e172338bddf184d6dbee29c98853e0a0485ecee7f27b9af0b4";

    /// SHA-256 of `b"bbbb"` — the digest `files::hash` returns for that content.
    const HASH_BBBB: &str =
        "sha256:81cc5b17018674b401b42f35ba07bb79e211239c23bffe658da1577e3e646877";

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

    /// A StableFile built from an observation's metadata and a known digest.
    fn stable_from_obs(
        obs: &Observation,
        digest: &str,
        first_observed_at: DateTime<Utc>,
    ) -> StableFile {
        StableFile {
            file: obs.file.clone(),
            size: obs.size,
            digest: digest.to_string(),
            mtime: DateTime::<Utc>::from(obs.mtime),
            mtime_aliases: vec![],
            first_observed_at,
            last_observed_at: obs.timestamp,
            deployment_id: obs.deployment_id.clone(),
            upload_rule_id: obs.upload_rule_id.clone(),
        }
    }

    /// A ledger entry whose size/mtime match `obs` (digest left as the `aaaa` default).
    fn ledger_entry_matching(obs: &Observation, first_observed_at: DateTime<Utc>) -> StableFile {
        let mut entry = stable_file(obs.file.clone(), first_observed_at);
        entry.size = obs.size;
        entry.mtime = DateTime::<Utc>::from(obs.mtime);
        entry
    }

    /// A prior ledger entry with an overridden digest (first_observed_at = ts(900)).
    fn prior_with_digest(file: File, digest: &str) -> StableFile {
        let mut prev = stable_file(file, ts(900));
        prev.digest = digest.to_string();
        prev
    }

    /// Clone `entry` with `mtime` appended to `mtime_aliases`.
    fn with_mtime_alias(mut entry: StableFile, mtime: SystemTime) -> StableFile {
        entry.mtime_aliases.push(DateTime::<Utc>::from(mtime));
        entry
    }

    /// Insert a single ledger history for `file`.
    fn seed_ledger(state: &mut State, file: &File, entry: StableFile) {
        state.ledger.insert(file.clone(), vec![entry]);
    }

    /// Insert `file` as a tracked candidate under `obs`.
    fn track(state: &mut State, file: &File, obs: Observation) {
        state
            .candidates
            .insert(file.clone(), candidate(file.clone(), obs));
    }

    /// Assert the candidate was retired and the file's ledger history matches.
    fn assert_retired_with_ledger(
        scanner: &CollectionScanner,
        file: &File,
        history: Vec<StableFile>,
    ) {
        assert!(!scanner.state.candidates.contains_key(file));
        assert_eq!(scanner.state.ledger.get(file), Some(&history));
    }

    fn assert_stable(outcome: StabilityOutcome) {
        assert!(matches!(outcome, StabilityOutcome::Stable(digest) if digest == HASH_AAAA));
    }

    fn assert_already_in_ledger(outcome: StabilityOutcome) {
        assert!(matches!(outcome, StabilityOutcome::AlreadyInLedger));
    }

    fn assert_unstable(outcome: StabilityOutcome) {
        assert!(matches!(outcome, StabilityOutcome::Unstable));
    }

    /// Temp dir + `aaaa` file + state/obs/candidate at ts(1000).
    /// Holds `dir` so the temp tree stays alive for the test.
    struct Case {
        dir: TempDir,
        file: File,
        state: State,
        obs: Observation,
        cand: Candidate,
    }

    async fn case(name: &str, window: i32) -> Case {
        let dir = dirs::temp("testing").unwrap();
        let file = write(&dir, name, b"aaaa").await;
        let state = State::new(config("d", "coll", &glob_for(&dir), window));
        let obs = observation(&state, file.clone(), ts(1000)).await;
        let cand = candidate(file.clone(), obs.clone());
        Case {
            dir,
            file,
            state,
            obs,
            cand,
        }
    }

    /// Config with the same collection id but rule id `r2` (for update_config tests).
    fn config_v2(glob: &str) -> Config {
        let mut v2 = rule("coll", glob, 0);
        v2.id = "r2".to_string();
        Config {
            deployment: deployment("d"),
            rule: v2,
        }
    }

    /// Empty state for `cfg` with `file` snapshotted into preexisting at `at`.
    async fn state_with_preexisting(cfg: Config, file: File, at: DateTime<Utc>) -> State {
        let mut state = State::new(cfg);
        let obs = observation(&state, file.clone(), at).await;
        state.preexisting.insert(file, obs);
        state
    }

    async fn scanner_new(dir: &Dir, window: i32, at: DateTime<Utc>) -> CollectionScanner {
        CollectionScanner::new(config("d", "coll", &glob_for(dir), window), at)
            .await
            .unwrap()
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

            let expected_state = state_with_preexisting(cfg, file, ts(1000)).await;
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
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;
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
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;

            let late_file = write(&dir, "late.mcap", b"aaaa").await;
            let cfg = config_v2(&glob_for(&dir));
            scanner.update_config(cfg.clone(), ts(1001)).await.unwrap();

            let expected_state = state_with_preexisting(cfg, late_file, ts(1001)).await;
            assert_eq!(scanner.state, expected_state);

            scanner.discover_candidates(ts(1002)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
            assert_eq!(scanner.state, expected_state);
        }

        // update_config leaves tracked candidates in place; they are not moved into preexisting.
        #[tokio::test]
        async fn update_config_preserves_candidates() {
            let dir = dirs::temp("testing").unwrap();
            let candidate_file = write(&dir, "pre.mcap", b"aaaa").await;
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;

            write_file(&candidate_file, b"bbbbbbbb").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            let expected_candidate = scanner
                .state
                .candidates
                .get(&candidate_file)
                .cloned()
                .expect("changed preexisting file promoted to candidate");

            let cfg = config_v2(&glob_for(&dir));
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
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;

            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
        }

        // A preexisting file whose size changed is promoted to a candidate.
        #[tokio::test]
        async fn changed_preexisting_promoted() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "pre.mcap", b"aaaa").await;
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;

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
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;

            write_file(&file, b"bbbbbbbb").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            assert_eq!(scanner.state.candidates.len(), 1);

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
            let obs = observation(&state, file.clone(), ts(1000)).await;
            seed_ledger(&mut state, &file, ledger_entry_matching(&obs, ts(900)));
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
            let c = case("w.mcap", 10).await;
            assert!(has_stability_window_elapsed(&c.state, &c.cand, ts(1010)).unwrap());
        }

        #[tokio::test]
        async fn not_elapsed_one_second_short() {
            let c = case("w.mcap", 10).await;
            assert!(!has_stability_window_elapsed(&c.state, &c.cand, ts(1009)).unwrap());
        }

        #[tokio::test]
        async fn window_zero_at_first_timestamp() {
            let c = case("w.mcap", 0).await;
            assert!(has_stability_window_elapsed(&c.state, &c.cand, ts(1000)).unwrap());
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
            let c = case("s.mcap", 0).await;
            assert_stable(
                super::differs_from_previous(&c.state, &c.cand)
                    .await
                    .unwrap(),
            );
        }

        #[tokio::test]
        async fn previous_same_digest_dedup() {
            let mut c = case("s.mcap", 0).await;
            seed_ledger(
                &mut c.state,
                &c.file,
                prior_with_digest(c.file.clone(), HASH_AAAA),
            );
            assert_already_in_ledger(
                super::differs_from_previous(&c.state, &c.cand)
                    .await
                    .unwrap(),
            );
        }

        #[tokio::test]
        async fn previous_different_digest_reports_new_version() {
            let mut c = case("s.mcap", 0).await;
            seed_ledger(
                &mut c.state,
                &c.file,
                prior_with_digest(
                    c.file.clone(),
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                ),
            );
            assert_stable(
                super::differs_from_previous(&c.state, &c.cand)
                    .await
                    .unwrap(),
            );
        }

        // Same size + same digest as the latest ledger entry => already reported.
        #[tokio::test]
        async fn previous_same_size_dedup() {
            let mut c = case("s.mcap", 0).await;
            seed_ledger(&mut c.state, &c.file, stable_file(c.file.clone(), ts(900)));
            assert_already_in_ledger(
                super::differs_from_previous(&c.state, &c.cand)
                    .await
                    .unwrap(),
            );
        }

        // Size no longer gates: a different-size prior with a different digest still
        // reports a new version (digest change decides).
        #[tokio::test]
        async fn previous_size_changed_reports_new_version() {
            let mut c = case("s.mcap", 0).await;
            let mut prev = prior_with_digest(
                c.file.clone(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            );
            prev.size = 99;
            seed_ledger(&mut c.state, &c.file, prev);
            assert_stable(
                super::differs_from_previous(&c.state, &c.cand)
                    .await
                    .unwrap(),
            );
        }
    }

    // =================== M7: determine_stability / outcome =================== //

    mod determine_stability {
        use super::*;

        // external crates
        use std::time::Duration;

        // Metadata short-circuits are owned here; digest/ledger outcomes are covered
        // by differs_from_previous. Keep one happy-path wiring check.
        #[tokio::test]
        async fn short_circuits_on_size_change() {
            let c = case("s.mcap", 0).await;
            let mut changed = c.obs.clone();
            changed.size += 1;
            assert_unstable(
                super::determine_stability(&c.state, &c.cand, &changed)
                    .await
                    .unwrap(),
            );
        }

        #[tokio::test]
        async fn short_circuits_on_mtime_change() {
            let c = case("s.mcap", 0).await;
            let mut changed = c.obs.clone();
            changed.mtime += Duration::from_secs(1);
            assert_unstable(
                super::determine_stability(&c.state, &c.cand, &changed)
                    .await
                    .unwrap(),
            );
        }

        #[tokio::test]
        async fn no_previous_stable() {
            let c = case("s.mcap", 0).await;
            assert_stable(
                super::determine_stability(&c.state, &c.cand, &c.obs)
                    .await
                    .unwrap(),
            );
        }

        #[tokio::test]
        async fn previous_matching_digest_is_already_in_ledger() {
            let mut c = case("s.mcap", 0).await;
            seed_ledger(&mut c.state, &c.file, stable_file(c.file.clone(), ts(900)));
            assert_already_in_ledger(
                super::determine_stability(&c.state, &c.cand, &c.obs)
                    .await
                    .unwrap(),
            );
        }

        #[tokio::test]
        async fn previous_different_digest_reports_new_version() {
            let mut c = case("s.mcap", 0).await;
            seed_ledger(
                &mut c.state,
                &c.file,
                prior_with_digest(
                    c.file.clone(),
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                ),
            );
            assert_stable(
                super::determine_stability(&c.state, &c.cand, &c.obs)
                    .await
                    .unwrap(),
            );
        }
    }

    // ============== M8: eval_candidate / evaluate_candidates =================== //

    mod eval_candidate_fn {
        use super::*;

        // Before the window elapses, evaluation waits.
        #[tokio::test]
        async fn not_yet_elapsed_waits() {
            let c = case("wait.mcap", 10).await;
            let action = eval_candidate(&c.state, &c.cand, ts(1009)).await.unwrap();
            assert!(matches!(action, EvalAction::WaitForStabilityWindow));
        }

        // A candidate whose file was deleted hits the Unstable (deleted) branch —
        // reachable directly but awkward through the actor.
        #[tokio::test]
        async fn deleted_file_is_unstable() {
            let c = case("gone.mcap", 10).await;
            files::delete(&c.file).await.unwrap();
            let action = eval_candidate(&c.state, &c.cand, ts(1010)).await.unwrap();
            assert!(matches!(action, EvalAction::Unstable));
        }

        // A stable candidate produces a StableFile with the expected payload.
        #[tokio::test]
        async fn stable_payload_fields() {
            let dir = dirs::temp("testing").unwrap();
            let file = write(&dir, "e.mcap", b"aaaa").await;
            let state = State::new(config("dpl-1", "coll", &glob_for(&dir), 10));
            let obs = observation(&state, file.clone(), ts(1000)).await;
            let expected = stable_from_obs(&obs, HASH_AAAA, ts(1000));
            let cand = candidate(file, obs);

            let action = eval_candidate(&state, &cand, ts(1010)).await.unwrap();
            let EvalAction::Stable(sf) = action else {
                panic!("expected Stable");
            };
            assert_eq!(sf, expected);
        }
    }

    mod eval_candidate_method {
        use super::*;

        // Re-evaluating a file already in the ledger with identical content emits
        // nothing, leaves the ledger history unchanged, retires the candidate, and
        // records the fresh mtime as an alias on the latest ledger entry.
        #[tokio::test]
        async fn evaluate_dedups_unchanged_ledgered_file() {
            let mut c = case("ins.mcap", 10).await;
            let prior = stable_file(c.file.clone(), ts(900));
            let expected = with_mtime_alias(prior.clone(), c.obs.mtime);
            seed_ledger(&mut c.state, &c.file, prior);
            track(&mut c.state, &c.file, c.obs.clone());
            let file = c.file.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert!(scanner
                .evaluate_candidates(ts(1010))
                .await
                .unwrap()
                .is_empty());
            assert_retired_with_ledger(&scanner, &file, vec![expected]);
        }

        // A new Stable version appends onto existing ledger history: the prior
        // entry is preserved, the candidate is retired, and history is [prior, new].
        #[tokio::test]
        async fn evaluate_appends_new_version_preserving_prior() {
            let mut c = case("ver.mcap", 10).await;
            let prior = stable_file(c.file.clone(), ts(900));
            seed_ledger(&mut c.state, &c.file, prior.clone());

            write_file(&c.file, b"bbbb").await;
            let changed = observation(&c.state, c.file.clone(), ts(1000)).await;
            let expected = stable_from_obs(&changed, HASH_BBBB, ts(1000));
            track(&mut c.state, &c.file, changed);
            let file = c.file.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert_eq!(
                scanner.evaluate_candidates(ts(1010)).await.unwrap(),
                vec![expected.clone()]
            );
            assert_retired_with_ledger(&scanner, &file, vec![prior, expected]);
        }

        // evaluate_candidates drops an Unstable (deleted) candidate: nothing
        // reported, the candidate removed, the ledger untouched.
        #[tokio::test]
        async fn evaluate_drops_unstable() {
            let mut c = case("drop.mcap", 10).await;
            track(&mut c.state, &c.file, c.obs.clone());
            files::delete(&c.file).await.unwrap();
            let file = c.file.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert!(scanner
                .evaluate_candidates(ts(1010))
                .await
                .unwrap()
                .is_empty());
            assert!(!scanner.state.candidates.contains_key(&file));
            assert!(scanner.state.ledger.is_empty());
        }

        // A ledgered file is reported once: the first tick emits it and retires the
        // candidate; re-promoting the same content and ticking again emits nothing
        // and does not grow the ledger history.
        #[tokio::test]
        async fn reevaluate_dedups_unchanged_file() {
            let mut c = case("dedup.mcap", 10).await;
            let expected = stable_from_obs(&c.obs, HASH_AAAA, ts(1000));
            track(&mut c.state, &c.file, c.obs.clone());
            let file = c.file.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert_eq!(
                scanner.evaluate_candidates(ts(1010)).await.unwrap(),
                vec![expected.clone()]
            );
            assert_retired_with_ledger(&scanner, &file, vec![expected.clone()]);

            // re-promote the same (unchanged) content and tick again.
            // AlreadyInLedger records the observation mtime as an alias.
            track(&mut scanner.state, &file, c.obs.clone());
            assert!(scanner
                .evaluate_candidates(ts(1020))
                .await
                .unwrap()
                .is_empty());
            assert_retired_with_ledger(
                &scanner,
                &file,
                vec![with_mtime_alias(expected, c.obs.mtime)],
            );
        }

        // A file reported once via the full discover -> evaluate cycle is not
        // re-discovered or re-emitted on later ticks; its ledger stays length 1.
        #[tokio::test]
        async fn static_reported_file_goes_quiet_across_ticks() {
            let dir = dirs::temp("testing").unwrap();
            write(&dir, "static.mcap", b"aaaa").await;
            // window 0 so the appearing file is immediately stable, but note it is
            // preexisting at creation — create the scanner first, then the file.
            let mut scanner = scanner_new(&dir, 0, ts(1000)).await;

            let file = write(&dir, "new.mcap", b"aaaa").await;
            scanner.discover_candidates(ts(1001)).await.unwrap();
            let first = scanner.evaluate_candidates(ts(1001)).await.unwrap();
            assert_eq!(first.len(), 1);
            assert_retired_with_ledger(&scanner, &file, first.clone());

            // later tick: the untouched file is skipped at discovery, emits nothing.
            scanner.discover_candidates(ts(1002)).await.unwrap();
            assert!(scanner.state.candidates.is_empty());
            assert!(scanner
                .evaluate_candidates(ts(1002))
                .await
                .unwrap()
                .is_empty());
            assert_eq!(scanner.state.ledger.get(&file), Some(&first));
        }

        // A touched-but-unchanged file (mtime moved, size+digest identical) is
        // evaluated once, retired, and its new mtime recorded as an alias on the
        // latest ledger entry; a later discovery with that aliased mtime is skipped.
        #[tokio::test]
        async fn touched_unchanged_file_records_mtime_alias_then_quiet() {
            let mut c = case("touch.mcap", 0).await;
            // The candidate is observed from disk (real, non-deterministic mtime);
            // the ledger entry deliberately carries an OLD mtime so the file's real
            // mtime differs from it. Content (size + digest) is identical, so the
            // outcome is AlreadyInLedger and the real mtime is recorded as an alias.
            let mut entry = ledger_entry_matching(&c.obs, ts(900));
            entry.mtime = ts(500);
            let expected = with_mtime_alias(entry.clone(), c.obs.mtime);
            seed_ledger(&mut c.state, &c.file, entry);
            track(&mut c.state, &c.file, c.obs.clone());
            let file = c.file.clone();
            let base = c.obs.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert!(scanner
                .evaluate_candidates(ts(1010))
                .await
                .unwrap()
                .is_empty());
            assert_retired_with_ledger(&scanner, &file, vec![expected]);
            assert!(scanner.state.is_latest_ledger_entry(&base));
        }

        // Changed content at a DIFFERENT size is reported as a new version.
        #[tokio::test]
        async fn changed_content_different_size_reports_new_version() {
            let mut c = case("grow.mcap", 0).await;
            let prior = ledger_entry_matching(&c.obs, ts(900));
            seed_ledger(&mut c.state, &c.file, prior.clone());

            write_file(&c.file, b"bbbbbbbb").await;
            let changed = observation(&c.state, c.file.clone(), ts(1001)).await;
            let expected =
                stable_from_obs(&changed, &files::hash(&c.file).await.unwrap(), ts(1001));
            track(&mut c.state, &c.file, changed);
            let file = c.file.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert_eq!(
                scanner.evaluate_candidates(ts(1011)).await.unwrap(),
                vec![expected.clone()]
            );
            assert_retired_with_ledger(&scanner, &file, vec![prior, expected]);
        }

        // Changed content at the SAME size (different digest) is reported as a new
        // version.
        #[tokio::test]
        async fn changed_content_same_size_reports_new_version() {
            let mut c = case("same.mcap", 0).await;
            let prior = ledger_entry_matching(&c.obs, ts(900));
            seed_ledger(&mut c.state, &c.file, prior.clone());

            write_file(&c.file, b"bbbb").await;
            let changed = observation(&c.state, c.file.clone(), ts(1001)).await;
            assert_eq!(changed.size, c.obs.size);
            let expected = stable_from_obs(&changed, HASH_BBBB, ts(1001));
            track(&mut c.state, &c.file, changed);
            let file = c.file.clone();
            let mut scanner = CollectionScanner::from_state(c.state);

            assert_eq!(
                scanner.evaluate_candidates(ts(1011)).await.unwrap(),
                vec![expected.clone()]
            );
            assert_retired_with_ledger(&scanner, &file, vec![prior, expected]);
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
            track(&mut state, &f1, o1);
            track(&mut state, &f2, o2);
            let mut scanner = CollectionScanner::from_state(state);

            scanner.evaluate_candidates(ts(1010)).await.unwrap();
            assert_eq!(scanner.ledger_count(), 2);
        }
    }

    // CollectionScanner::prune_ledger is a pass-through to State::prune_ledger
    // (behavior covered in state tests). These only assert the hook is wired.
    mod prune_ledger {
        use super::*;

        fn scanner_with_ledger_entry(at: DateTime<Utc>) -> CollectionScanner {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            seed_ledger(&mut state, &file, stable_file(file.clone(), at));
            CollectionScanner::from_state(state)
        }

        #[test]
        fn retains_at_cutoff() {
            let mut scanner = scanner_with_ledger_entry(ts(1000));
            scanner.prune_ledger(ts(1000)).unwrap();
            assert_eq!(scanner.ledger_count(), 1);
        }

        #[test]
        fn drops_after_cutoff() {
            let mut scanner = scanner_with_ledger_entry(ts(1000));
            scanner.prune_ledger(ts(1001)).unwrap();
            assert_eq!(scanner.ledger_count(), 0);
        }
    }
}
