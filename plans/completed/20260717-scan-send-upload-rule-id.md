# Stamp scan observations with the upload rule id, not the collection id

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `mirurobotics/agent`) | read-write | One-line production fix in `agent/src/scan/collection.rs` plus corrections and additions to the inline test modules in `agent/src/scan/{collection,state,scanner}.rs`. |

This plan lives in `plans/completed/` of the agent repo because all code changes are contained in this repository. Base branch: `main`. Working branch: `claude/task-mode-pr-agent-nxwei3`.

## Purpose / Big Picture

The agent (a Rust binary on customer robots) scans device directories for files matching "upload rules" and uploads stable files to cloud storage via the Miru backend. An **upload rule** (`agent/src/models/upload_rule.rs`) has two distinct identifiers: `id` (an `upl_rule_*` string, the rule's own identity, which the backend's `POST /agent/v1/uploads` endpoint requires) and `upload_collection_id` (an `upl_col_*` string naming the collection that groups rule versions).

Today the scanner stamps every file observation with the **collection** id in the field named `upload_rule_id` (`agent/src/scan/collection.rs` line 187). That wrong value flows unchanged through the whole pipeline into the backend request, and the backend answers 404 `resource_not_found` for every upload create request, from every rule, unconditionally. Production evidence from a staging device log on 2026-07-17:

    Upload Rule with id 'upl_col_NVbcMGwVyZapCSyuLhy1bijQn9suEdMRe' not found

The scanned-upload feature has therefore never worked end-to-end. After this change, the scanner stamps `state.cfg.rule.id` (the real `upl_rule_*` id), the backend accepts the create request, and files scanned on a device actually arrive in the configured bucket.

Observable outcome: deploy a release with an upload rule to a device, drop a matching file in the rule's glob directory, and the agent log shows `uploaded file <path> (rule upl_rule_..., digest sha256:...)` instead of the 404 error above. In tests: the scan module emits `StableFile` values whose `upload_rule_id` equals the rule's `id`, never its `upload_collection_id`.

## Progress

- [ ] Fix `observe_file` in `agent/src/scan/collection.rs` to stamp `state.cfg.rule.id`.
- [ ] Correct the wrong assertions and misleading comments in the inline test modules of `agent/src/scan/collection.rs` and `agent/src/scan/scanner.rs`; pin deterministic rule ids in the test rule helpers.
- [ ] Update the collection-id-shaped `upload_rule_id` fixture values in `agent/src/scan/state.rs` tests to rule-shaped values.
- [ ] Add regression tests that pin the rule-id stamp with realistic, distinct `upl_rule_*` / `upl_col_*` values (collection-level and scanner-actor-level).
- [ ] Run `./scripts/test.sh` from the repo root; full suite green.
- [ ] Run `scripts/covgate.sh`; scan module coverage still meets its `.covgate` threshold (98.83).
- [ ] Commit, push, and run preflight; CI (Lint + Test jobs of `.github/workflows/ci.yml`) green on the pushed branch head.

## Surprises & Discoveries

(Add entries as you go. Findings from plan authoring:)

- Observation: `observe_file` is the **only** production site that populates `Observation.upload_rule_id`. Everything downstream is a pure copy: `build_stable_file` (`agent/src/scan/collection.rs` line 288, `first_obs.upload_rule_id.clone()`), the scan→upload bridge (`agent/src/workers/scan_upload_bridge.rs` line 78, `Job.upload_rule_id: stable.upload_rule_id`), and the executor (`agent/src/upload/executor.rs` line 110, `CreateUploadRequest.upload_rule_id: job.upload_rule_id.clone()`). No other site — not the snapshot-restore path (`SingleThreadScanner::new` / `CollectionScanner::from_state`), not `update_rules` in `agent/src/scan/scanner.rs`, not `agent/src/disk/upload_rules.rs` (which keys the rule cache by `rule.id`, correctly), not `agent/src/workers/sync_scan_bridge.rs` — synthesizes the field from a collection id.
  Evidence: `grep -rn upload_rule_id agent/src` shows exactly one assignment from config (`collection.rs:187`); all others are copies or test fixtures.

