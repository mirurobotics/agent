# Prune the scanner ledger inside the discovery glob pass, gated by an audit-history threshold

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

This plan **supersedes** (in part) `plans/active/20260715-prune-scanner-ledger-on-scan-cadence.md`. That plan wired an external, worker-driven prune (`ScannerExt::prune` called by the scan driver after every scan tick) and later pivoted its semantics from age-based to existence-based (`file.exists()`), landing as commits `1fdb2e3` and `f790cfc` on branch `claude/practical-newton-1dsjq9`. This plan replaces that design: the worker-driven prune plumbing is **removed entirely** and pruning moves **inside the scanner's own discovery pass**, keyed off the glob result the scanner already computes, and gated by a minimum ledger size so a large audit history is retained. When this work lands, move the old plan to `plans/completed/` with a closing note pointing here.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Move ledger pruning into the collection scanner's discovery pass (`agent/src/scan/collection.rs`, `agent/src/scan/state.rs`), add a prune threshold constant, and remove the now-redundant external prune plumbing (`agent/src/scan/scanner.rs`, `agent/src/workers/scan.rs`, `agent/tests/mocks/scanner.rs`). Rewrite the prune tests at every level. |

This plan lives in `agent/plans/backlog/` (the repo's plan directories are `plans/{backlog,active,completed}` at the repo root; the `.agents` policy's nominal `.agents/exec-plans/` location is overridden by this repo's convention and by the task instruction).

The Rust crate root is `agent/agent/` — the workspace `Cargo.toml` is at the repo root `/home/user/agent/Cargo.toml` and the crate manifest at `agent/Cargo.toml` (paths below are relative to the repo root `/home/user/agent`; source lives under `agent/src/...`). All work happens on the already-checked-out branch `claude/practical-newton-1dsjq9`; do not switch branches.

## Purpose / Big Picture

The scanner keeps a per-collection **ledger** — `CollectionState.ledger: HashMap<File, Vec<StableFile>>` (`agent/src/scan/state.rs`) — recording every file that has gone "stable" and been reported for upload, so the same content is never re-reported. The ledger has two jobs that pull in opposite directions:

1. **Dedup** — discovery skips a file whose on-disk metadata matches its latest ledger entry, and evaluation dedups by digest. An entry is useful for dedup only while its file can still be re-observed, i.e. while it still matches the rule's glob.
2. **Audit** — the ledger is the device-side history of what was reported when. Operators want to review it, so aggressively deleting entries the moment a file disappears throws away reviewable history for no memory benefit when the ledger is small.

The current implementation on this branch (existence-based prune, driven by the scan worker after every tick) serves (1) but damages (2): it deletes every entry whose file is gone, immediately, unconditionally — even a 10-entry ledger loses its history. It also adds a whole command-plumbing path (`ScannerExt::prune` → `Command::Prune` → `SingleThreadScanner::prune`) just to trigger work the scanner could do during the pass it already runs.

After this change: while a collection's ledger holds fewer than 1000 files, **nothing is ever pruned** — the full history stays reviewable. Once the ledger reaches 1000 files, each discovery pass drops the entries whose file no longer appears in the rule's freshly-globbed source, using the same single glob call discovery already makes. No extra glob, no extra actor command, no worker involvement. The scan driver worker reverts to its original scan-only shape.

Observable outcome: run the scan/collection/state test suites and see threshold-gated glob-set pruning behavior; read `agent/src/workers/scan.rs` and see no prune anywhere; `grep -rn "Command::Prune" agent/src/scan/` returns nothing.

## Progress

- [x] Edit 1: `agent/src/scan/state.rs` — threshold constant + repurposed `prune_ledger(&mut self, globbed)`.
- [x] Edit 2: `agent/src/scan/collection.rs` — glob once per pass at the callers, thread the file list through `observe_untracked`, prune inside `discover_candidates`.
- [x] Edit 3: `agent/src/scan/scanner.rs` — remove `SingleThreadScanner::prune`, `Command::Prune`, worker match arm, `ScannerExt::prune`, `Scanner::prune`.
- [x] Edit 4: `agent/src/workers/scan.rs` — remove `prune_ledger` helper, both call sites, and the in-file test module (revert to pre-feature scan-only shape).
- [x] Edit 5: `agent/tests/mocks/scanner.rs` — remove `MockScanner::prune`.
- [x] Tests: rewrite `state.rs::prune_ledger` (3 tests), `collection.rs::prune_ledger` (4 tests, discovery-integrated), replace `scanner.rs::prune` with 3 scan-driven prune tests (incl. the optional `undeployed_collection_is_not_pruned`).
- [x] Build + full test suite green locally (minus known root-sandbox environment failures; see Surprises); covgates re-checked — `scan` 99.10 ≥ 98.83, `workers` 85.80 ≥ 84.67, no regeneration needed.
- [ ] Preflight reports CLEAN (CI green on the pushed head of `claude/practical-newton-1dsjq9`).
- [x] Move `plans/active/20260715-prune-scanner-ledger-on-scan-cadence.md` to `plans/completed/` with a closing note.

## Surprises & Discoveries

(Add entries as you go.)

- Observation (authoring): No test-only threshold override is needed. The pruned entries never need real files on disk — seeding ≥1000 in-memory `HashMap` entries keyed to paths inside (or outside) the temp dir is microseconds of work — so tests exercise the real production constant.
  Evidence: `stable_file()` fixtures in `state.rs`/`collection.rs` tests construct `StableFile` values with no I/O; only the *retained* (glob-matched) files need to exist on disk, and only in the collection-level tests.
- Observation (authoring): The only production caller of `ScannerExt::prune` is the scan worker; the only other implementor is `MockScanner`. `grep -rn "\.prune(" agent/src agent/tests` shows scan-side hits only in `workers/scan.rs`, `scan/scanner.rs`, and `tests/mocks/scanner.rs` (the `cache` module has its own unrelated `prune`). Removal is contained.
- Observation (implementation, 2026-07-17): The root sandbox has more environment-artifact test failures than the 3 previously catalogued. Running `cargo test --no-fail-fast` as root also fails 15 integration tests in the `mod` binary (permission-denied fixtures that root bypasses, a `/root`-home-dir assertion, unwritable-dest gcs/s3 cases, `sync::deployments::apply_error_isolation`). Verified pre-existing by stashing this change and re-running a sample — all fail on the unchanged tree too. None touch scan/worker code. CI (non-root) is the authority.
- Observation (implementation, 2026-07-17): Removing `file.exists()` from `CollectionState::prune_ledger` left `PathExt` unused in `state.rs` — dropped from the import. Coverage after the change: `scan` 99.10% (gate 98.83), `workers` 85.80% (gate 84.67) — deleting the worker-prune code *raised* the workers ratio, so no covgate regeneration was needed.

## Decision Log

- Decision: **Prune inside the discovery pass, using the discovery glob result.** `CollectionScanner::discover_candidates` (the method) performs the single `files::glob` call for the pass, hands the file list to candidate discovery, then calls `CollectionState::prune_ledger(&globbed)` on the same list.
  Rationale: Discovery already computes exactly the set the prune needs (the files that can currently be re-observed). Reusing it means one glob per pass, no second I/O walk, no actor command, no worker coupling. It also fixes a semantic gap in the existence-based version: a file that *exists* but no longer *matches the glob* can never be re-discovered (discovery only considers globbed files), so its entry is dead weight for dedup — glob-set pruning drops it, `file.exists()` pruning kept it forever.
  Date/Author: 2026-07-17 / plan author.

- Decision: **Threshold-gate the prune: only fire when `ledger.len() >= LEDGER_PRUNE_THRESHOLD` (1000).** Below the threshold nothing is pruned, ever; above it, only glob-set-absent keys are removed — the prune does **not** trim down to the threshold, and entries for still-globbed files are kept unbounded.
  Rationale: The user wants the ledger reviewable for auditing; a small ledger costs nothing, so keep all of it. The threshold bounds only the *stale* population: once large, entries that can no longer serve dedup are shed. Not trimming to exactly N is deliberate — live files' history must survive for dedup correctness (dropping a live file's entry causes a duplicate upload), so the ledger may legitimately stay above 1000 when ≥1000 globbed files exist.
  Date/Author: 2026-07-17 / plan author.

