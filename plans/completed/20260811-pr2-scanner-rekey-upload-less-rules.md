# PR 2 — Scanner re-keying + upload-less rules

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | All changes land here (crate `miru-agent` under agent/) |
| agent (generated) | libs/backend-api | read-only | Generated wire models — untouched; wire stays v0.4 |

Second PR of the umbrella plan `plans/active/20260809-adopt-file-rules-spec-v0.5.md`.
Ships as a two-PR stack so the behavior change is not buried in rename churn:

1. `refactor/rule-scanner-rename` (base `main`, PR 1 merged as `2b24b4f`) — the
   mechanical vocabulary rename (`scan/collection.rs` -> `scan/rule.rs`,
   `CollectionScanner` -> `RuleScanner`, `CollectionState` -> `RuleState`), zero
   behavior change, scanners still keyed by collection id.
2. `feat/scanner-rekey-upload-less-rules` (base = the rename branch) — everything
   below: the re-keying, upload-less rules, event shape, and bridge gate.

Merge the rename PR first, retarget this one at `main`, then merge it.

## Purpose / Big Picture

PR 1 gave the agent a `FileRule` whose `upload` block is `Option`, but nothing exercises
the `None` arm — the scan pipeline still assumes every rule uploads, in two places:

1. `SingleThreadScanner.scanners` is keyed by `upload_collection_id`, which in v0.5 lives
   *inside* the optional `upload` block. A retention-only rule has no collection ID, so it
   has no map key.
2. The scan→upload bridge mints an upload job for every stable file it observes.

Consequently PR 1 had to skip upload-less rules outright (`warn!("scan: skipping file rule
{rule_id} with no upload block")`) — they get no scanner, are never globbed, and never
reach the ledger. This PR inverts that: **scanning is driven by the rule, uploading is one
optional consequence of it.**

- Key scanners by `FileRuleID` — always present, always unique — and drop the
  duplicate-collection-ID invariant in favour of duplicate-rule-ID.
- Scan (glob + stability window + ledger) every rule regardless of `upload`.
- Mint an upload job only when the rule has an `upload` block.
- Rename the module's collection vocabulary to rule vocabulary now that the keying is by
  rule (`CollectionScanner` → `RuleScanner`, `CollectionState` → `RuleState`,
  `scan/collection.rs` → `scan/rule.rs`).

Still no retention engine — a stable file from a retention-only rule reaches the ledger and
is emitted, and the bridge drops it. Deleting it is PR 3. Wire stays v0.4, so the adapter
cannot yet *produce* `upload: None`; this PR makes the pipeline structurally capable of it
ahead of the PR 4 wire flip.

## Progress

- [x] M1: Vocabulary rename (collection → rule) across scan/
- [x] M2: Re-key scanners by FileRuleID; invariants and snapshot shape
- [x] M3: Scan upload-less rules; gate job minting on the ScanEvent upload block
- [x] M4: Tests, covgate, preflight, push

## Surprises & Discoveries

- **`FileRuleUpload` needed `Eq`**, exactly as `FileRuleRetention` did in PR 1 and for
  the same reason: `StableFile` derives `Eq`, so `ScanEvent` carrying an
  `Option<FileRuleUpload>` cannot derive it otherwise. All fields are `String`, so the
  derive is sound.
- **The commit split is by path, not by compilability** — same honest split PR 1 landed
  with. The `ScanEvent` shape change lives in `scan/scanner.rs` alongside the re-keying,
  so the scan-module commit and the bridge commit are mutually dependent. Splitting them
  to compile independently would mean two passes over `scanner.rs`.
- **Three test helpers were keyed to collection ids and had to invert, not just rename.**
  `active_collections` returned `upload_collection_id` values; under rule keying the
  scanner map's keys are rule ids, so it became `active_rule_ids` over `rule_ids()`. Two
  tests (`update_rules_updates_in_place_carrying_state`,
  `update_rules_resnapshots_preexisting`) deliberately changed the rule id while holding
  the collection id fixed, to prove state carries across a redeploy — under rule keying
  that now creates a *second* scanner, so both were rewritten to hold the rule id fixed
  and vary the digest/glob instead. The behavior under test is unchanged; only the
  identity that expresses "same scanner entry" moved.
- **`update_rules_skips_rule_without_upload` inverted into
  `update_rules_scans_rule_without_upload`** — the clearest single marker of what this PR
  changes. PR 1 asserted an upload-less rule produces no scanner; PR 2 asserts it
  produces one.
- **`collection_ids` survived as a test helper.** It went unused once `active_rule_ids`
  stopped calling it, but rather than delete it, it now backs
  `update_rules_allows_shared_collection_id` — asserting both rules keep the shared
  collection id while occupying separate scanner slots.

## Decision Log