- Observation: `upload_rule_id` does **not** participate in scan identity or dedup. Grouping is keyed by collection id in maps that are separate from the field: `SingleThreadScanner.scanners: HashMap<UploadCollectionID, CollectionScanner>` and `deployed: HashSet<UploadCollectionID>` (`agent/src/scan/scanner.rs` lines 56-57), persisted as `ScannerSnapshot.collections` keyed by `UploadCollectionID` (`agent/src/scan/state.rs` line 159). Dedup identity is metadata-only: `Observation::equal_metadata` compares size + mtime (`state.rs` lines 107-109), `StableFile::equal_metadata` compares size + mtime/mtime-aliases (`state.rs` lines 139-146), and `is_preexisting` / `is_latest_ledger_entry` use only those (`state.rs` lines 54-65); content dedup uses the file digest (`collection.rs`, `differs_from_previous`). The ledger is keyed by `File`. So the field is a pure payload passthrough that was simply mispopulated — the fix cannot disturb grouping, dedup, or ledger reconstruction.
  Evidence: `state.rs` lines 54-65, 107-109, 139-146; `scanner.rs` lines 152-184.

- Observation: the scanner-actor test comment at `agent/src/scan/scanner.rs` lines 1297-1298 ("upload_rule_id is stamped from the collection id in observe_file") documents the bug as if it were design intent, and `agent/src/scan/collection.rs` lines 1309-1343 (`stable_file_takes_identity_from_first_observation`) asserts `sf.upload_rule_id == "coll1"`. These encode the wrong behavior and must be corrected, not preserved.

## Decision Log

- Decision: Fix the value at its single production source (`observe_file`), changing `state.cfg.rule.upload_collection_id.clone()` to `state.cfg.rule.id.clone()`; make no other production changes.
  Rationale: The field is a payload passthrough with exactly one producer (see Surprises & Discoveries). Fixing the producer fixes every consumer; renaming fields, threading the rule differently, or re-stamping downstream would be churn with no behavioral benefit.
  Date/Author: 2026-07-17 / plan author (Claude).

- Decision: Keep the existing "identity from the FIRST observation" semantics: `build_stable_file` continues to copy `first_obs.upload_rule_id`, so a candidate discovered under rule version r1 is emitted with r1's id even if a same-collection config swap to r2 happens between discovery and evaluation (pinned by `stable_file_takes_identity_from_first_observation`).
  Rationale: This is deliberate, tested behavior orthogonal to the bug: the rule id active at discovery is the rule that caused the observation. Rule versions remain addressable backend entities (releases reference them by id in `release.upload_rule_ids`). In the worst case — an old rule id deleted server-side — the create request 404s and the uploader retries through its existing ladder (at most 9 total attempts) before dropping the job.
  Date/Author: 2026-07-17 / plan author (Claude).

- Decision: No migration or versioning of persisted scanner state (`scanner.json`) or of the persisted upload queue (`upload_queue.json`). Devices upgrading with pre-fix persisted state self-heal.
  Rationale: The scan snapshot (`ScannerSnapshot`, written to `<data root>/scanner.json`; path from `agent/src/disk/layout.rs` lines 45-47, wired in `agent/src/app/state.rs` around line 147) persists full `CollectionState`s: `cfg` (deployment + rule, both id fields intact), `preexisting` observations, in-flight `candidates`, and the `ledger` of `StableFile`s — all carrying `upload_rule_id` values that today hold collection ids. Deserialization is unaffected by this fix (same field name and type; only the runtime value changes). Post-upgrade consequences, all acceptable: (a) `preexisting` and `ledger` entries with stale collection ids are harmless — the field is never read for identity/dedup (metadata + digest only), and ledger entries are never re-emitted; (b) each candidate persisted **before** the upgrade still emits one `StableFile` carrying the stale collection id (identity from first observation), producing at most one more poisoned upload job per pre-fix in-flight candidate — retried through the uploader's existing ladder (at most 9 attempts) and then dropped, and never re-emitted because dedup is metadata/digest-based; (c) the persisted upload queue may hold poisoned pre-fix jobs — same bounded retry-then-drop applies; (d) once a file re-observes under the fixed code, the backend's digest-based dedup prevents duplicate uploads. Since the feature never worked, no device has correct historical state worth migrating.
  Date/Author: 2026-07-17 / plan author (Claude).

- Decision: A separate uploader permanent-4xx hardening (drop upload jobs immediately on definitive 4xx backend errors) was drafted for this branch but **descoped by user decision on 2026-07-17** (its draft PR was closed unmerged and the branch reset). It must not be implemented as part of this plan; poisoned jobs instead exhaust the existing bounded retry ladder before being dropped, which is acceptable.
  Rationale: User explicitly limited scope to the scan id root-cause fix; the retry ladder is finite, so stale persisted jobs still terminate without it.
  Date/Author: 2026-07-17 / plan author (Claude), amended by orchestrator.