- Decision: **The threshold counts ledger keys (`ledger.len()`, i.e. files), not total `StableFile` entries.** It is a `pub(crate) const LEDGER_PRUNE_THRESHOLD: usize = 1000;` in `agent/src/scan/state.rs`, not a config knob.
  Rationale: `retain` removes whole keys — keys are the unit the prune can actually reduce, so gating on anything else (e.g. summed `Vec` lengths) could fire the prune yet remove nothing when a few live files hold deep histories. It also matches the existing `ledger_count()` accessor, keeping "the number the gate reads" and "the number tests/ops observe" the same metric. A per-rule or app-level option is over-engineering absent a requirement; a `pub(crate)` const is greppable, testable by reference, and trivially promotable to config later.
  Date/Author: 2026-07-17 / plan author.

- Decision: **No test-only threshold override.** Tests seed `LEDGER_PRUNE_THRESHOLD` (or +1) in-memory ledger entries directly and reference the constant by name.
  Rationale: 1000 `HashMap` inserts of small structs is negligible; no `#[cfg(test)]` const shadowing (which would leave the production value uncompiled under test), no constructor parameter, no cfg soup. Tests exercise the real gate at its real value.
  Date/Author: 2026-07-17 / plan author.

- Decision: **Remove the external prune plumbing entirely**: `ScannerExt::prune`, `Command::Prune` (+ its `Worker::run` match arm), `SingleThreadScanner::prune`, `Scanner::prune`, the `CollectionScanner::prune_ledger` pass-through, `workers/scan.rs`'s `prune_ledger` helper and both call sites plus its prune-focused in-file test module, and `MockScanner::prune`. The worker reverts to its pre-feature scan-only shape.
  Rationale: With pruning inside `scan()`'s discovery pass, an external prune trigger is dead API. `scan()` already persists the snapshot after each pass, so the prune result is captured by the existing persist — the separate persist inside `SingleThreadScanner::prune` was the only other thing that path did. Verified no other consumers exist (see Surprises).
  Date/Author: 2026-07-17 / plan author.

