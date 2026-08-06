# Delete uploaded files on-device once their delete delay elapses, via a dedicated delete worker

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Add a **delete worker**: a new `agent/src/delete/` actor module owning a persisted pending-deletion queue, a new interval driver in `agent/src/workers/delete.rs`, an internal-only `delete_delay_secs` threaded from `UploadRuleDestination` through `StableFile` and `Job`, executor integration replacing the inline post-upload delete, and app wiring in `agent/src/app/`. Mirror test files under `agent/tests/`. |
| `libs/backend-api/` | read-only | Generated OpenAPI client. The wire `UploadRuleDestination` has NO `delete_delay_secs`; the backend→internal mapping sets it to 0. Not modified, not regenerated. |
| `api/specs/` | read-only | OpenAPI specs. Not modified — the wire change ships separately (openapi PR #212). |

This plan lives in the agent repo because every code change is inside `agent/`. Work happens on branch `feat/delete-worker` (already checked out, clean, based on `main`).

## Purpose / Big Picture

Today, when an upload rule's delete policy is `after_upload`, the agent deletes the local source file inline, immediately after the backend confirms the upload (`agent/src/upload/executor.rs::delete_source_file`). There is no way to keep the file around for a grace period after upload, and deletion knowledge is buried inside the upload executor.

After this change, deletion is a first-class worker with a **delete delay**: when an upload is confirmed for an `after_upload` rule, the executor enqueues a persisted *pending deletion* record — "this exact file (path, size, mtime, digest) became deletable at `eligible_at`; delete it once `delete_delay_secs` have passed". A `Deleter` actor sweeps its queue on a fixed cadence (default 60s), deleting each due file only if it is provably unchanged since upload. With `delete_delay_secs = 0` (the only value the backend can express today) behavior is preserved to within one sweep interval.

The record is deliberately event-agnostic: it names *when the file became deletable*, not *why*. Openapi PR #212 (not merged) generalizes delete policies into retention policies (`never`, `after_stable`, `after_upload`, `after_upload_or_expiry` + `expiry_secs`). When that lands, new eligibility events (stabilization, expiry) simply become new producers of the same `PendingDelete` record — the worker itself does not change.

You can see it work by running the test suite: sweep tests drive a `Deleter` with an injected clock and assert a due, unchanged file is deleted, a not-yet-due file is kept, and a modified file is dropped without deletion; an executor test asserts a confirmed `after_upload` upload enqueues a pending deletion instead of deleting inline.

## Progress

- [ ] Milestone 1: thread `delete_delay_secs` (internal-only, default 0) through `UploadRuleDestination` → `StableFile` → `Job`; serde back-compat tests.
- [ ] Milestone 2: new `agent/src/delete/` module — `PendingDelete`, persisted queue, sweep semantics, `Deleter` actor with `DeleterExt`; in-src sweep tests + actor tests + `.covgate`.
- [ ] Milestone 3: executor integration — `LiveExecutor` enqueues a `PendingDelete` instead of deleting inline; executor tests updated.
- [ ] Milestone 4: interval driver `agent/src/workers/delete.rs` mirroring `workers/scan.rs`; `MockDeleter` + driver tests.
- [ ] Milestone 5: app wiring — `AppOptions`, `AppState` (spawn + shutdown ordering), `run.rs` init/shutdown/duplicate-guard tests.
- [ ] Milestone 6: preflight to CI-green on the pushed branch head.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go. Planning-time decisions are recorded below.)

- Decision: Build against the agent's CURRENT internal types (`DeletePolicy { Never, AfterUpload }`) plus a NEW internal-only `delete_delay_secs: i64` (serde default 0; backend→internal mapping hardcodes 0), with NO spec/API changes — no edits to `api/specs/`, no regeneration of `libs/backend-api`.
  Rationale: The concept comes from openapi PR #212 (mirurobotics/openapi), which is OPEN and not merged. #212 renames upload rules to file rules and replaces `delete_policy` with a required `retention` block (`policy` enum naming the eligibility event — `never`/`after_stable`/`after_upload`/`after_upload_or_expiry` — plus `delete_delay_secs` from the eligibility event and `expiry_secs` from last modification as the backstop for permanently-failed uploads, which the agent drops after 9 attempts). Coding against an unmerged spec would couple this branch to a moving target. Instead the worker's persisted unit of work is an event-agnostic `PendingDelete` carrying `eligible_at` + `delete_delay_secs`, so #212's generalization layers on later purely by adding new producers of the same record — no rework of the worker. Today the only expressible eligibility event is a confirmed upload under `AfterUpload`, and the only expressible delay is 0. Date/Author: 2026-08-05 / ben@miruml.com.