- Decision: Correct, rather than preserve, every test that encodes the collection-id stamp, and pin deterministic rule ids in scan test helpers. Use realistic, distinct id shapes (`upl_rule_*` vs `upl_col_*`) in the new regression tests so an accidental field swap can never pass again.
  Rationale: Existing tests asserted the bug (`collection.rs:1334`, `scanner.rs:913/1168/1305/1361`); several test rules leave `UploadRule.id` at its `Default` (`unknown-<uuid>`, random per run), which would make post-fix assertions nondeterministic if unpinned.
  Date/Author: 2026-07-17 / plan author (Claude).

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

All paths are relative to the repo root (`/home/user/agent` locally). Source lives in `agent/src/`; most scan tests are inline `#[cfg(test)] mod tests` blocks inside the source files; integration-style tests live in `agent/tests/` mirroring the module tree. Read `AGENTS.md` and `ARCHITECTURE.md` at the repo root for conventions (import ordering, error idioms, test commands).

The pipeline, end to end:

1. **Rules arrive.** The sync-scan bridge (`agent/src/workers/sync_scan_bridge.rs`) resolves the deployed release's upload rules from disk (`agent/src/disk/upload_rules.rs` — a `FileCache<UploadRuleID, UploadRule>` keyed by rule id) and pushes them into the scanner actor via `update_rules(deployment, rules)`.
2. **Scanner groups by collection.** `SingleThreadScanner` (`agent/src/scan/scanner.rs`) keeps one `CollectionScanner` per `rule.upload_collection_id`; pushing a new rule version for the same collection swaps the config in place (`CollectionState::set_config`, which rejects collection-id changes) so dedup state carries across rule versions.
3. **Observation.** Each scan tick, `CollectionScanner` globs the rule's source and calls `observe_file` (`agent/src/scan/collection.rs` lines 168-189), which builds an `Observation { file, timestamp, size, mtime, deployment_id, upload_rule_id }`. **Line 187 is the bug**: `upload_rule_id: state.cfg.rule.upload_collection_id.clone()`.
4. **Stability + emission.** After the rule's stability window, an unchanged file becomes a `StableFile` (`build_stable_file`, `collection.rs` lines 272-291) copying `first_obs.upload_rule_id`, is appended to the per-file ledger, and is broadcast as `ScanEvent::StableFile`.
5. **Bridge → uploader → backend.** `agent/src/workers/scan_upload_bridge.rs` copies the `StableFile` into an upload `Job` (`agent/src/upload/job.rs`); the executor (`agent/src/upload/executor.rs`, `new_upl_request`) copies `job.upload_rule_id` into `CreateUploadRequest.upload_rule_id` and POSTs it to the backend, which looks up an Upload Rule by that id and 404s on an `upl_col_*` value.
6. **Persistence.** The scanner snapshots its whole state (`ScannerSnapshot` in `agent/src/scan/state.rs`) to `<data root>/scanner.json` after every mutation; the uploader persists its queue to `<data root>/upload_queue.json`. Both restore on restart.

Key model: `agent/src/models/upload_rule.rs` lines 78-88 — `UploadRule { id, upload_collection_id, upload_collection_name, digest, source, destination, created_at, updated_at }`. `id` is the `upl_rule_*` identity the backend requires; `upload_collection_id` is the `upl_col_*` grouping key.

Downstream pins that already exist (no changes needed, listed so the reader sees the full regression chain): `agent/tests/workers/scan_upload_bridge.rs` `stable_file_becomes_upload_job` pins `StableFile.upload_rule_id → Job.upload_rule_id` by full-struct equality; `agent/tests/upload/executor.rs` `create_request_maps_job_fields` pins `Job.upload_rule_id → CreateUploadRequest.upload_rule_id`. The missing link — rule id (not collection id) stamped into `Observation`/`StableFile` inside the scan module — is what the new tests below pin.

## Plan of Work

All edits are in `agent/src/scan/`. No public API, wire format, or persistence schema changes.

**1. The fix.** In `agent/src/scan/collection.rs`, function `observe_file` (line 187), change

    upload_rule_id: state.cfg.rule.upload_collection_id.clone(),

to

    upload_rule_id: state.cfg.rule.id.clone(),