- Decision: **`CollectionState::prune_ledger` is repurposed, not removed**: new signature `pub(crate) fn prune_ledger(&mut self, globbed: &[File])`, returning nothing (it is infallible — the fallible glob happens at the caller).
  Rationale: State mutation stays on `CollectionState` next to its siblings (`set_config`, `latest_ledger_entry_mut`); the collection scanner stays an orchestrator. Dropping the `Result<(), ScanErr>` return removes ceremony that existed only to satisfy the old actor-command signature.
  Date/Author: 2026-07-17 / plan author.

- Decision: **`update_config` (rule redeploy) does not prune**, even though it also globs (via `discover_preexisting`). Pruning runs only in the periodic discovery pass.
  Rationale: One prune site is easier to reason about and test; `update_config`'s glob serves a different purpose (re-snapshotting preexisting files for the new rule); and the next scan tick (≤ `scan_interval_secs`, default 60s) prunes anyway, so a config-time prune buys at most one minute. Skipping it also gives a transiently-misconfigured glob (e.g. a bad rule pushed then corrected) one tick of grace before history keyed outside it is dropped.
  Date/Author: 2026-07-17 / plan author.

- Decision: **Accepted correctness caveat**: glob-set pruning drops entries for files that still exist but no longer match the rule's glob. If a rule's glob is narrowed and later re-broadened across deploys while the ledger is ≥1000 files, a pruned-then-rematched unchanged file will be re-reported (duplicate upload).
  Rationale: While a file is outside the glob it cannot be discovered, so its entry serves no dedup purpose — pruning it is safe *at that moment*. The narrow-then-rebroaden sequence is rare, requires the ≥1000 gate to be open, and costs one redundant upload, not data loss. Documented here and in a comment on `prune_ledger`.
  Date/Author: 2026-07-17 / plan author.