- Decision: Replace the executor's inline `delete_source_file` with enqueueing onto the `Deleter`; with `delete_delay_secs == 0` the file is deleted on the next sweep after confirmation instead of inline (a delay of at most one sweep interval, default 60s).
  Rationale: A single deletion path. The worker is the only component that ever deletes rule-matched files, which is exactly the shape #212's retention block needs (retention is enforced on-device even with no release deployed). The small timing change is deliberate and harmless: the file was already uploaded durably, and nothing reads it between confirm and sweep. Enqueue failure must not fail the upload (it is already confirmed) — log and continue, mirroring today's best-effort delete. Date/Author: 2026-08-05 / ben@miruml.com.
- Decision: Path-safety invariant — the worker deletes ONLY paths present in its persisted queue, and records only ever enter the queue from the upload executor after a confirmed upload of a rule-matched file. The worker never globs or walks the filesystem for deletion candidates. Before deleting, it re-stats the file and deletes only if size and mtime still match the recorded values; on mismatch it drops the entry WITHOUT deleting.
  Rationale: The repo has no canonicalize-under-root containment helper (`agent/src/filesys/path.rs` is lexical-only), so queue-only provenance is the guardrail against deleting anything the pipeline did not upload. The metadata re-check protects a file modified after upload: the scanner will re-observe it, re-stabilize, re-upload, and a fresh pending deletion is enqueued after that upload. When unsure, never delete. Date/Author: 2026-08-05 / ben@miruml.com.
- Decision: The `Deleter` actor spawns whenever the uploader does (the executor needs its handle); `AppOptions.enable_delete_worker` gates only the interval driver. A deleter spawn failure degrades to uploads-without-deletion (executor holds `Option<Arc<D>>`), never to a boot failure.
  Rationale: Matches the fail-open pattern of `AppState::init_scanner`/`init_uploader` (`agent/src/app/state.rs:132-225`) — the agent must boot even when an optional subsystem cannot. Date/Author: 2026-08-05 / ben@miruml.com.

## Outcomes & Retrospective