**2. Correct `collection.rs` inline tests.**

- Test helper `rule(collection_id, glob, window)` (line ~336): pin a deterministic rule id, e.g. add `id: "upl_rule_1".to_string()` to the struct literal (currently it inherits `Default`, a random `unknown-<uuid>`). Tests that construct expected values through the production `observe_file` (the `observation` helper) are id-agnostic, but pinning keeps every emitted value deterministic. Where a helper-built fixture's `upload_rule_id` is compared against production output only via metadata (e.g. `bare_observation`, `stable_file`, `ledger_entry_matching` — all currently `"coll"`), change the value to `"upl_rule_1"` so no fixture encodes a collection-shaped value in the rule-id field.
- `stable_file_takes_identity_from_first_observation` (lines ~1314-1343): this test's *purpose* (identity from the FIRST observation across a config swap) is correct and must be kept; only the expected value is wrong. Give the two configs distinct **rule ids** as well as distinct collection ids — e.g. first config rule id `"upl_rule_first"` / collection `"upl_col_first"`, second `"upl_rule_second"` / `"upl_col_second"` (the `config` helper needs a variant that also sets `rule.id`, or set `cfg.rule.id` inline after building) — and change the assertion at line 1334 from `assert_eq!(sf.upload_rule_id, "coll1")` to `assert_eq!(sf.upload_rule_id, "upl_rule_first")`. Update the comment block above it (lines 1309-1313) to say the identity comes from the first observation's **rule id**.
- Add a focused regression test in the `discovery` or a new `observe` section: create a temp dir + file, build a `CollectionState` whose rule has `id: "upl_rule_123"` and `upload_collection_id: "upl_col_123"` (identical suffixes so only the prefix distinguishes them — a swap can never pass), call the production `observe_file`, and assert `obs.upload_rule_id == "upl_rule_123"`. Also drive the full discover→evaluate path in the same test (or a sibling) and assert the emitted `StableFile.upload_rule_id == "upl_rule_123"`.

**3. Correct `scanner.rs` inline tests.**

- `subscribe_receives_stable_file_payload` (lines ~888-924): the expected `StableFile` (full-struct equality) has `upload_rule_id: DEFAULT_COLL_ID`. The rule is built as `rule_in_collection("rule-1", DEFAULT_COLL_ID, ...)`. Change the rule id to `"upl_rule_123"`, the collection to `"upl_col_123"` (or keep `DEFAULT_COLL_ID` for the collection and just fix the expectation to `"rule-1"` — prefer the realistic distinct ids since this full-struct assertion is the actor-level regression pin), and set the expected `upload_rule_id` to the rule id.
- `update_rules_keeps_legacy_scanner_until_candidates_drain` (lines ~1141-1178): emitted pairs are collected from `stable.upload_rule_id`; expectations at lines 1170-1172 use the collection ids (`"legacy"`, `"current"`). After the fix the emitted values are the rule ids: change to `("legacy-rule", "legacy.mcap")` and `("current-rule", "current.mcap")`.
- `distinct_collections_do_not_share_dedup` (lines ~1280-1310): the emitted set at lines 1304-1309 expects collection ids `{"coll-1","coll-2"}`; rules are `"c1"`/`"c2"`. Change the expectation to `{"c1","c2"}` and rewrite the comment at lines 1297-1298 (it currently documents the bug: "upload_rule_id is stamped from the collection id in observe_file") to say the StableFiles are distinguished by their rules' ids.
- `scan_isolates_bad_glob_collection_from_emitting_sibling` (line 1361): `assert_eq!(sf.upload_rule_id, "good")` — the rule id is `"r-good"`; change the expectation to `"r-good"`.

**4. Correct `state.rs` inline test fixtures.** The `Observation`/`StableFile` fixtures at lines 224, 237, 325, 477, 538, 696 use `upload_rule_id: "coll"`. These are inert (metadata-only comparisons), but they encode a collection-shaped value in the rule-id field; change them to `"upl_rule_1"`. No assertions in `state.rs` change behavior. (The `rule()` helper here may also pin `id: "upl_rule_1"` for symmetry with `collection.rs`.)

**5. Verify no other test in the repo encodes the swap.** From the repo root run

    grep -rn "upload_rule_id" agent/src agent/tests

and confirm every remaining occurrence is either the fixed production line, a pure copy, or a rule-shaped value. `agent/tests/` fixtures already use rule-shaped values (`"rule_1"`, `"uplr_1"`, `r.id`) and need no changes.