- Decision: **Non-deployed collections are intentionally never glob-pruned.** `SingleThreadScanner::scan` calls `discover_candidates` only when `self.deployed.contains(cid)`; undeployed collections just drain their remaining candidates and are removed wholesale by the existing inactive-collection logic once empty.
  Rationale: The asymmetry is harmless — an undeployed collection's entire state (ledger included) is dropped shortly anyway, which is a stronger prune than the glob-set one. No code change needed; stated so a future reader knows it is deliberate.
  Date/Author: 2026-07-17 / plan author.

## Outcomes & Retrospective

2026-07-17 (implementation): All five edits landed exactly as planned. `prune_ledger` now lives on `CollectionState` as `fn prune_ledger(&mut self, globbed: &[File])` gated by `LEDGER_PRUNE_THRESHOLD` (1000 keys); `CollectionScanner::discover_candidates` globs once and feeds both candidate discovery and the prune; `observe_untracked`/`discover_preexisting`/`discover_candidates` (free fns) take the glob slice and became infallible; the entire external prune path (`ScannerExt::prune`, `Command::Prune` + match arm, `SingleThreadScanner::prune`, `Scanner::prune`, `CollectionScanner::prune_ledger` pass-through, worker helper + in-file test module, `MockScanner::prune`) is gone and the scan worker is back to scan-only. Tests: 3 state-level, 4 collection-level (incl. `discovery_prunes_existing_but_unmatched_file` and `update_config_does_not_prune`), 3 actor-level (incl. the optional `undeployed_collection_is_not_pruned`, which proved cleanly testable by keeping a window-waiting candidate alive). Validation: build clean; full suite green minus pre-existing root-sandbox artifacts; import linter + fmt + clippy (`--all-targets --all-features -D warnings`) clean; scan 99.10 / workers 85.80 vs gates 98.83 / 84.67 — no covgate changes. Remaining: CI preflight on the pushed head (orchestrator-owned).

## Context and Orientation

Assume zero prior context. Everything needed is here.

### The scanner, in one paragraph

`agent/src/scan/` implements a file-upload scanner as an actor. `SingleThreadScanner` (`scanner.rs`) owns a map of per-collection `CollectionScanner`s (`collection.rs`), each wrapping a `CollectionState` (`state.rs`). On every `scan()` the actor: evaluates in-flight **candidates** (files waiting out a stability window; stable ones are emitted once and appended to the **ledger**), then — for **deployed** collections only — runs **discovery**: `files::glob(&state.cfg.rule.source.glob)` (a sync call returning `Result<Vec<File>, FileSysErr>`, `agent/src/filesys/files.rs:484`) lists the rule's source files, and each globbed file not already tracked / preexisting / matching its latest ledger entry becomes a new candidate. After the pass, `persist_snapshot()` writes the whole state (ledger included) to the on-disk snapshot file. The scan driver worker (`agent/src/workers/scan.rs`) calls `scanner.scan()` immediately at startup and then on a fixed cadence (default 60s).

### Exact current state on branch `claude/practical-newton-1dsjq9` (what this plan changes)

- `agent/src/scan/state.rs:86` — `CollectionState::prune_ledger(&mut self) -> Result<(), ScanErr>` does `self.ledger.retain(|file, _| file.exists());`. `ledger_count()` (line 42) returns `self.ledger.len()` — the number of FILES (keys), not total `StableFile` entries.
- `agent/src/scan/collection.rs:118` — `CollectionScanner::prune_ledger()` pass-through to the state method. `observe_untracked` (line 127) is the single glob site used by both `discover_preexisting` (constructor + `update_config`) and the free `discover_candidates`.
- `agent/src/scan/scanner.rs` — `SingleThreadScanner::prune` (line 221, loops collections + persists), `Command::Prune` (line 271) + `Worker::run` match arm (line 341), `ScannerExt::prune` (line 242), `Scanner::prune` impl (line 446). `scan()` (line 186) evaluates, discovers (deployed only), removes drained inactive collections, persists, emits.
- `agent/src/workers/scan.rs` — `run_impl` calls `prune_ledger(scanner)` (an error-swallowing helper wrapping `scanner.prune()`) after the initial scan and after each cadence tick. An in-file `#[cfg(test)] mod tests` (added by the superseded feature) has a `RecordingScanner` with prune counting/failure injection, a `Harness`, and two tests: `prunes_on_initial_pass_and_each_tick`, `prune_error_does_not_stop_the_loop`.
- `agent/tests/mocks/scanner.rs` — `MockScanner` implements `ScannerExt` including a trivial `prune()`. `agent/tests/workers/scan.rs` drives the worker through `MockScanner` but never references prune directly (its three tests are scan/shutdown-only and survive this change untouched).
- Tests covering existence-based pruning that must be rewritten or removed: `state.rs` `mod prune_ledger` (3 tests), `collection.rs` `mod prune_ledger` (2 tests), `scanner.rs` `mod prune` (2 tests: `prune_retains_present_drops_gone`, `prune_persists_snapshot`), the worker in-file `mod tests` (2 tests).
- Coverage gates: CI (`.github/workflows/ci.yml`, job step `./scripts/covgate.sh`) enforces per-module minimums from `.covgate` files — `agent/src/scan/.covgate` = 98.83, `agent/src/workers/.covgate` = 84.67 (both adjusted on this branch). `./scripts/update-covgates.sh` recomputes them.