(Fill in on completion.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (a Rust workspace, edition 2021; the binary crate is `miru-agent` under `agent/`). All paths are repo-relative; all commands run from the repo root. Branch `feat/delete-worker` is checked out, clean, identical to `main`. Note: `ARCHITECTURE.md` is partly stale — trust the code.

The **upload pipeline** today:

1. The `Scanner` actor (`agent/src/scan/scanner.rs`) polls configured globs; a file stable for its rule's window is appended to a persisted ledger and emitted as `ScanEvent::StableFile(StableFile)` (broadcast, cap 256).
2. The scan-upload bridge (`agent/src/workers/scan_upload_bridge.rs::enqueue_stable_file`, line 70) maps each `StableFile` to a `Job` field-by-field and enqueues it on the `Uploader` actor.
3. The `Uploader` actor (`agent/src/upload/uploader.rs`) pops its persisted queue and calls `executor.upload(&job)` with retries (max 9 total attempts, then the job is dropped).
4. `LiveExecutor::upload` (`agent/src/upload/executor.rs:99-127`) does `create_upload` → `transfer` → `confirm_upload` → `delete_source_file(job)`. `delete_source_file` (lines 82-95) deletes iff `job.delete_policy == DeletePolicy::AfterUpload`, best-effort (`warn!` and swallow). **This inline delete is what this plan replaces.**

Key files and current state (verified on `main`):

- `agent/src/models/upload_rule.rs` — `DeletePolicy { #[default] Never, AfterUpload }` (lines 14-20), mapped to the backend enum by `impl_status_enum!` (lines 22-36, unknown → `Never`). `UploadRuleDestination` (lines 55-61): `bucket_id`, `bucket_name`, `path`, `delete_policy`; its `From<backend_client::UploadRuleDestination>` impl is at lines 63-72. Grep confirms NO `delete_delay_secs`, `uploaded_at`, or "deletable" concept exists anywhere in source, generated client, or specs.
- `agent/src/scan/state.rs` — `StableFile` (lines 123-137) is PERSISTED (inside `CollectionState.ledger` → `ScannerSnapshot` → `scanner.json`), so any new field needs `#[serde(default)]` to read old snapshots. `delete_policy` (line 135-136) is the precedent.
- `agent/src/scan/collection.rs` — `build_stable_file` (line 308) stamps `delete_policy` from the rule; the new field follows the same route.
- `agent/src/upload/job.rs` — `Job` struct (9 fields incl. `delete_policy`), serde-derived, persisted inside `upload_queue.json` via `QueueSnapshotFile` (`agent/src/upload/queue.rs:36`).
- `agent/src/upload/executor.rs` — `UploadExecutor` trait (line 31) with a documented cancel-safety contract; `LiveExecutor<C, T, X>` is generic over its collaborators (the repo's pattern — its `Ext` traits use `async fn`/`impl Future` and are not dyn-compatible, so collaborators are generic params, never trait objects).
- `agent/src/upload/uploader.rs` — the actor template: `UploaderExt { enqueue, len, shutdown }` (line 87), `Command` enum + `oneshot` responders, `dispatch!` macro, `Worker::run`, `Uploader::spawn(buffer_size, executor, options, snapshot_file, sleep_fn) -> Result<(Uploader, JoinHandle<()>), _>`.
- `agent/src/scan/scanner.rs` — the simpler actor template the `Deleter` mirrors: `ScannerArgs { now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>, broadcast_capacity, snapshot_file }` (lines 39-53, the mockable-clock precedent), `Command` enum (line 264), `Worker` (line 292), `Scanner::spawn` (line 373), `ScannerExt` impl via `send_command` (lines 387-453).
- `agent/src/filesys/state_file.rs` — `SingleThreadStateFile<ContentT, PatchT>`: `new_with_default(file, default)` (line 54), `read() -> Arc<ContentT>` (line 86), `patch(patch)` (line 96, writes only on change, atomic). Pattern behind `scanner.json` and `upload_queue.json`.
- `agent/src/filesys/files.rs` — `delete(file)` treats `NotFound` as `Ok(())` (idempotent); `metadata`, `size`, `last_modified` for the re-stat.
- `agent/src/disk/layout.rs` — `Layout` under `<fs_root>/var/lib/miru/`: `scanner_snapshot()` (line 45), `upload_queue()` (line 49). The delete queue file joins them.
- `agent/src/app/options.rs` — `AppOptions` uses `enable_<worker>: bool` + `<worker>: <mod>::Options` pairs (lines 38-59).
- `agent/src/app/state.rs` — `AppState::init` spawns actors; `init_scanner` (line 136) and `init_uploader` (line 179) are the fail-open templates; `shutdown()` (line 227) orders: uploader → scanner → syncer → event_hub → storage → token_mngr.
- `agent/src/app/run.rs` — worker spawn template `init_scan_worker` (line 310); `ShutdownManager` (line 427) with one `Option<JoinHandle>` slot per worker, `register_handle` rejecting duplicates (line 479); `shutdown_impl` (line 538) joins workers in numbered steps 1-7 then app state as step 8. Six `register_handle_rejects_*_duplicates` tests (lines 745-910) and `shutdown_impl_maps_*_join_error` tests are the templates for the new worker's tests. Shutdown primitive is `tokio::sync::broadcast::channel::<()>(1)`; each worker receives `Box::pin(async move { let _ = shutdown_rx.recv().await; })`.
- `agent/src/workers/scan.rs` — the interval-driver template (whole file, 70 lines): `Options { scan_interval_secs: i64 }` default 60, `run(options, scanner, sleep_fn, shutdown_signal)` with `tokio::select!`, immediate initial pass, errors logged and swallowed.

Test layout: `./scripts/test.sh` runs `RUST_LOG=off cargo test --features test`; `--features test` is MANDATORY. `agent/tests/` is one integration binary — new modules must be listed in `agent/tests/mod.rs` and `agent/tests/workers/mod.rs`. Mocks live in `agent/tests/mocks/` (registered in `mocks/mod.rs`): `MockScanner` (`num_scan_calls`, `set_scan`) and `SleepController` (`agent/tests/mocks/error.rs`: `sleep_fn()`/`await_sleep()`/`release()`) drive the scan-driver tests in `agent/tests/workers/scan.rs`; `MockUploadExecutor` + a `Harness` with `within`/`wait_until` helpers drive `agent/tests/workers/scan_upload_bridge.rs`. Executor tests (`agent/tests/upload/executor.rs`) write real temp files via `filesys::files`. Scanner sweep-style logic is tested IN-SRC (`#[cfg(test)] mod tests` with an injected `Clock` over `AtomicI64` — see `agent/src/scan/scanner.rs:475+`). Use `#[serial]` (serial_test) only for shared-OS-resource tests.

Errors: `thiserror` structs + `crate::errors::Error` + `impl_error!` for the aggregating enum (template: `agent/src/upload/errors.rs`). Logging: `tracing` only, message prefix `"delete: ..."`. Lint (`scripts/lint.sh` + `tools/lint`): three import groups with `// standard crates` / `// internal crates` / `// external crates` headers; the field-by-field-assert linter flags 4+ `assert_eq!` on one variable's fields per test (prefer whole-struct asserts, or `// lint:allow(field-by-field-assert)`).

Coverage: per-module `.covgate` minimums enforced by `scripts/covgate.sh` — `workers` 84.67, `upload` 96.00, `scan` 98.83, `models` 100, `app` 90.38, `disk` 96.79. New `agent/src/delete/` needs its own `.covgate`. Known gotcha: `agent/src/workers/.covgate` (84.67) can fail LOCALLY (~83) even on unrelated branches — do not lower it; CI is authoritative.

CI (`.github/workflows/ci.yml`): three jobs — `lint` (`LINT_FIX=0 ./scripts/lint.sh`), `test` (`./scripts/covgate.sh`), `tools`. `scripts/preflight.sh` runs lint + covgate + tools-lint + tools-covgate in parallel and prints "Preflight clean".

**Out of scope:** openapi PR #212 wire changes and vendoring; any edit under `api/specs/` or `libs/backend-api` (never hand-edited); the `after_stable` and `after_upload_or_expiry` retention policies; `expiry_secs` and any expiry clock. The `PendingDelete` record (event-agnostic `eligible_at` + `delete_delay_secs`) deliberately leaves room for all of these as future producers, but this plan implements only the after-upload producer with delay 0 on the wire.

## Plan of Work

Threaded producer-first; each milestone compiles under `--features test` and is independently testable.

### Milestone 1 — thread `delete_delay_secs` through rule → `StableFile` → `Job`

1. `agent/src/models/upload_rule.rs`: add to `UploadRuleDestination` (after `delete_policy`):

       /// Seconds to keep the file after it becomes deletable (i.e. after a
       /// confirmed upload under `AfterUpload`). Internal-only until openapi
       /// #212 lands: the backend cannot express it yet, so the wire mapping
       /// sets 0 (delete on the next sweep). `#[serde(default)]` keeps cached
       /// `upload_rules.json` written by older agents deserializable.
       #[serde(default)]
       pub delete_delay_secs: i64,

   In the `From<backend_client::UploadRuleDestination>` impl set `delete_delay_secs: 0` with a one-line comment citing #212. `Default` derive gives 0 automatically.
2. `agent/src/scan/state.rs`: add `#[serde(default)] pub delete_delay_secs: i64` to `StableFile` after `delete_policy` (old `scanner.json` snapshots lack it).
3. `agent/src/scan/collection.rs`: `build_stable_file` gains the stamp — either pass `state.rule().destination.delete_delay_secs` alongside the existing `delete_policy` argument from `differs_from_previous` (line 285), or (cleaner, if the parameter list grows awkward) pass `&UploadRuleDestination` once and stamp both fields from it. Keep the change minimal.
4. `agent/src/upload/job.rs`: add `pub delete_delay_secs: i64` to `Job`; `agent/src/workers/scan_upload_bridge.rs::enqueue_stable_file`: copy `delete_delay_secs: stable.delete_delay_secs`.
5. Build with `--features test`; the compiler flags every `UploadRuleDestination`/`StableFile`/`Job` struct literal (in-src `#[cfg(test)]` modules of `scan/state.rs`, `scan/collection.rs`, `scan/scanner.rs`; `agent/tests/models/upload_rule.rs`; `agent/tests/upload/{executor,uploader,queue}.rs`; `agent/tests/workers/scan_upload_bridge.rs`). Add `delete_delay_secs: 0` (or the value under test) everywhere flagged.
6. Tests: in `agent/tests/models/upload_rule.rs` extend the existing backend-mapping test (line 105+) to assert the mapped `delete_delay_secs == 0`, and extend the serde back-compat JSON test (line 46 area, which omits new fields) to assert the default 0. In `agent/src/scan/state.rs` extend the existing `StableFile`-without-field serde test pattern for `delete_delay_secs`. In `agent/src/scan/collection.rs` extend the stamping test: a rule destination with `delete_delay_secs: 300` yields a `StableFile` with 300. In `agent/tests/workers/scan_upload_bridge.rs` extend the whole-struct `assert_eq!(expected, jobs[0])` with the copied field.

### Milestone 2 — the `delete` module: record, queue, sweep, actor

1. Create `agent/src/delete/mod.rs`:

       pub mod deleter;
       pub mod errors;
       pub mod queue;

       pub use self::deleter::{Deleter, DeleterArgs, DeleterExt};
       pub use self::errors::DeleteErr;
       pub use self::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile, PendingDelete};

   Register `pub mod delete;` in `agent/src/lib.rs` (alphabetical: between `crypt` and `deploy` — keep the list sorted).
2. `agent/src/delete/errors.rs`: `SendActorMessageErr`/`ReceiveActorMessageErr` aliases (as in `upload/errors.rs:4-5`), a `QueueFullErr { capacity, file, trace }`, and `enum DeleteErr { QueueFullErr, SendActorMessageErr, ReceiveActorMessageErr }` wired with `crate::impl_error!`.
3. `agent/src/delete/queue.rs`: the persisted record and snapshot:

       #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
       pub struct PendingDelete {
           /// The exact file the executor uploaded. The worker only ever
           /// deletes paths carried by these records (path-safety invariant).
           pub file: File,
           /// Size/mtime/digest as recorded at upload time (from the `Job`).
           /// The sweep re-stats and deletes only on a size+mtime match.
           pub size: u64,
           pub mtime: DateTime<Utc>,
           pub digest: String,
           /// When the file became deletable (today: upload confirmation).
           /// Event-agnostic on purpose — future retention policies (#212)
           /// add producers, not fields.
           pub eligible_at: DateTime<Utc>,
           pub delete_delay_secs: i64,
           // for logging only
           pub upload_rule_id: String,
           pub deployment_id: String,
       }

       #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
       pub struct DeleteQueueSnapshot { pub entries: Vec<PendingDelete> }

   `impl Patch<DeleteQueueSnapshot> for DeleteQueueSnapshot` (whole-snapshot replace, as `upload/queue.rs:28-32`) and `pub type DeleteQueueSnapshotFile = SingleThreadStateFile<DeleteQueueSnapshot, DeleteQueueSnapshot>;`. Add `PendingDelete::due_at(&self) -> DateTime<Utc>` returning `eligible_at + chrono::Duration::seconds(delete_delay_secs.max(0))`.
4. `agent/src/disk/layout.rs`: add next to `upload_queue()`:

       pub fn delete_queue(&self) -> filesys::File {
           self.root().file("delete_queue.json")
       }

5. `agent/src/delete/deleter.rs`, mirroring the scanner actor (`scan/scanner.rs`) closely:
   - `DeleterArgs { now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>, queue_capacity: usize, snapshot_file: Option<DeleteQueueSnapshotFile> }` with `Default` (`Utc::now`, 4096, `None`).
   - `SingleThreadDeleter`: seeds `entries: Vec<PendingDelete>` from the snapshot; `enqueue(pd)` first removes any existing entry with the same `file` (newest record wins — bounds the queue to one entry per path), rejects with `QueueFullErr` at capacity (`warn!`; the file simply stays on disk — the safe direction), then persists; `sweep()` walks entries with `now = (self.now_fn)()`:
     - not due (`now < due_at()`) → keep.
     - due → re-stat via `files::metadata` (returns `std::fs::Metadata`; convert `.modified()`'s `SystemTime` to `DateTime<Utc>` via `DateTime::<Utc>::from(...)`, exactly as `build_stable_file` does at `agent/src/scan/collection.rs:319` — never compare a `SystemTime` to a `DateTime<Utc>` directly). Missing (`NotFound`) → drop the entry (`info!`; already gone, success). Size or converted mtime differs from the record → drop WITHOUT deleting (`info!("delete: {} changed since upload; dropping without deleting", ...)`); the scanner re-observes, re-uploads, and a fresh record arrives. Matches → `files::delete`; on `Ok` drop the entry (`info!`), on `Err` keep it and `warn!` (retried next sweep — never crashes the loop).
     - persist the snapshot once after the pass if anything changed. `sweep()` returns `Ok(())`; per-entry failures are logged, not propagated.
   - `trait DeleterExt` (with `#[allow(async_fn_in_trait)]` + `#[allow(clippy::len_without_is_empty)]`): `enqueue(PendingDelete)`, `sweep()`, `len()`, `shutdown()`, all `-> Result<_, DeleteErr>`.
   - `Command` enum + `Worker { deleter, receiver }` + `dispatch!` + `Deleter { sender }` handle with `pub fn spawn(buffer_size: usize, args: DeleterArgs) -> Result<(Deleter, JoinHandle<()>), DeleteErr>` and a `ScannerExt`-style `send_command` impl of `DeleterExt`.
6. Tests. In-src `#[cfg(test)] mod tests` in `deleter.rs` (mirroring `scan/scanner.rs`'s `Clock` over `AtomicI64`) covering, against `SingleThreadDeleter` with real temp files (`files::temp` + `files::write_bytes`, the same helpers `agent/tests/upload/executor.rs:342` uses): zero-delay entry deleted on first sweep; positive-delay entry kept until the clock advances past `due_at`, then deleted; size-changed and mtime-changed files dropped without deletion (file still exists); missing file dropped as success; delete-failure entry retained (point the record at a path whose parent is a file so `remove_file` fails `NotADirectory`, not `NotFound`); enqueue same-path replacement; capacity rejection; snapshot persistence across a rebuild (`SingleThreadDeleter::new` from the same file re-seeds entries). Integration tests in new `agent/tests/delete/{mod.rs,deleter.rs}` (add `pub mod delete;` to `agent/tests/mod.rs`) covering the actor round-trip: spawn → enqueue → len → sweep → shutdown, and enqueue-after-shutdown returning `SendActorMessageErr`.
7. Create `agent/src/delete/.covgate`. After the milestone's covgate run, set it to the achieved percentage rounded DOWN to two decimals (target ≥ 95; the module is new, so aim high like `upload`'s 96.00).

### Milestone 3 — executor enqueues instead of deleting

1. `agent/src/upload/executor.rs`: add a fourth generic param `D: DeleterExt` and field `deleter: Option<Arc<D>>` to `LiveExecutor`; `new(http_client, token_mngr, transfer, deleter)`. Replace `delete_source_file` with:

       async fn enqueue_pending_delete(&self, job: &Job) {
           if job.delete_policy != DeletePolicy::AfterUpload {
               return;
           }
           let Some(deleter) = &self.deleter else {
               warn!("upload: no deleter available; skipping deletion for {}", job.file);
               return;
           };
           let record = PendingDelete {
               file: job.file.clone(),
               size: job.size,
               mtime: job.mtime,
               digest: job.digest.clone(),
               eligible_at: Utc::now(),
               delete_delay_secs: job.delete_delay_secs,
               upload_rule_id: job.upload_rule_id.clone(),
               deployment_id: job.deployment_id.clone(),
           };
           // best-effort: the upload is already confirmed durable; a failed
           // enqueue must never fail the job (that would re-drive it).
           if let Err(e) = deleter.enqueue(record).await {
               warn!("upload for {} confirmed but enqueueing its deletion failed: {e:?}", job.file);
           }
       }

   Call it where `delete_source_file(job)` was (executor.rs:125). Imports: `crate::delete::{DeleterExt, PendingDelete}` (internal group), `chrono::Utc` (external). Cancel safety is preserved: if the future is dropped before the enqueue, the file stays; re-observation + backend digest dedup re-drive it exactly as today.
2. Fix construction sites: `agent/src/app/state.rs::init_uploader` temporarily passes `None::<Arc<crate::delete::Deleter>>` (properly wired in Milestone 5); `agent/tests/upload/executor.rs` constructions gain a deleter argument.
3. Tests in `agent/tests/upload/executor.rs`: add a `MockDeleter` in `agent/tests/mocks/deleter.rs` (register in `mocks/mod.rs`) implementing `DeleterExt` with a recorded-calls `Vec<PendingDelete>` behind a mutex plus a scriptable enqueue result (mirror `MockUploadExecutor`'s shape). Cases: confirmed `after_upload` upload records exactly one enqueue whose `file`/`delete_delay_secs` match the job and the source file STILL EXISTS on disk (no inline delete); `never` records no enqueue; enqueue failure still returns `Ok` with `ConfirmUpload` called once; `deleter: None` with `after_upload` still returns `Ok`. Update the existing delete-policy executor tests (which asserted inline deletion) to the new semantics.

### Milestone 4 — the interval driver worker

1. Create `agent/src/workers/delete.rs` as a line-for-line analog of `agent/src/workers/scan.rs`: `Options { pub sweep_interval_secs: i64 }` with `Default` 60; `pub async fn run<F, Fut, DeleterT: DeleterExt>(options, deleter, sleep_fn, shutdown_signal)` — `tokio::select!` shutdown arm vs `run_impl` which performs an immediate initial `deleter.sweep()` (restart idempotency: entries that came due while the agent was down are processed promptly), then `loop { sleep_fn(interval); sweep }` with errors `error!`-logged and swallowed. Register `pub mod delete;` in `agent/src/workers/mod.rs`.
2. Tests in new `agent/tests/workers/delete.rs` (add `pub mod delete;` to `agent/tests/workers/mod.rs`), mirroring `agent/tests/workers/scan.rs` with `MockDeleter` (add `num_sweep_calls` + `set_sweep` to it, mirroring `MockScanner`) and `SleepController`: immediate initial sweep; sweeps on each released tick; sweep error does not stop the loop; shutdown wins the select.

### Milestone 5 — app wiring

1. `agent/src/app/options.rs`: add `pub enable_delete_worker: bool` and `pub delete_worker: crate::workers::delete::Options` to `AppOptions` (default `true` / `Default::default()`), following the existing pairs.
2. `agent/src/app/state.rs`: add `pub deleter: Option<Arc<crate::delete::Deleter>>` to `AppState`. Add `init_deleter(layout, enable) -> (Option<Arc<Deleter>>, Option<JoinHandle<()>>)` mirroring `init_uploader`'s fail-open shape (snapshot error → run without persistence; spawn error → `None` + `error!`), using `DeleteQueueSnapshotFile::new_with_default(layout.delete_queue(), Default::default())`. In `AppState::init`, spawn the deleter BEFORE the uploader (gated on the same `enable_uploader` flag) and pass `deleter.clone()` into `LiveExecutor::new` (replacing Milestone 3's `None`); push `deleter_handle` into the `shutdown_handle` join list. In `AppState::shutdown`, shut the deleter down immediately AFTER the uploader (the uploader's executor is its producer; the delete driver has already been joined by the `ShutdownManager` at this point) and before the scanner.
3. `agent/src/app/run.rs`: add `init_delete_worker(options.delete_worker, deleter, shutdown_manager, shutdown_tx.subscribe())` mirroring `init_scan_worker` (line 310), registered into a new `delete_worker_handle: Option<JoinHandle<()>>` slot with name `"delete_worker_handle"`. In `init`, inside the existing `if let Some(scanner)` block after the scan-upload bridge: `if options.enable_delete_worker { if let Some(deleter) = &app_state.deleter { init_delete_worker(...) } }`. In `shutdown_impl`, insert step 8 "delete driver worker" (join the handle, same error mapping) after the scan-upload bridge (step 7) and renumber app state to step 9 — the driver must join before `AppState::shutdown` runs so no sweeps race actor shutdown.
4. Tests in `run.rs`'s `#[cfg(test)] mod tests`: `register_handle_rejects_delete_worker_duplicates` and `shutdown_impl_maps_delete_worker_join_error`, both mirroring the scan-worker versions; extend `shutdown_impl_ok_when_all_steps_succeed` with the new slot (keeps `app` covgate 90.38 satisfied).

### Milestone 6 — preflight to CI-green

Local preflight, push, drive CI to green. See Concrete Steps and Validation and Acceptance.

## Concrete Steps

All commands run from the repo root on branch `feat/delete-worker`. After EVERY milestone: build, test, covgate, then commit exactly that milestone's files.

### Milestone 1

    cargo build -p miru-agent --features test

Expected before fixture fixes: `error[E0063]: missing field 'delete_delay_secs'` at each flagged literal (see Plan of Work M1.5). Fix, rebuild clean, add/extend the tests, then:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: suite passes; `models` (100), `scan` (98.83), `upload` (96.00), `workers` (84.67 — CI-authoritative if locally short) gates met.

    git add agent/src/models/upload_rule.rs agent/src/scan/state.rs agent/src/scan/collection.rs agent/src/upload/job.rs agent/src/workers/scan_upload_bridge.rs agent/tests/models/upload_rule.rs agent/tests/upload agent/tests/workers/scan_upload_bridge.rs
    git commit -m "feat(models): thread internal-only delete_delay_secs from rule to Job"

### Milestone 2

Create the module + layout method + tests per Plan of Work M2. Then:

    cargo build -p miru-agent --features test
    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: new in-src and `agent/tests/delete/` tests pass; covgate reports a coverage figure for `agent/src/delete/` — write that figure (rounded down) into `agent/src/delete/.covgate` and re-run `./scripts/covgate.sh` to confirm it gates green. `disk` gate (96.79) still met (the one-line `delete_queue()` is exercised by the actor tests via a snapshot file; if covgate flags it, add a trivial layout assertion to the existing disk tests).

    git add agent/src/delete agent/src/lib.rs agent/src/disk/layout.rs agent/tests/delete agent/tests/mod.rs
    git commit -m "feat(delete): pending-deletion queue and Deleter actor with metadata-checked sweep"

### Milestone 3

Edits per Plan of Work M3, then build/test/covgate as above. Expected: executor tests prove enqueue-not-delete; `upload` gate 96.00 met.

    git add agent/src/upload/executor.rs agent/src/app/state.rs agent/tests/mocks agent/tests/upload/executor.rs
    git commit -m "feat(upload): enqueue pending deletion after confirmed upload instead of deleting inline"

### Milestone 4

Edits per Plan of Work M4, then build/test/covgate. Expected: driver tests pass; `workers` gate met on CI (local ~83 shortfall is the known gap — do not lower the gate).

    git add agent/src/workers/delete.rs agent/src/workers/mod.rs agent/tests/workers/delete.rs agent/tests/workers/mod.rs agent/tests/mocks
    git commit -m "feat(workers): interval driver for the delete worker"

### Milestone 5

Edits per Plan of Work M5, then build/test/covgate. Expected: `app` gate 90.38 met; full suite green.

    git add agent/src/app/options.rs agent/src/app/state.rs agent/src/app/run.rs
    git commit -m "feat(app): wire the delete worker into app options, state, and shutdown"

### Milestone 6

    ./scripts/preflight.sh

Expected: "Preflight clean", modulo the known local `agent/src/workers/.covgate` gap. Then push and watch CI:

    git push -u origin feat/delete-worker
    gh pr checks --watch   # or: gh run watch

If any job fails: `gh run view --log-failed`, fix, commit (`fix(...)` scope), push, re-watch. Do not take the PR out of draft or report the task complete until all three jobs are green on the pushed head.

## Validation and Acceptance

Behavioral acceptance (all via `./scripts/test.sh`; every listed test is new or extended by this plan and fails before its milestone's edit):

1. Plumbing: an old cached rule / old `scanner.json` / backend payload without `delete_delay_secs` deserializes to 0; a rule destination with `delete_delay_secs: 300` stamps 300 onto the `StableFile` and the bridge copies it onto the `Job`.
2. Sweep safety (in-src `delete` tests, injected clock): due + unchanged → file deleted, entry dropped; not due → untouched; size or mtime changed → entry dropped, FILE STILL EXISTS; missing → dropped as success; delete I/O error → entry retained and retried, loop never panics; queue survives a restart via `delete_queue.json`.
3. Executor: confirmed `after_upload` upload leaves the file on disk and enqueues exactly one `PendingDelete` matching the job; `never` enqueues nothing; enqueue failure or absent deleter still returns `Ok` with confirm called once.
4. Driver: immediate initial sweep, cadenced sweeps, error-swallowing, clean shutdown; `register_handle_rejects_delete_worker_duplicates` and the join-error mapping pass in `run.rs`.

Run `./scripts/test.sh` and expect all tests to pass, then `./scripts/covgate.sh` and expect the `models`, `scan`, `upload`, `delete`, `workers`, `app`, and `disk` gates met (workers: CI-authoritative).

CI acceptance (authoritative): **preflight must report CLEAN — i.e. CI green (all three jobs: `lint`, `test`, `tools`) on the pushed head of `feat/delete-worker` — before the PR leaves draft or the task is reported complete.** Local validation is `./scripts/preflight.sh` (bundling `scripts/lint.sh` and `scripts/covgate.sh` plus the tools equivalents), but GitHub Actions (`.github/workflows/ci.yml`) is the source of truth. The known local `agent/src/workers/.covgate` shortfall (~83 vs 84.67) does not count as a failure if CI's workers coverage passes; never lower that gate.

## Idempotence and Recovery

Every edit is additive (fields with serde defaults, a new module, a new worker, new option/slot/step) and safe to re-apply; re-running any step once its artifact exists is a no-op. Build/test/covgate/preflight are read-only.

Runtime idempotence is the design's core: `files::delete` treats `NotFound` as success; a `PendingDelete` re-enqueued for the same path replaces the older record; a sweep interrupted by shutdown re-runs its due entries from `delete_queue.json` after restart (the immediate initial sweep). A crash between upload confirmation and enqueue loses only the deletion, never data: the file stays on disk, the scanner's ledger already holds it, and the worst case is a retained file — the failure mode is always "kept too long", never "deleted wrongly". Rules with delay 0 (all rules until #212 lands) behave as today within one sweep interval.

Recovery during implementation: before a commit, `git checkout -- <files>` restores a clean tree; `git reset --hard main` is a safe full rollback pre-push (the branch exists only for this work). After a commit, `git revert <sha>` or `git reset --hard <prev-sha>` (pre-push) backs out a milestone — each milestone is its own commit specifically so it can be reverted or bisected independently. On-device rollback is equally safe: an older agent ignores `delete_queue.json` and unknown serde fields default away.
