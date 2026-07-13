# Persist the scanner state to disk using the StateFile abstraction

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Add serde derives to the scanner state types (`agent/src/scan/state.rs`, `agent/src/filesys/file.rs`), a `PersistedState` container + `ScanStateFile` alias, a `Layout::scanner_state()` path, and restore/persist wiring in `agent/src/scan/scanner.rs` and `agent/src/scan/collection.rs`. New tests are inline `#[cfg(test)]` tests in the touched modules. |
| `libs/` | untouched | Generated OpenAPI code; not involved. |

This plan lives in `plans/` of the agent repo because all changes are inside this repo. Work happens on branch `feat/persist-scanner-state` (already created from `main` at `206c2d0`).

## Purpose / Big Picture

The scanner (`agent/src/scan/`) watches glob patterns from upload rules, waits for matched files to become stable, and emits `StableFile` events for upload. Its entire memory — which files existed before a rule was deployed (`preexisting`), which files are mid-stability-window (`candidates`), and which stable versions were already reported (`ledger`) — lives only in RAM. Every agent restart wipes it: already-reported files get re-reported (duplicate uploads), and files that were mid-window at shutdown get re-classified as "preexisting" and are silently never uploaded.

After this change the scanner persists its per-collection state to a single JSON file (`/var/lib/miru/scanner_state.json` on a real device) through the existing `SingleThreadStateFile` abstraction (`agent/src/filesys/state_file.rs`, renamed from `CachedFile` in PR #136 as a prerequisite for this work). On restart the scanner restores that state when rules are re-deployed, so: (1) a file already reported before the restart is not re-reported, (2) a file that appeared while the agent was down under an unchanged rule is discovered and reported, and (3) a mid-window candidate survives the restart and is reported once its window elapses. A missing or corrupt state file is fail-open: the scanner starts fresh exactly as it does today.

The scanner is not yet driven from `agent/src/main.rs` (see the `#[allow(dead_code)]` notes in `agent/src/scan/mod.rs` — the driving worker lands in a separate PR), so all observable behavior in this plan is exercised through the existing actor API (`Scanner::spawn` → `update_rules`/`scan`/`shutdown`) in tests. `Layout::scanner_state()` is added now so the future driver PR has a canonical path.

## Progress

- [x] Milestone 1: serde derives + `PersistedState`/`ScanStateFile` types + unit tests; commit.
- [x] Milestone 2: `Layout::scanner_state()`, restore/persist wiring in the scanner, inline restart tests; commit.
- [ ] Milestone 3: local validation (build, test, lint) and CI green on the pushed head.

## Surprises & Discoveries

- The local default rustc (1.94.0) can no longer build the lockfile (`aws-types@1.4.0` requires 1.94.1); all local builds/tests ran with `RUSTUP_TOOLCHAIN=1.97.0`. Pre-existing environment gap, unrelated to this change.
- `gh` turned out to be available in the execution environment, so CI is watched via `gh pr checks` rather than the GitHub MCP tools the plan anticipated.
- `SingleThreadStateFile::new_with_default` writes the default to disk at construction on any read failure, so `scanner_state.json` exists before the first scan — the missing-file test asserts existence but notes the earlier creation point.
- The anticipated ~15 root-permission test failures did not reproduce in this environment; the full suite ran 1702 passed / 0 failed.
- Milestone ordering deviation: source for both milestones was implemented and committed first (`feat(scan): make scanner state serializable`, `feat(scan): persist scanner state via StateFile`, plus a `style(scan)` import-linter fix from the refine pass), then all tests landed in one `test(scan)` commit — same content as the planned per-milestone commits, different slicing.
- Tests added beyond the plan's list (coverage-driven): `restore_rejects_collection_id_change` (the `set_config` error line in `restore` is unreachable via the actor), `Layout::scanner_state` path test, and `persist_failure_is_swallowed` (directory planted at the state-file path makes the atomic rename fail with EISDIR even as root, exercising the warn-and-continue arm).

## Decision Log

- Decision: Persist the entire per-collection `State` (all four fields: `cfg`, `preexisting`, `candidates`, `ledger`) rather than only the ledger.
  Rationale: the ledger alone is not enough — a mid-window candidate that is not persisted would be re-snapshotted as preexisting on restore and silently never uploaded; persisting `cfg` lets restore compare the stored rule digest against the incoming rule; persisting `preexisting` lets an unchanged rule skip the re-snapshot so files created during agent downtime are still captured. The whole struct is small metadata (no file contents). Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: One state file for the whole scanner (`PersistedState { collections: HashMap<UploadCollectionID, State> }` at `Layout::scanner_state()` = `/var/lib/miru/scanner_state.json`), not one file per collection.
  Rationale: all mutation already funnels through the single scanner actor, so one `SingleThreadStateFile` owner is natural; it avoids inventing file lifecycle management (create/delete per collection id) and matches the flat-file convention of `device.json` / `settings.json` at the layout root. Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: Serialize the in-memory types directly (serde derives on `State`, `Config`, `Observation`, `Candidate`, `StableFile`, and `#[serde(transparent)]` on `filesys::File`) instead of a parallel snapshot schema.
  Rationale: matches the existing pattern (`Token`, `models::Device` are persisted directly); a transparent `File` serializes as its `PathBuf`, which serde_json accepts as a JSON object key (string), so the `HashMap<File, _>` fields serialize as-is; `SystemTime` (in `Observation.mtime`) is natively serde-serializable. Schema drift in a future version fails deserialization and falls into the fail-open path — acceptable for reconstructible dedup state. Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: `PatchT = PersistedState` itself (full replacement: `impl Patch<PersistedState> for PersistedState { fn patch(&mut self, p) { *self = p } }`), written via `StateFile::patch`.
  Rationale: scanner mutations touch arbitrary subsets of the maps, so a field-level patch struct buys nothing; `patch()` already suppresses the disk write when the composed snapshot equals the current file content (its `PartialEq` check), which makes no-change scan ticks free. Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: Restore lazily at `update_rules` time (per collection id), not eagerly at spawn.
  Rationale: `SingleThreadScanner::scan` prunes any non-deployed scanner with no candidates; eagerly restored collections would be pruned by a scan tick that arrives before the first `update_rules`, destroying the just-restored ledger. Lazily, restored entries wait in a `restored: HashMap<UploadCollectionID, State>` side map (still included in every persisted snapshot) until their collection id is deployed again. Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: On restore, if the incoming rule's `digest` equals the persisted `cfg.rule.digest`, keep the persisted `preexisting` map (skip the re-snapshot); if it differs, re-run `discover_preexisting` exactly like `update_config` does today.
  Rationale: an agent restart under an unchanged rule must not suppress files written while the agent was down (the primary data-capture win of persistence); a changed rule keeps parity with the documented re-push semantics (`update_rules_resnapshots_preexisting` test in `agent/src/scan/scanner.rs`). Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: Persist at the end of `update_rules`, `scan`, and `prune`, plus once on `Shutdown`; persist failures are `warn!`-and-continue; `state_file: Option<ScanStateFile>` in `ScannerArgs` with `None` meaning in-memory-only (the current behavior, and the `Default`).
  Rationale: those are the only state-mutating commands (`clear_rules` only touches the non-persisted `deployed` set); a failed write must not stop scanning (degraded dedup beats no uploads); `Option` keeps every existing test and the `ScannerArgs::default()` construction unchanged. Fail-open reads come free from `SingleThreadStateFile::new_with_default`, which replaces a missing or unreadable file with the default. Known limitation (document, do not solve here): the ledger records "reported", not "uploaded" — a crash between emitting a `StableFile` event and the upload completing means that file is not re-emitted after restart. Durable upload acks are out of scope. Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: Do not modify any `.covgate` file. `agent/src/scan/.covgate` is 98.85 — meet it with tests, never ratchet it to a local number (local vs CI coverage differs in this repo).
  Rationale: repo policy; CI's covgate sees different coverage than local runs. Date/Author: 2026-07-12 / agents@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (Rust workspace; the binary crate is `miru-agent` under `agent/`). All paths are repo-root-relative; all commands run from the repo root. Read `AGENTS.md` first: import-group ordering (standard / internal / external), `./scripts/test.sh` wraps `RUST_LOG=off cargo test --features test` (never run bare `cargo test`), lint via `./scripts/update-deps.sh` then `./scripts/lint.sh`, per-directory `.covgate` coverage gates enforced by `scripts/covgate.sh`.

Key pieces:

- `agent/src/scan/state.rs` — `State { cfg: Config, preexisting: HashMap<File, Observation>, candidates: HashMap<File, Candidate>, ledger: HashMap<File, Vec<StableFile>> }` plus `Config { deployment: Deployment, rule: UploadRule }`, `Observation` (has `mtime: std::time::SystemTime`), `Candidate`, `StableFile`. None have serde derives today; `State` also lacks `Clone`.
- `agent/src/scan/collection.rs` — `CollectionScanner { state: State }` with `new(config, now)` (snapshots preexisting via the private `discover_preexisting`), `from_state(state)`, `update_config(config, now)` (calls `State::set_config`, which rejects a changed `upload_collection_id`, then re-snapshots preexisting), `discover_candidates`, `evaluate_candidates` (appends to `ledger`), `prune_ledger`. It exposes no way to get the `State` back out — persistence needs a state accessor.
- `agent/src/scan/scanner.rs` — `SingleThreadScanner { scanners: HashMap<UploadCollectionID, CollectionScanner>, deployed: HashSet<_>, now_fn, subscriber_tx }` plus the tokio actor (`Command`, `Worker`, `Scanner::spawn(buffer_size, args)`). `ScannerArgs { now_fn, broadcast_capacity }` with a `Default` impl. `scan()` evaluates all scanners, discovers for deployed ones, prunes non-deployed scanners with no candidates, then emits. Test-only accessors `get_rules` / `get_ledger_count` are `#[cfg(feature = "test")]`.
- `agent/src/filesys/state_file.rs` — `SingleThreadStateFile<ContentT, PatchT>` where `ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq`. `new(file)` fails on missing/corrupt JSON; `new_with_default(file, default)` falls back to atomically creating the file with `default` on ANY read error (this is the fail-open mechanism); `read()` returns `Arc<ContentT>` from memory; `write(data)` is an atomic whole-file write; `patch(p)` applies `Patch::patch` and skips the write when the content is unchanged. `models::Patch<PatchT>` (in `agent/src/models/mod.rs`) is `fn patch(&mut self, patch: PatchT)`.
- `agent/src/filesys/file.rs` — `File { path: PathBuf }` with `Clone/Debug/PartialEq/Eq/Hash` only. `files::write_json` creates parent directories and supports atomic writes, so no directory setup is needed for the state file.
- `agent/src/disk/layout.rs` — `Layout` maps the on-disk tree rooted at `<filesystem_root>/var/lib/miru/`; flat state files live at the root (`device()` → `device.json`, `settings()` → `settings.json`). Existing consumers of the pattern: `agent/src/authn/token_mngr.rs` (`TokenFile = SingleThreadStateFile<Token, token::Updates>` at `layout.auth().token()`), `agent/src/disk/device.rs` (`Device = ConcurrentStateFile<models::Device, device::Updates>` at `layout.device()`).
- Serializability of `Config`'s members: `Deployment` and `UploadRule` (in `agent/src/models/`) both derive `Serialize` and implement custom, forward-tolerant `Deserialize`. `UploadRule.digest: String` identifies the rule content and drives the restore decision. `UploadCollectionID` is a `String` alias.
- Tests: scanner behavior tests are inline `#[cfg(test)]` modules in `agent/src/scan/*.rs` (there is no `agent/tests/scan/` mirror because the modules are `pub(crate)`); they use the deterministic `Clock` helper, `dirs::temp("testing")` RAII temp dirs, and `files::write_bytes`. `agent/tests/filesys/state_file.rs` covers the StateFile abstraction itself and needs no changes.
- Environment quirk: ~15 unrelated tests fail when run as root (permission-bit tests); verify any failure also exists on the unmodified branch point before attributing it to this change. `gh` is unavailable — watch CI via the GitHub MCP tools (`mcp__github__actions_list`, `mcp__github__get_job_logs`, `mcp__github__pull_request_read`) against `mirurobotics/agent`.

## Plan of Work

Milestone 1 — make the state serializable (no behavior change):

1. `agent/src/filesys/file.rs`: add `serde::{Serialize, Deserialize}` to `File`'s derive list plus `#[serde(transparent)]` on the struct, so `File` serializes as its path string (valid as a JSON map key). Import per repo grouping rules.
2. `agent/src/scan/state.rs`: add `Serialize, Deserialize` to the derives of `Config`, `Observation`, `Candidate`, `StableFile`; add `Clone, Serialize, Deserialize` to `State`. Add at the bottom of the types section:

       #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
       pub(crate) struct PersistedState {
           pub(crate) collections: HashMap<UploadCollectionID, State>,
       }

       impl Patch<PersistedState> for PersistedState {
           fn patch(&mut self, patch: PersistedState) {
               *self = patch;
           }
       }

       pub(crate) type ScanStateFile = SingleThreadStateFile<PersistedState, PersistedState>;

   (`UploadCollectionID` from `crate::models`, `Patch` from `crate::models`, `SingleThreadStateFile` from `crate::filesys::state_file`.)
3. Inline tests in `agent/src/scan/state.rs`: a `mod persistence` asserting (a) a populated `State` (non-empty preexisting/candidates/ledger) round-trips through `serde_json::to_string` → `from_str` equal to the original, and (b) `PersistedState::patch` replaces the whole value. Reuse the existing fixture helpers (`config`, `stable_file`, `observation`).
4. Commit: `feat(scan): make scanner state serializable`.

Milestone 2 — wire persistence into the scanner:

1. `agent/src/disk/layout.rs`: add

       pub fn scanner_state(&self) -> filesys::File {
           self.root().file("scanner_state.json")
       }

2. `agent/src/scan/collection.rs`:
   - Add `pub(crate) fn state(&self) -> &State { &self.state }` (persistence snapshot accessor).
   - Add a restore constructor:

         pub(crate) async fn restore(
             state: State,
             config: Config,
             now: DateTime<Utc>,
         ) -> Result<Self, ScanErr>

     Behavior: `let digest_unchanged = state.cfg.rule.digest == config.rule.digest;` then build `Self::from_state(state)`, apply `self.state.set_config(config)?` (same collection id by construction, so this cannot hit `InvalidRule` when keyed correctly), and only when `!digest_unchanged` re-snapshot: `self.state.preexisting = discover_preexisting(&self.state, now).await?`.
3. `agent/src/scan/scanner.rs`:
   - `ScannerArgs` gains `pub state_file: Option<ScanStateFile>`; `Default` sets `None`.
   - `SingleThreadScanner` gains fields `state_file: Option<ScanStateFile>` and `restored: HashMap<UploadCollectionID, State>`. `new(args)` moves `args.state_file` in and seeds `restored` from `state_file.read().collections` when present (`Arc` deref + clone).
   - Private helper `async fn persist(&mut self)`: if `state_file` is `Some`, compose `PersistedState { collections }` from every live scanner (`scanner.state().clone()` keyed by collection id) plus every remaining `restored` entry, call `state_file.patch(snapshot).await`, and on `Err` log `warn!("scan: failed to persist scanner state: {err}")` — never propagate.
   - `update_rules`: for a collection id not in `self.scanners`, first try `self.restored.remove(&rule.upload_collection_id)`; if `Some(state)`, insert `CollectionScanner::restore(state, config, now).await?`, else `CollectionScanner::new(config, now).await?` as today. Call `self.persist().await` before returning `Ok`.
   - `scan`: call `self.persist().await` after `emit_stable_files` (end of the pass).
   - `prune`: also `prune_ledger(before)` each `restored` entry and drop restored entries left with an empty ledger and no candidates (they hold nothing worth restoring); then `self.persist().await`.
   - `Worker::run`, `Command::Shutdown` arm: call `self.scanner.persist().await` before responding/breaking, so a graceful shutdown lands the final state.
4. Inline tests in `agent/src/scan/scanner.rs` (new `mod persistence` using the existing `Clock`/`single_coll`-style helpers; add a helper that spawns a scanner whose `ScannerArgs.state_file` is `Some(ScanStateFile::new_with_default(file, PersistedState::default()).await.unwrap())` over a temp-dir file, and a `restart` helper that shuts the actor down and spawns a fresh one on the same file):
   - `dedup_survives_restart`: deploy rule, file goes stable (ledger 1), shutdown; respawn on the same state file, re-deploy the identical rule, scan twice past the window → no `StableFile` re-emitted, `get_ledger_count() == 1`.
   - `downtime_file_uploaded_when_digest_unchanged`: after shutdown, write a new file, respawn + re-deploy the same rule (same `digest`) → the new file is discovered, goes stable, and is emitted exactly once.
   - `downtime_file_suppressed_when_digest_changed`: same as above but the re-deployed rule has a different `digest` → re-snapshot marks the downtime file preexisting; nothing is emitted.
   - `midwindow_candidate_survives_restart`: window 10, discover at t, shutdown at t+1, respawn + re-deploy, advance past window → emitted exactly once.
   - `missing_state_file_starts_fresh`: spawn with a path in an empty temp dir → behaves like today and creates the file (assert it exists after a scan).
   - `corrupt_state_file_starts_fresh`: write `not json` to the path first → spawn succeeds (fail-open), scanner works, and the file ends up as valid JSON again.
   - `persisted_snapshot_written`: after a file goes stable, `files::read_json::<PersistedState>` on the path contains the collection id with a 1-entry ledger.
   - `prune_drops_restored_entries`: restored-but-undeployed collection disappears from the snapshot after `prune(now)` with a cutoff past its ledger entries.
5. `cargo fmt -p miru-agent`; commit: `feat(scan): persist scanner state via StateFile`.

Milestone 3 — validation: build, full test suite, lint, push, CI to green (details below). Produces no commit unless fixes are needed.

## Concrete Steps

All commands run from the repo root on branch `feat/persist-scanner-state`.

Milestone 1:

    # edit agent/src/filesys/file.rs and agent/src/scan/state.rs per Plan of Work
    cargo build -p miru-agent
    ./scripts/test.sh
    cargo fmt -p miru-agent
    git add -A && git commit -m "feat(scan): make scanner state serializable"

Milestone 2:

    # edit agent/src/disk/layout.rs, agent/src/scan/collection.rs, agent/src/scan/scanner.rs per Plan of Work
    cargo build -p miru-agent
    ./scripts/test.sh          # wraps: RUST_LOG=off cargo test --features test
    cargo fmt -p miru-agent
    git add -A && git commit -m "feat(scan): persist scanner state via StateFile"

Milestone 3:

    ./scripts/update-deps.sh   # refresh Cargo.lock before lint, per AGENTS.md
    ./scripts/lint.sh          # import linter, fmt --check, machete, audit, clippy
    ./scripts/covgate.sh       # coverage gates; add tests if scan/ dips below 98.85 — do NOT edit .covgate
    git push origin feat/persist-scanner-state

Then watch the `CI` workflow (jobs: `lint`, `test`, `tools`) on the pushed head via the GitHub MCP tools against `mirurobotics/agent` (`gh` is unavailable in this environment). If `update-deps.sh` changed `Cargo.lock`, commit it separately as `build(deps): refresh Cargo.lock`.

## Validation and Acceptance

1. `cargo build -p miru-agent` succeeds after each milestone.
2. `./scripts/test.sh` passes with zero new failures. All pre-existing tests pass unchanged (Milestone 1 is behavior-neutral; Milestone 2 leaves the `state_file: None` path identical to today). The new tests listed in Plan of Work Milestone 1 step 3 and Milestone 2 step 4 all pass; each of the restart tests fails if its wiring piece is removed (e.g. `dedup_survives_restart` fails without the restore path, `downtime_file_uploaded_when_digest_unchanged` fails if restore always re-snapshots preexisting).
3. Behavior acceptance (all demonstrated by the named tests): a stable-reported file is not re-reported across an actor restart on the same state file; a file created between shutdown and respawn under an unchanged rule digest is reported exactly once; a missing or corrupt `scanner_state.json` never errors — the scanner starts fresh and rewrites the file.
4. `./scripts/lint.sh` is clean; `./scripts/covgate.sh` passes without modifying any `.covgate` file.
5. Preflight reports CLEAN: the `CI` workflow (lint, test, tools jobs) is green on the pushed head of `feat/persist-scanner-state`. The PR must not leave draft, and the task must not be reported complete, until CI is green on that head.

## Idempotence and Recovery

All steps are safe to re-run: edits are convergent, `cargo build`/`test.sh`/`lint.sh` are read-only or idempotent, and `files::write_json` writes atomically so an interrupted state-file write can never corrupt an existing file (the fail-open `new_with_default` covers every other corruption source). Before a milestone commit, `git checkout -- <file>` reverts a bad edit; the branch exists only for this work, so `git reset --hard 206c2d0` is a full rollback pre-push, and a force-push of the reset branch recovers post-push (no other consumers of the branch). At runtime, deleting `/var/lib/miru/scanner_state.json` is always a safe operator reset — the scanner regenerates it on the next mutation.