### Terms

- **Ledger**: `HashMap<File, Vec<StableFile>>` — per-file history of reported stable versions; dedup source + audit record.
- **Discovery pass / glob pass**: the per-scan `files::glob` + candidate-promotion step in `CollectionScanner::discover_candidates`.
- **Glob set**: the `Vec<File>` returned by `files::glob` for the rule's source pattern this pass — the complete set of files the rule can currently observe.
- **Threshold / gate**: `LEDGER_PRUNE_THRESHOLD` (1000 ledger keys); pruning is a no-op below it.
- **Preexisting**: files snapshotted at collection creation / rule update; never promoted to candidates unless their metadata changes.

## Plan of Work

All edits in the `agent` crate. Keep the existing terse style: three-section imports, `if { exit }` disqualifying conditionals, small functions.

### Edit 1 — `agent/src/scan/state.rs`: threshold constant + repurposed prune

Above `CollectionState` (near the other module-level items), add:

    /// Minimum number of ledger files (keys) before pruning activates. Below
    /// this the full ledger is kept as reviewable audit history; at or above
    /// it, discovery prunes entries whose file no longer matches the rule's
    /// glob (see `prune_ledger`).
    pub(crate) const LEDGER_PRUNE_THRESHOLD: usize = 1000;

Replace the existing `prune_ledger` (line 86) with:

    /// Drop ledger entries whose file is absent from this pass's glob set.
    /// Gated: a ledger below LEDGER_PRUNE_THRESHOLD keys is left untouched so
    /// small histories stay auditable. Caveat: a glob narrowed then later
    /// re-broadened can re-report an unchanged file whose entry was pruned
    /// while outside the glob (rare; costs one duplicate upload).
    pub(crate) fn prune_ledger(&mut self, globbed: &[File]) {
        if self.ledger.len() < LEDGER_PRUNE_THRESHOLD {
            return;
        }
        let globbed: HashSet<&File> = globbed.iter().collect();
        self.ledger.retain(|file, _| globbed.contains(file));
    }

`HashSet` is already imported in this file (`std::collections::{HashMap, HashSet}`); `File` already implements `Hash`/`Eq` (it keys the ledger). The `HashSet` is built only when the gate opens, so sub-threshold passes cost one `len()` check.

### Edit 2 — `agent/src/scan/collection.rs`: glob once at the caller, prune in discovery

Change `observe_untracked` to take the glob result instead of globbing itself; it becomes infallible (its only error source was the glob — per-file observation failures are already skip-and-continue):

    async fn observe_untracked(
        state: &CollectionState,
        globbed: &[File],
        now: DateTime<Utc>,
    ) -> Vec<(File, Observation)> {
        let mut observed = Vec::new();
        for file in globbed {
            if state.is_candidate(file) { continue; }
            let observation = match observe_file(state, file.clone(), now).await {
                Ok(observation) => observation,
                Err(err) => {
                    warn!("skipping unreadable file {}: {err}", file.path().display());
                    continue;
                }
            };
            observed.push((file.clone(), observation));
        }
        observed
    }