## Concrete Steps

All commands run from the repo root (`/home/user/agent`), on branch `claude/task-mode-pr-agent-nxwei3`.

1. Make the edits in Plan of Work order (fix first, then tests). The fix is a single-line diff in `agent/src/scan/collection.rs`.

2. Run the scan-module tests fast first:

       ./scripts/test.sh scan

   (The script wraps `RUST_LOG=off cargo test --package miru-agent --features test`; a trailing filter narrows to matching test names. `--features test` is required — without it, test helpers behind `#[cfg(feature = "test")]` are missing and failures are misleading.)

   Expected: all scan tests pass. Before the production fix is applied, the new regression tests fail with an assertion like `left: "upl_col_123", right: "upl_rule_123"` — verify this once by stashing the fix (see Validation).

3. Run the full suite:

       ./scripts/test.sh

   Expected: 0 failed. Tests that bind shared OS resources are `#[serial]`; do not remove that.

4. Run coverage gates:

       ./scripts/covgate.sh

   Expected: every module at or above its `.covgate` threshold (scan: 98.83, upload: 97.00, workers: 84.67). The change adds tests and removes none, so coverage should not regress.

5. Lint locally before pushing:

       ./scripts/lint.sh

   Expected: import linter, `cargo fmt`, machete/diet, audit, and clippy all clean.

6. Commit (Conventional Commits, from the repo root of this repo):

       git add agent/src/scan
       git commit -m "fix(scan): stamp observations with the upload rule id, not the collection id"

7. Push and run preflight (`$preflight`): watch the CI workflow (`.github/workflows/ci.yml`, Lint + Test jobs) on the pushed branch head and fix any failures from the CI logs.

## Validation and Acceptance

- **Regression demonstration (fails before, passes after).** With the test edits in place but the one-line production fix reverted (`git stash push agent/src/scan/collection.rs` or temporarily undo line 187), run `./scripts/test.sh scan`: the new `observe_file` regression test and the updated `subscribe_receives_stable_file_payload` must FAIL, with the collection-shaped value (`upl_col_123`) on the wrong side of the assertion. Re-apply the fix; run `./scripts/test.sh scan` again: all pass.
- **Full suite.** `./scripts/test.sh` from the repo root: expect 0 failures. In particular these previously-wrong tests now pass with rule-id expectations: `scan::collection` `stable_file_takes_identity_from_first_observation`; `scan::scanner` `subscribe_receives_stable_file_payload`, `update_rules_keeps_legacy_scanner_until_candidates_drain`, `distinct_collections_do_not_share_dedup`, `scan_isolates_bad_glob_collection_from_emitting_sibling`.
- **Chain check (no code change, read-only).** Confirm the existing downstream pins still pass: `workers::scan_upload_bridge` `stable_file_becomes_upload_job` (StableFile→Job) and `upload::executor` `create_request_maps_job_fields` (Job→CreateUploadRequest). Together with the new scan-module pin, the rule id is asserted at every hop from observation to the wire request.
- **Coverage.** `./scripts/covgate.sh` reports scan ≥ 98.83.
- **Preflight gate (mandatory).** Preflight must report **CLEAN — CI green (Lint and Test jobs of `.github/workflows/ci.yml`) on the pushed branch head — before the PR leaves draft or the task is reported complete.** A local green run does not satisfy this gate.
- **Behavioral acceptance (staging, post-merge).** On a staging device with a deployed upload rule, drop a file matching the rule's glob; the agent log must show a successful upload line (`uploaded file ... (rule upl_rule_..., ...)`) and the backend must show the upload; the `Upload Rule with id 'upl_col_...' not found` error must no longer appear for newly scanned files. (A one-time bounded retry-then-drop 404 sequence for candidates persisted before the upgrade is expected and acceptable — see Decision Log.)

## Idempotence and Recovery

- Every step is safely repeatable: the production change is a one-line value swap; test edits are deterministic; `./scripts/test.sh`, `./scripts/covgate.sh`, and `./scripts/lint.sh` are read-only with respect to source.
- No persisted-state migration exists to get half-done: devices need no data changes (see Decision Log). Rolling back is a single-commit revert with no compat concerns — old state files remain readable in both directions because the schema is unchanged.
- If CI fails after push, fix from the CI job logs and push again. The branch carries no other in-flight work (it sits at main's head), so plain pushes suffice; never force-push once this plan's commits are on the remote.
