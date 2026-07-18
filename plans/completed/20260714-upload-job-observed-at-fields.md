# Add `first_observed_at` / `last_observed_at` fields to the upload `Job` struct

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Add two `DateTime<Utc>` fields to `Job` in `agent/src/upload/job.rs` and extend the single construction site in `agent/tests/upload/queue.rs`. |
| `libs/backend-api/` | read-only | Generated OpenAPI code; `src/models/upload_source.rs` motivates the fields but is not modified. |

This plan lives in the agent repo because all changes are inside it. Work happens on branch `feat/upload-job-observed-at` (already checked out, clean, based on `main`).

## Purpose / Big Picture

An upload `Job` (`agent/src/upload/job.rs`) is the unit of work the upload pipeline carries from the scanner toward the backend. When the agent mints an upload against the backend, the request's `UploadSource` model (`libs/backend-api/src/models/upload_source.rs`, generated from the backend OpenAPI spec) requires two timestamps: `first_observed_at` (when the device agent first observed the file) and `last_observed_at` (the most recent observation before the upload was minted). The scanner already tracks both on `StableFile` (`agent/src/scan/state.rs` lines 129–130), but `Job` cannot carry them today, so the mint request cannot be populated.

After this change, `Job` has `first_observed_at: DateTime<Utc>` and `last_observed_at: DateTime<Utc>` fields with doc comments explaining their provenance, and the existing queue tests pass with the extended struct. Nothing consumes the fields yet — the producer that fills them from real scanner observations, and the executor/transfer code that stamps them into the mint request, are future PRs (executor work lives on open PR #147 and must not be touched here). Do not add any speculative plumbing beyond the two fields.

## Progress

- [x] Milestone 1: add fields, update the test helper, validate, commit. (2026-07-14: fields added to `agent/src/upload/job.rs`, `make_job` extended in `agent/tests/upload/queue.rs`; `cargo test --locked --features test upload` passed 43/43 and `cargo fmt -p miru-agent -- --check` clean. Full preflight runs in a separate follow-up pass.)

## Surprises & Discoveries

- None. The construction-site grep from Context and Orientation was re-run before editing and still returned exactly two hits (the struct definition and `make_job`); the compiler confirmed no other construction sites.

## Decision Log

- Decision: Do not add a new test for the fields. `Job` is plain data with derived `PartialEq`; the queue tests in `agent/tests/upload/queue.rs` already assert whole `Job` structs (`assert_eq!(first.job, job_a)` in `requeue::preserves_attempts_and_appends_at_tail`, `assert_eq!(entry.job, expected)` in `pop_front::returns_jobs_in_fifo_order`), so the new fields round-trip through the queue under existing assertions. A dedicated field round-trip test would be vacuous and would fight the repo's prefer-whole-struct-assertion convention.
  Rationale: existing whole-struct assertions already cover the fields; the repo lints against field-by-field asserts. Date/Author: 2026-07-14 / ben@miruml.com.
- Decision: No serde derives, constructors, or conversion helpers are added.
  Rationale: `Job` has none today (`#[derive(Clone, Debug, PartialEq)]` only, constructed by struct literal); the mapping to `UploadSource` belongs to the future executor PR. Date/Author: 2026-07-14 / ben@miruml.com.

## Outcomes & Retrospective

`Job` now carries `first_observed_at` and `last_observed_at` (`DateTime<Utc>`) with provenance doc comments, placed after `mtime`. The only construction site (`make_job` in `agent/tests/upload/queue.rs`) was extended with `Utc::now()` values; existing whole-struct queue assertions cover the new fields with no new tests, per the Decision Log. No serde derives, constructors, or producer/executor plumbing were added — mapping `StableFile` observations into `Job` and stamping `UploadSource` remain future PRs.

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (a Rust workspace; the binary crate is `miru-agent` under `agent/`). All paths are relative to the repo root; all commands run from the repo root.

- `agent/src/upload/job.rs` — the whole file is the `Job` struct (7 fields: `file`, `size`, `digest`, `mtime`, `upload_rule_id`, `deployment_id`, `release_id`), deriving `Clone, Debug, PartialEq`. `chrono::{DateTime, Utc}` is already imported.
- `agent/tests/upload/queue.rs` — the only place a `Job` is constructed on `main` (verified with `grep -rn "Job {" agent/src agent/tests`): the `make_job(name: &str) -> Job` helper at the top, used by every queue test via struct literal. Re-verify this grep before editing in case `main` has moved.
- `agent/src/scan/state.rs` lines 129–130 — `StableFile.first_observed_at` / `.last_observed_at`, the scanner-side source of truth these fields mirror.
- `libs/backend-api/src/models/upload_source.rs` — generated `UploadSource` with required `first_observed_at` / `last_observed_at` string fields; read-only context.

Out of scope: executor/transfer code (open PR #147), the producer that maps `StableFile` → `Job`, and any change under `libs/`.

## Plan of Work

1. In `agent/src/upload/job.rs`, add two fields to `Job` (placement after `mtime` keeps the timestamps together):

       /// When the agent first observed the file on the device. Mirrors the
       /// scanner's observation timestamps (`StableFile.first_observed_at` in
       /// `agent/src/scan/state.rs`) and is stamped into the backend upload
       /// mint request's `UploadSource.first_observed_at` (required field).
       pub first_observed_at: DateTime<Utc>,
       /// The most recent observation of the file before the upload was
       /// minted. Mirrors `StableFile.last_observed_at` in
       /// `agent/src/scan/state.rs` and is stamped into the backend upload
       /// mint request's `UploadSource.last_observed_at` (required field).
       pub last_observed_at: DateTime<Utc>,

2. In `agent/tests/upload/queue.rs`, extend the `make_job` struct literal with `first_observed_at: Utc::now(),` and `last_observed_at: Utc::now(),`. No other test changes (see Decision Log).

## Concrete Steps

All commands run from the repo root on branch `feat/upload-job-observed-at`.

Step 1 — confirm the construction-site inventory is still current (expect exactly two hits: the struct definition and `make_job`):

    grep -rn "Job {" agent/src agent/tests --include="*.rs" | grep -v "JobError"

Step 2 — make the two edits described in Plan of Work.

Step 3 — build and test:

    cargo build -p miru-agent
    ./scripts/test.sh

Expected: build clean; full suite passes with an unchanged test count. `./scripts/test.sh` wraps `RUST_LOG=off cargo test --features test`; the `--features test` flag is mandatory — never invoke bare `cargo test`.

Step 4 — preflight:

    ./scripts/preflight.sh

Expected: final line reports `Preflight clean`.

Step 5 — commit (end of Milestone 1):

    git add agent/src/upload/job.rs agent/tests/upload/queue.rs
    git commit -m "feat(upload): add observed-at timestamps to upload Job"

## Validation and Acceptance

1. Before the edit, adding the fields alone makes `cargo build -p miru-agent --features test` fail with `missing fields first_observed_at and last_observed_at` at the `make_job` literal in `agent/tests/upload/queue.rs`; after Step 2 it compiles. This proves the grep-verified inventory is complete — the compiler finds any construction site the grep missed.
2. `./scripts/test.sh` passes with zero failures; the existing whole-struct assertions in the `requeue` and `pop_front` queue tests now compare the new fields as part of `Job` equality. No new tests are added.
3. `./scripts/preflight.sh` reports `Preflight clean`. This is required before publishing the branch.

## Idempotence and Recovery

Every step is safe to re-run: the edits are plain additive field insertions (re-applying is a no-op once present), and build/test/preflight are read-only. If an edit goes wrong before committing, `git checkout -- agent/src/upload/job.rs agent/tests/upload/queue.rs` restores a clean tree; the branch exists only for this work, so `git reset --hard main` is a safe full rollback prior to push.