Thread the parameter through the two free wrappers (`discover_preexisting`, `discover_candidates`) — both take `globbed: &[File]` and lose their `Result` where the glob was the only error (keep `discover_preexisting`'s signature returning the map, now infallible; adjust callers' `?` accordingly).

Glob at the three call sites:

- `CollectionScanner::new`: `let globbed = files::glob(&state.cfg.rule.source.glob)?;` then `state.preexisting = discover_preexisting(&state, &globbed, now).await;`. No prune here (brand-new state has an empty ledger anyway, unless restored — restoration goes through `from_state`, which does not glob; first scan prunes).
- `CollectionScanner::update_config`: same shape; **no prune** (Decision Log).
- `CollectionScanner::discover_candidates` (the method) — the prune seam:

      pub(crate) async fn discover_candidates(&mut self, now: DateTime<Utc>) -> Result<(), ScanErr> {
          let globbed = files::glob(&self.state.cfg.rule.source.glob)?;
          let candidates = discover_candidates(&self.state, &globbed, now).await;
          self.state
              .candidates
              .extend(candidates.into_iter().map(|c| (c.file.clone(), c)));
          self.state.prune_ledger(&globbed);
          Ok(())
      }

Delete the `CollectionScanner::prune_ledger` pass-through (line 118-120). A glob error propagates as before (the actor's `scan()` warns per-collection and continues) — on such a tick no prune runs, which is correct: never prune against a set you failed to compute. Note the prune result is persisted by the existing `persist_snapshot()` at the end of `SingleThreadScanner::scan` — no new persist call anywhere.

### Edit 3 — `agent/src/scan/scanner.rs`: remove the actor prune path

Remove: `SingleThreadScanner::prune` (lines 221-227), the `Command::Prune` variant (lines 271-273), its `Worker::run` match arm (lines 341-347), `ScannerExt::prune` (line 242), and the `Scanner::prune` trait-impl method (lines 446-449). Nothing else in `scan()` changes — discovery now prunes internally and the existing persist captures it.

### Edit 4 — `agent/src/workers/scan.rs`: revert the worker to scan-only

Remove the `prune_ledger` helper (lines 74-78) and both call sites (lines 61, 70), restoring `run_impl` to: initial `scan()` (error logged), then loop { sleep; `scan()` (error logged) }. Remove the entire in-file `#[cfg(test)] mod tests` (lines 80-277) — it exists solely to pin worker-driven pruning. The worker's scan-driving, error-survival, and shutdown behavior remain covered by the integration tests in `agent/tests/workers/scan.rs`, which need no changes. Drop now-unused imports if any (check `debug`/`error`/`info` usage after the removal — all three remain used).

### Edit 5 — `agent/tests/mocks/scanner.rs`: shrink the mock

Delete the `async fn prune(...)` impl (lines 125-127) — the trait method no longer exists, so leaving it is a compile error, which is the safety net proving no consumer was missed.

### Confirm-nothing-else check (before starting)

From the repo root run `grep -rn "\.prune(\|Command::Prune\|prune_ledger\|ScannerExt" agent/src agent/tests` and confirm scan-related hits are only the files named in Edits 1-5 (the `cache` module's prune is a separate, unrelated API — do not touch it).

## Test Plan (encoded steps — implement exactly)

### State-level — rewrite `mod prune_ledger` in `agent/src/scan/state.rs`

Use the existing `stable_file()` fixture; entries need no on-disk files. Reference `LEDGER_PRUNE_THRESHOLD` by name (`use super::LEDGER_PRUNE_THRESHOLD;`). Helper suggestion: `fn seed_n(state: &mut CollectionState, n: usize)` inserting keys `File::new(format!("/none/{i}.mcap"))`.

1. `below_threshold_prunes_nothing` — seed `LEDGER_PRUNE_THRESHOLD - 1` entries; call `prune_ledger(&[])` (empty glob set — maximally aggressive input); assert `ledger_count()` unchanged. This is the audit-history guarantee.
2. `at_threshold_drops_unglobbed_keeps_globbed` — seed exactly `LEDGER_PRUNE_THRESHOLD` entries; build a glob slice containing, say, 3 of the seeded files; prune; assert count == 3, the globbed keys remain with their **full `Vec` history** (seed one of them with a 2-entry Vec and assert both survive), and a specific unglobbed key is gone. Exactly-at-threshold firing pins the `>=` boundary.
3. `empty_ledger_noop` — prune an empty ledger with an empty slice; count stays 0 (also proves the gate returns early without building the set).

### Collection-level — rewrite `mod prune_ledger` in `agent/src/scan/collection.rs` (discovery-integrated, real temp files)

Drive through the public seam: seed state, wrap in `CollectionScanner::from_state`, call `scanner.discover_candidates(ts)`, assert on the ledger. Reuse `dirs::temp`, `write`, `glob_for`, `stable_file`, `seed_ledger` fixtures.

1. `discovery_prunes_stale_entries_at_threshold` — temp dir; write 2 real `.mcap` files and seed matching ledger entries for them (`ledger_entry_matching` so discovery also skips them as already-reported); seed `LEDGER_PRUNE_THRESHOLD` additional entries keyed to never-created paths inside the dir (e.g. `dir.file(format!("gone{i}.mcap"))`); run `discover_candidates`; assert `ledger_count() == 2`, both live files' entries retained, a sampled `gone*` key absent, and no candidates were created for the live files.
2. `discovery_below_threshold_keeps_stale_entries` — same shape with `LEDGER_PRUNE_THRESHOLD - 2` fake entries + 1 live file (total below gate); run discover; assert nothing pruned.
3. `discovery_prunes_existing_but_unmatched_file` — the behavioral delta vs. the superseded existence-based prune: write a real file that does NOT match the glob (e.g. `keep.txt` in a `*.mcap`-globbed dir), seed a ledger entry for it, pad past the threshold with fakes; run discover; assert the `.txt` entry was dropped even though the file exists.
4. `update_config_does_not_prune` — seed ≥threshold stale entries, call `update_config` with a same-collection v2 rule; assert `ledger_count()` unchanged (pins the Decision-Log choice).

### Actor-level — replace `mod prune` in `agent/src/scan/scanner.rs`

1. `scan_prunes_ledger_via_discovery` — build a `SingleThreadScanner` directly (the `scan_isolates_bad_glob_collection_from_emitting_sibling` test shows the pattern): construct a `CollectionState` whose ledger holds `LEDGER_PRUNE_THRESHOLD` stale (non-globbed) entries plus one entry matching a real on-disk file, insert it into `scanners` and `deployed`, run `single.scan().await`; assert the ledger shrank to 1 via state inspection. Then `clear_rules`-style: not needed — one focused test.
2. `scan_prune_is_persisted` — same seeding through the persisted fixture (`persisted_coll` / `spawn_persisted` helpers exist; seed via a pre-written snapshot file: build the `ScannerSnapshot` with the padded ledger, `patch` it into a `ScanSnapshotFile`, spawn the scanner from it, deploy the rule, `scan_once`); read the snapshot JSON and assert the persisted ledger no longer contains a sampled stale key. This replaces `prune_persists_snapshot` and proves the existing scan-persist captures the prune (no dedicated persist path anymore).
3. `undeployed_collection_is_not_pruned` (optional if awkward through the actor — the drain/remove logic usually deletes the whole collection first; if a clean seam does not exist, document the asymmetry in a comment instead of forcing a test).

### Worker + integration

- Delete the two prune tests with the worker's in-file module (Edit 4). Do NOT port them — the worker no longer prunes.
- `agent/tests/workers/scan.rs`: no changes (verify it still compiles — it only uses scan/shutdown).
- `agent/tests/mocks/scanner.rs`: `prune` removed (Edit 5).

### Coverage

CI's covgate step enforces `agent/src/scan/.covgate` (98.83) and `agent/src/workers/.covgate` (84.67). The new `prune_ledger` and the discovery seam are covered by the tests above (gate check, set-build branch, retain both ways, glob-error early return already covered by existing bad-glob tests). Removing worker prune code and its tests shifts the workers module's ratio; if the covgate job fails on a threshold, run `./scripts/update-covgates.sh` from the repo root and commit the regenerated `.covgate` files (they were adjusted the same way when the superseded feature landed).

## Concrete Steps

All commands from the repo root `/home/user/agent` (workspace root; the package is `miru-agent`).

1. Run the confirm-nothing-else grep (see Plan of Work) and record the result in Surprises & Discoveries if anything unexpected appears.
2. Make Edits 1-5.
3. Build: `cargo build --package miru-agent` — expect clean. A missing-method error on `MockScanner`/`Scanner` means a leftover `prune` reference; the compiler names the file.
4. Targeted tests: `cargo test --package miru-agent --features test scan::` and `cargo test --package miru-agent --features test --test mod workers` (or simply the full `cargo test --package miru-agent --features test`). Expect the rewritten prune tests to pass and all pre-existing scan/worker tests to remain green.
5. Lint: `./scripts/lint.sh` (repo root). Fix any drift (unused imports in `workers/scan.rs` and `state.rs` are the likely candidates).
6. Coverage sanity (optional locally, mandatory in CI): `./scripts/covgate.sh`; if a module gate fails, `./scripts/update-covgates.sh` and commit the `.covgate` changes.
7. Commit on `claude/practical-newton-1dsjq9` (conventional commit, e.g. `refactor(scan): prune ledger via discovery glob set behind audit threshold`), push, and watch CI.
8. Move `plans/active/20260715-prune-scanner-ledger-on-scan-cadence.md` to `plans/completed/`, appending a closing note that its worker-driven prune was superseded by this plan; commit with the same PR.

## Validation and Acceptance

Acceptance is behavioral:

- `cargo test --package miru-agent --features test` from `/home/user/agent` passes in full. The new tests fail before the change (e.g. `below_threshold_prunes_nothing` fails against existence-based pruning because a sub-threshold missing-file entry currently gets dropped; `discovery_prunes_existing_but_unmatched_file` fails because an existing-but-unmatched file is currently retained) and pass after.
- `grep -rn "prune" agent/src/workers/ agent/src/scan/scanner.rs` returns no scan-prune hits (worker file has none; scanner.rs has none).
- Reading `agent/src/scan/collection.rs::discover_candidates` shows exactly one `files::glob` call per pass feeding both candidate discovery and `prune_ledger`.

### Preflight / CI gate (mandatory)

Preflight MUST report **CLEAN** — CI green on the pushed head of `claude/practical-newton-1dsjq9` — before this task is reported complete or the PR leaves draft. Heavyweight validation (lint, tests, covgate) runs in GitHub Actions; drive fixes from CI job logs. Known operational hazard: GitHub Actions event delivery has been flaky on this repo recently (dropped `synchronize` events) — if a push produces no workflow run within a couple of minutes, re-trigger with an empty commit (`git commit --allow-empty -m "chore: retrigger ci"` and push), as was already done on this branch (`85de875`).

## Idempotence and Recovery

- All edits are plain source changes; re-running build/test/lint commands is non-destructive and repeatable.
- The change is behavior-compatible on disk: no snapshot format change (the ledger type is untouched), so a partially-landed state never corrupts persisted scanner state; pruning an already-pruned ledger is a no-op.
- If the build fails after Edit 3 with unresolved `prune` references, Edits 4-5 are incomplete — the compiler errors enumerate every leftover call site; fix those rather than re-adding the trait method.
- If a covgate fails in CI, regenerate with `./scripts/update-covgates.sh` and commit — do not hand-edit thresholds upward.
- Rollback path: `git revert` the commit(s); no migrations or external state involved.
- Do not switch branches; all work happens on `claude/practical-newton-1dsjq9`.

---

Change note (2026-07-17): Initial authoring. Verified against the working tree at commit `85de875`: the existence-based prune implementation and its full call/test graph, the single-glob discovery seam in `collection.rs`, `files::glob`'s signature, the absence of other `ScannerExt::prune` consumers, and the covgate values on this branch. Supersedes (in part) `plans/active/20260715-prune-scanner-ledger-on-scan-cadence.md`, to be moved to `plans/completed/` when this lands.