- **Same-rule re-push refreshes only the deployment (Ben's call, 2026-08-11).** A rule's
  content is immutable per id (content-digested; edits mint a new id), so the only real
  delta on the every-sync re-push path is the deployment stamped onto observations —
  which is load-bearing (`CreateUploadRequest.deployment_id`). `update_config` — which
  also re-globbed and re-snapshotted `preexisting` on every push — is replaced by
  `RuleScanner::set_deployment`; `RuleState::set_config` and the `InvalidRule` error are
  deleted. This fixes a latent lost-file window that predates this PR: a file appearing
  between a scan tick and a sync was swallowed into `preexisting` and never uploaded
  (pinned by `update_rules_repush_does_not_swallow_new_files`). Known semantic change:
  files appearing while a rule is undeployed are now uploaded when the same rule id is
  later redeployed, instead of being suppressed as preexisting — preexisting-backlog
  suppression now happens exactly once, at a rule's first deploy.
- **`RuleScanner::rule()` is no longer test-gated.** The scan tick needs it in
  production to attach the upload block to emitted events; the `#[cfg(feature = "test")]`
  gate made the no-features build fail (unnoticed because tests, clippy, and CI all run
  with the test feature enabled).

- **Bridge gating carries `Option<FileRuleUpload>`, not a bool.** `ScanEvent::StableFile`
  becomes a struct variant `{ file: StableFile, upload: Option<FileRuleUpload> }`. Rejected
  a bool flag as lossy, and rejected putting the field on `StableFile` itself: `StableFile`
  is persisted in the ledger, so routing metadata there would duplicate per entry and go
  stale against the rule that produced it. The event carries it because the event is
  transient and the rule is in scope at emit time. Cost is cloning five `String`s per
  stable file; the bridge gate reads as `if let Some(upload)`.
- **Stale snapshots are invalidated by renaming the persisted field, not by migration
  code.** Re-keying `ScannerSnapshot.collections` from collection ID to rule ID is a silent
  semantic change — both are `String`, so an old `scanner.json` would deserialize with
  wrong keys and stale ledgers could re-upload already-confirmed files. Renaming the field
  to `rules` makes old snapshots fail to parse, and `ScanSnapshotFile::new_with_default`
  overwrites with the default on any read error. This is the reset-on-parse-failure path
  the umbrella plan designates (zero production users).
- **`DuplicateCollectionID` → `DuplicateFileRuleID`.** The invariant is real and worth
  keeping; it just moves to a field that exists on every rule. Duplicate rule IDs in one
  deployment remain a hard error.
- **`InvalidRule` re-targets to rule ID.** Keying by rule ID makes a rule-ID change
  structurally impossible through `set_config`, but the check stays as defense in depth
  (and holds the scan module's 98.83% covgate).
- **Vocabulary rename ships as its own preparatory PR**, not folded in here and not
  deferred to PR 5 (revised from the first draft of this plan, which folded it in: the
  mixed diff buried the two real behavior changes in rename churn). The types are
  `pub(crate)` and confined to `scan/`, so the rename PR is provably a no-op, and this
  PR's diff is behavior-only.

## Context and Orientation

### Current shape (main @ 2b24b4f)

- `agent/src/scan/scanner.rs:56` — `scanners: HashMap<UploadCollectionID, CollectionScanner>`
- `agent/src/scan/scanner.rs:57` — `deployed: HashSet<UploadCollectionID>`
- `agent/src/scan/scanner.rs:160-170` — duplicate-collection-ID check, skips `upload: None`
- `agent/src/scan/scanner.rs:173-192` — `update_rules` skips `upload: None` with a warn
- `agent/src/scan/scanner.rs:215-243` — scan tick iterates by `cid`; prunes inactive
- `agent/src/scan/state.rs:74-92` — `set_config` rejects a collection-ID change
- `agent/src/scan/state.rs:165-169` — `ScannerSnapshot { collections, deployed }`
- `agent/src/scan/scanner.rs:23-25` — `ScanEvent::StableFile(StableFile)` tuple variant
- `agent/src/workers/scan_upload_bridge.rs:70-86` — enqueues a Job for every stable file

### Invariants that must hold after this PR

- A rule with an `upload` block behaves exactly as today: globbed, stability-windowed,
  ledgered, emitted, and enqueued as an upload job.
- A rule without an `upload` block is globbed, stability-windowed, ledgered, and emitted —
  and produces **no** upload job.
- Two rules in one deployment sharing an `upload_collection_id` is now legal (collection
  IDs are optional and no longer keys); two rules sharing an ID is not.
- `Job` is unchanged — the backend resolves the destination from `file_rule_id`, so no
  upload fields need to reach the executor.

## Plan of Work

### M1 — Vocabulary rename (ships as the base PR)
`git mv agent/src/scan/collection.rs agent/src/scan/rule.rs`; `CollectionScanner` →
`RuleScanner`, `CollectionState` → `RuleState`, `scan::collection` → `scan::rule`.
Mechanical; no behavior change; lands as `refactor/rule-scanner-rename`. Locals
(`cid` → `rule_id`) and log strings move in M2 with the keying they describe.

### M2 — Re-key and invariants
Map/set key type `UploadCollectionID` → `FileRuleID`; `ScannerSnapshot.collections` →
`rules`; `DuplicateCollectionID` → `DuplicateFileRuleID`; `InvalidRule` fields to rule ID.

### M3 — Upload-less rules
Remove both `let Some(upload) = &rule.upload else { continue }` guards in `update_rules`.
`ScanEvent::StableFile` becomes a struct variant carrying `upload: Option<FileRuleUpload>`,
populated from `scanner.rule().upload` at emit time. Bridge enqueues only on `Some`.

### M4 — Tests and validation
Update inline tests in scan/ and the bridge tests in `agent/tests/`. Add coverage for:
retention-only rule scans and ledgers but mints no job; two rules sharing a collection ID
coexist; duplicate rule IDs error; stale collection-keyed `scanner.json` resets.

## Validation and Acceptance

- `./scripts/test.sh` green.
- `scripts/covgate.sh` — scan module holds ≥ 98.83.
- `scripts/lint.sh` clean (import linter, fmt, machete, clippy).
- Behavior for upload-bearing rules is unchanged: verified by the existing scan and bridge
  test suites passing without semantic edits (only rename/shape churn).
