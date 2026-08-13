# Unify the upload and retention job queues behind one generic queue

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `mirurobotics/agent`) | read-write | All changes: the new shared queue module, the two worker migrations, and test consolidation. |

This plan lives in `plans/backlog/` in this repo because all code written by it is in this repo's Rust crate `miru-agent` (source rooted at `agent/src/`). Work happens on the existing branch `refactor/shared-worker-queue`, branched from `main` at `c1ebf64`.

Note: the repo root contains both `AGENTS.md` and `CLAUDE.md`. `CLAUDE.md` is a symlink, but its target is `AGENTS.MD` (uppercase extension), which does not exist on this case-sensitive filesystem — so the symlink is currently broken and `cat CLAUDE.md` fails. Read `AGENTS.md` directly; it is the single conventions file.

## Purpose / Big Picture

The agent has two background workers that each own a durable job queue: the **uploader** (uploads data files to the backend) and the **retention deleter** (deletes local files once their retention TTL expires). Their queues — `agent/src/data_uploads/upload/queue.rs` (193 lines) and `agent/src/data_uploads/retention/queue.rs` (180 lines) — are near-duplicates: same struct, same at-least-once durability contract, same snapshot-rewrite persistence, differing only in the job type, two log-prefix strings, one readiness clause, and a handful of methods that exist on one but not the other.

After this change there is exactly **one** queue implementation, generic over the job type, used by both workers. A reader gains: one place to read the durability invariant, one place to fix a bug in it, and both workers automatically getting the methods that previously only one had.

This is a **pure refactor**. Nothing a user can observe changes: no wire format change, no config change, no generated-code change, no behavior change. The observable acceptance is therefore "the same tests pass, plus new tests that pin the on-disk format byte-for-byte" — see Validation and Acceptance.

## Progress

- [ ] M1 — Pin the upload queue's on-disk JSON shape with a raw-JSON round-trip test (regression guard for everything after).
- [ ] M2 — Add the generic queue module `agent/src/data_uploads/queue/`, the two `QueueJob` impls, its test file, and a measured `.covgate`.
- [ ] M3 — Migrate the uploader's queue onto the generic (delete the duplicated implementation, add the aliases); measure `upload/` covgate.
- [ ] M4 — Migrate the retention deleter's queue onto the generic (delete the duplicated implementation, add the aliases); measure `retention/` covgate.
- [ ] M5 — Consolidate the duplicated test cases and re-measure all gates.
- [ ] M6 — Preflight CLEAN, push, draft PR, CI green on the pushed branch head.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

(Add entries as you go. The authoring-time decisions are stated inline in Plan of Work; record any departure from them here with a rationale.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### The two queues today

Both live under `agent/src/data_uploads/`. `upload/queue.rs` defines:

- `QueueEntry { id: Uuid (serde default = Uuid::new_v4), job: Job, attempts: u32, next_attempt_at: Option<DateTime<Utc>> (serde default) }`
- `QueueSnapshot { entries: Vec<QueueEntry> }`, with `impl Patch<QueueSnapshot> for QueueSnapshot` (whole-value replace)
- `pub type QueueSnapshotFile = SingleThreadStateFile<QueueSnapshot, QueueSnapshot>`
- `pub struct Queue { jobs: VecDeque<QueueEntry>, capacity: usize, snapshot_file: Option<QueueSnapshotFile> }`

with methods `new(capacity)`, `from_snapshot(capacity, file)`, `len`, `is_empty`, `async enqueue(job) -> Result<(), UploadErr>`, `async remove(id) -> Option<QueueEntry>`, `async requeue(entry)`, `next_ready(now) -> Option<QueueEntry>`, `reset_invalid_deadlines(horizon)`, `earliest_next_attempt() -> Option<DateTime<Utc>>`, plus private `remove_impl`, `verify_capacity`, `persist`.

`retention/queue.rs` is the same shape with these differences:

- Snapshot types are named `DeleteQueueSnapshot` / `DeleteQueueSnapshotFile`; the deque field is `entries`, not `jobs`.
- `attempts` also carries `#[serde(default)]` (upload's does not).
- Capacity rejection is inlined in `enqueue` instead of a `verify_capacity` helper, and returns `DeleteErr::QueueFullErr`.
- Readiness goes through `fn is_ready(entry, now) -> bool` = `entry.job.due_at() <= now && entry.next_attempt_at.is_none_or(|at| at <= now)` — **two** clauses. Upload has only the `next_attempt_at` clause, because an upload job is due the moment it is enqueued; a retention job has a TTL that must elapse first (`retention::Job::due_at()` in `agent/src/data_uploads/retention/job.rs:24`).
- Adds `count_ready(now) -> usize` (the sweep's per-tick loop budget). Lacks `reset_invalid_deadlines` and `earliest_next_attempt`.
- Adds two `#[cfg(test)] pub(crate)` accessors, `entries() -> Vec<Job>` and `queue_entries() -> Vec<QueueEntry>`, consumed by roughly 15 call sites in the inline `mod tests` of `agent/src/data_uploads/retention/deleter.rs` (about lines 524–1183).
- Log prefix is `delete:` where upload uses `upload:`; upload additionally emits an `info!` inside `next_ready` that retention does not.

### Semantics that must be preserved exactly

- **Per-entry identity**: a fresh `Uuid` is minted at `enqueue`. No field of `Job` is a key. Duplicate jobs are legal and independently removable by id.
- **At-least-once durability**: `next_ready` returns a *clone* and deliberately **leaves the entry in the deque and in the snapshot**. An entry only leaves disk via `remove(id)`. A crash mid-work therefore replays the entry at next boot. This is the single most important invariant in these files.
- **Strict FIFO with skip**: `next_ready` scans from the head and returns the first *eligible* entry, skipping ineligible ones in place. `requeue` rotates an entry to the tail.
- **Full-snapshot rewrite** on every mutation (`enqueue`, `remove`, `requeue`); selection never persists.
- **Capacity gates `enqueue` only.** `requeue` and `from_snapshot` bypass it by design, so an over-capacity backlog loaded from disk drains before it accepts more.
- **The queue stores but never computes `attempts` / `next_attempt_at`.** Both workers own their own backoff policy via `cooldown::calc(&backoff, entry.attempts - 1)`.
- **Persist failures are logged with `warn!` and swallowed**, never returned.

### The on-disk format (do not break this)

`agent/src/disk/layout.rs:49` and `:53` place the files at `<root>/var/lib/miru/upload_queue.json` and `<root>/var/lib/miru/delete_queue.json`. Both are managed by `SingleThreadStateFile<C, P>` (`agent/src/filesys/state_file.rs:29`), whose constructor `new_with_default` (`:54`) **falls back to writing the default value on any read or parse error**. There is no version field. Consequently a change to the serialized shape does not produce an error — it **silently wipes a live user's queue**. `upload_queue.json` is released user data.

The serialized envelope is byte-identical between the two queues; only the nested `"job"` object differs:

    {"entries":[{"id":"<uuid>","job":{ ... nested, not flattened ... },"attempts":0,"next_attempt_at":null}]}

A generic `QueueEntry<J>` / `QueueSnapshot<J>` serializes **identically** to today's concrete types — serde ignores type parameters. The refactor is therefore wire-safe provided that: no enum wrapper is introduced, no `#[serde(tag = ...)]`, no `version` field, no `#[serde(flatten)]`, and no rename or reorder of `entries` / `id` / `job` / `attempts` / `next_attempt_at`. Adding `#[serde(default)]` to `attempts` (matching retention) is a safe loosening and is intended.

### Callers

- `agent/src/data_uploads/upload/uploader.rs` is the only production consumer of the upload queue: constructs it at `:467-470`, then uses `next_ready` `:162`, `is_empty` `:169`, `reset_invalid_deadlines` `:184`, `earliest_next_attempt` `:187`, `remove` `:210/:252/:278/:283`, `requeue` `:347` (via `requeue_after` `:294`), `enqueue` `:425`, `len` `:433`; imports `QueueSnapshotFile` at `:14` and `:455`.
- `agent/src/data_uploads/retention/deleter.rs` is the only production consumer of the delete queue: constructs it at `:80-83`, then `len` `:93`, `enqueue` `:97`, `count_ready` `:106`, `next_ready` `:107`, `remove` `:113/:126`, `requeue` `:136`.
- `agent/src/app/state.rs` only constructs the two snapshot files (`:182` delete, `:217` upload), both failing open to `None`.
- Re-exports: `agent/src/data_uploads/upload/mod.rs:12` and `agent/src/data_uploads/retention/mod.rs:10`.

`DeleterExt` (`deleter.rs:232-249`) returns `-> impl Future + Send` rather than using `async fn`, so generic callers can prove `Send`. Any generic queue used inside those futures must stay `Send`; the job type parameter therefore needs a `Send` bound.

### Coverage gates ("covgate")

`scripts/covgate.sh` (wrapping `scripts/lib/covgate.sh`) runs one `cargo llvm-cov --json` pass over `--package miru-agent --features test`, then for every directory containing a `.covgate` file computes **LLVM region coverage** — `sum(summary.regions.covered) / sum(summary.regions.count)` — over every coverage file whose absolute `filename` **starts with that directory's path**. Matching is prefix-based, so a gate is recursive over subdirectories. A `.covgate` containing `0` skips the check. A gate with no matching files warns and passes.

Existing thresholds relevant here: `agent/src/data_uploads/upload/.covgate` = `96.00`, `agent/src/data_uploads/retention/.covgate` = `98.39`, `agent/src/data_uploads/scan/.covgate` = `98.83`. There is **no** `.covgate` at `agent/src/` and **none** at `agent/src/data_uploads/` — so anything placed directly in `agent/src/data_uploads/` would be ungated. `agent/src/services/` and `agent/src/services/deployment/` show that nesting a gate inside a gated directory is an established pattern.

`AGENTS.md:81` says: "Each module has a `.covgate` file with a minimum coverage percentage. Run `scripts/covgate.sh` to enforce. When adding or modifying code, verify coverage still passes." **This plan adds its own hard constraint on top of that: never lower an existing `.covgate` value — a failing gate here is a missing test, not a threshold to lower.** `scripts/update-covgates.sh` only ratchets values **up** and will not create a new file. Creating a `.covgate` for a genuinely new gated directory is allowed and is required by AGENTS.md's "Adding a new module" checklist (`AGENTS.md:105`).

**The risk this plan must actively manage:** `upload/queue.rs` is 193 of 1314 non-`mod.rs` lines in `upload/` (~14.7%) and is exhaustively tested. Removing it leaves the remaining `upload/` files — dominated by `uploader.rs` at 542 lines — to clear 96.00 on their own. The same applies to `retention/` at 98.39, though `deleter.rs`'s large inline test suite makes it likelier to hold. This is measured, not assumed, at the end of M3 and M4.

### Repo conventions that will bite a newcomer

- **Import ordering**: three groups, each preceded by a literal comment line `// standard crates`, `// internal crates`, `// external crates`, blank line between groups. Enforced by `tools/lint` via `.lint-imports.toml`.
- **Errors**: each module has an `errors.rs`; leaf errors derive `thiserror::Error` and `impl crate::errors::Error`; aggregating enums use `crate::impl_error!`; every error carries `trace: Box<Trace>` built with `crate::trace!()`. The error crate is `agent/src/errors/` — there is no external `errs` crate.
- **Feature flag**: `#[cfg(feature = "test")]` gates mocks and setters; never in a production path.
- **Tests**: `agent/tests/` is a single integration-test crate rooted at `agent/tests/mod.rs`. The tree mirrors `agent/src/`. Run with `./scripts/test.sh` (which is `RUST_LOG=off cargo test --features test`).
- **Lint**: `./scripts/lint.sh` runs the import linter, a field-by-field-assert linter over `agent/tests` (4+ `assert_eq!` on fields of the *same* variable inside one test function is a failure; suppress with `// lint:allow(field-by-field-assert)` inside the test body), `cargo fmt`, machete/diet, audit, and clippy with `-D warnings --all-features`.
- **Preflight**: `./scripts/preflight.sh` runs lint + covgate + tools lint + tools tests in parallel and must print `Preflight clean`.
- **CI** (`.github/workflows/ci.yml`): the `lint` job runs `LINT_FIX=0 ./scripts/lint.sh` (`:35`); the test job runs `./scripts/covgate.sh` (`:56`); the tools job runs `./tools/lint/scripts/lint.sh` (`:84`) and `./tools/lint/scripts/covgate.sh` (`:87`). A covgate regression is a red build.
- **Generics prior art to mirror**: `SingleThreadStateFile<ContentT, PatchT>` (`agent/src/filesys/state_file.rs:29`) with a `where` clause and `PhantomData`; the `Patch<PatchT>` trait (`agent/src/models/mod.rs:33`); `Worker<ExecutorT, D, F, Fut, N>` (`uploader.rs:134`); `HttpBackend<'a, C: ClientI, T: TokenManagerExt>` (`agent/src/services/backend.rs:25`).

### Prior decision this plan supersedes

`plans/active/20260812-retention-queue-structural-durability.md` — the change that made the two queues near-identical — was deliberately a *mirror* exercise, not a unify one (`:29` "The reference implementation is the upload queue after 5a6cae2 … Mirror its structure and naming."; `:183` "Do not rename `DeleteQueueSnapshot*` to the uploader's `QueueSnapshot*`; … the rename is churn with no reader benefit."). It also deferred a `Queue::earliest_due_at()` analogue. There is no recorded decision *against* unifying, and `TECH_DEBT.md` has no entry on this duplication. Mirroring was the right call then because the two queues were not yet known to be identical; now that they demonstrably are, unification is the follow-through. Note that this plan honours the `:183` constraint: the public alias names `DeleteQueueSnapshot` / `DeleteQueueSnapshotFile` and `QueueSnapshot` / `QueueSnapshotFile` are all **kept**, as aliases over the generic.

## Plan of Work

### The design

Create `agent/src/data_uploads/queue/mod.rs` — a **directory**, so it can carry its own `.covgate` (a bare `agent/src/data_uploads/queue.rs` would be ungated, since there is no `.covgate` at `agent/src/data_uploads/`). Register it with `pub mod queue;` in `agent/src/data_uploads/mod.rs`. No `errors.rs` is needed: the queue does not own an error type (see the error hook below).

It contains:

**1. The job trait.** One trait carries every per-worker difference:

    pub trait QueueJob: Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static {
        /// Error returned by `Queue::enqueue` when the queue is at capacity.
        type QueueFullErr;
        /// Log prefix, e.g. "upload" or "delete".
        const LABEL: &'static str;
        /// Earliest instant this job may be actioned. Jobs actionable on
        /// arrival return `DateTime::<Utc>::MIN_UTC`.
        fn due_at(&self) -> DateTime<Utc>;
        /// The file this job concerns, for logs and the capacity error.
        fn file(&self) -> String;
        fn queue_full_err(capacity: usize, file: String) -> Self::QueueFullErr;
    }

`upload::Job::due_at()` returns `DateTime::<Utc>::MIN_UTC` ("always due"). That collapses `is_ready` to one implementation — `entry.job.due_at() <= now && entry.next_attempt_at.is_none_or(|at| at <= now)` — which is exactly upload's behavior when the first clause is vacuously true, and makes `count_ready` correct for both workers for free.

The error hook is an associated type with a factory, rather than a shared `QueueFullErr` leaf with `From` impls, because it lets `enqueue` keep its exact current signatures — `Result<(), UploadErr>` and `Result<(), DeleteErr>` — so `uploader.rs`, `deleter.rs`, and both test suites compile and assert unchanged. `upload::Job` sets `type QueueFullErr = UploadErr` and returns `UploadErr::QueueFullErr(upload::errors::QueueFullErr { .. })`; `retention::Job` sets `type QueueFullErr = DeleteErr` similarly. Both `Display` strings ("upload queue is full …" / "delete queue is full …") are untouched, so upload's `err.to_string().contains("queue is full")` assertion and retention's `DeleteErr::QueueFullErr(full)` destructuring both keep passing.

Note that `queue_full_err` deliberately takes **no** `trace` argument. Each worker's impl — in `agent/src/data_uploads/upload/queue.rs` and `agent/src/data_uploads/retention/queue.rs` — calls `crate::trace!()` inside its own method body when building the error. That way `trace!()` is invoked at the trait-impl site (inside each worker's module), not inside the generic, so the recorded trace location stays inside the worker rather than pointing at shared code. Threading a `Box<Trace>` in as a parameter would defeat this, since the queue's only call site is `verify_capacity` inside `agent/src/data_uploads/queue/mod.rs`.

**2. The data types.**

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct QueueEntry<J> {
        #[serde(default = "Uuid::new_v4")]
        pub id: Uuid,
        pub job: J,
        #[serde(default)]
        pub attempts: u32,
        #[serde(default)]
        pub next_attempt_at: Option<DateTime<Utc>>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct QueueSnapshot<J> { pub entries: Vec<QueueEntry<J>> }

    impl<J> Patch<QueueSnapshot<J>> for QueueSnapshot<J> { fn patch(&mut self, patch: Self) { *self = patch; } }

    pub type QueueSnapshotFile<J> = SingleThreadStateFile<QueueSnapshot<J>, QueueSnapshot<J>>;

Two derive pitfalls to expect, both of which produce confusing compiler errors:

- **Do not put `J: QueueJob` bounds on `QueueEntry<J>` / `QueueSnapshot<J>`.** `#[derive(Deserialize)]` generates a `J: Deserialize<'de>` bound, which does not unify with a `DeserializeOwned` bound written on the struct. Leave *these two* structs unbounded (serde's derive emits its own per-field bounds) and put `J: QueueJob` on the `impl` blocks. If serde's inferred bounds still fight you, add `#[serde(bound(serialize = "J: Serialize", deserialize = "J: DeserializeOwned"))]` — that is a bound annotation only and does not alter the emitted JSON. This guidance is specific to these two serde-derived types; it does **not** extend to `Queue<J>`, which must carry `J: QueueJob` on its own definition (see below).
- **Do not `#[derive(Default)]` on `QueueSnapshot<J>`.** The derive adds a spurious `J: Default` bound that neither `Job` type satisfies, and `agent/src/app/state.rs:182` and `:217` call `Default::default()` on the snapshot. Hand-write it:

      impl<J> Default for QueueSnapshot<J> { fn default() -> Self { Self { entries: Vec::new() } } }

**3. The queue.**

    pub struct Queue<J: QueueJob> { entries: VecDeque<QueueEntry<J>>, capacity: usize, snapshot_file: Option<QueueSnapshotFile<J>> }

The bound on the **definition** (equivalently a `where J: QueueJob` clause) is required, not optional: the `snapshot_file` field is `Option<QueueSnapshotFile<J>>` = `SingleThreadStateFile<QueueSnapshot<J>, QueueSnapshot<J>>`, and `SingleThreadStateFile` carries its bounds on its own struct definition (`agent/src/filesys/state_file.rs:29-32`: `where ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq`). An unbounded `Queue<J>` therefore fails well-formedness with `E0277`.

`impl<J: QueueJob> Queue<J>` provides the **superset** of both current method sets: `new`, `from_snapshot`, `len`, `is_empty`, `enqueue`, `remove`, `requeue`, `next_ready`, `count_ready`, `reset_invalid_deadlines`, `earliest_next_attempt`, plus private `remove_impl`, `is_ready`, `persist`. Bodies are copied verbatim from the existing implementations, with the `upload:` / `delete:` literals replaced by `{}` formatted with `J::LABEL` and the capacity check factored into a private `verify_capacity` (upload's shape). `#[allow(dead_code)]` is **not** used; unused methods on a shared generic are acceptable, and are covered by the shared test suite (see below).

**Watch the persist warning specifically.** The worker name appears **twice** in it, and only the first occurrence is the log prefix: `agent/src/data_uploads/upload/queue.rs:190` is `warn!("upload: failed to persist upload queue: {err}")`, and `agent/src/data_uploads/retention/queue.rs:165` has the same shape. Substitute **both** occurrences, so the generic reads `warn!("{label}: failed to persist {label} queue: {err}")`. Mechanically replacing only the prefix would make the retention worker log "failed to persist **upload** queue".

Retention's two `#[cfg(test)] pub(crate)` accessors (`entries()`, `queue_entries()`) move onto the generic `impl` unchanged. `#[cfg(test)]` applies to the same unit-test build as `deleter.rs`'s inline `mod tests`, and `pub(crate)` reaches crate-wide, so `deleter.rs`'s ~15 call sites need no edit.

**4. The worker-side shims.** `upload/queue.rs` and `retention/queue.rs` shrink to a trait impl plus type aliases:

    // upload/queue.rs
    impl QueueJob for Job { type QueueFullErr = UploadErr; const LABEL: &'static str = "upload"; ... }
    pub type Queue = crate::data_uploads::queue::Queue<Job>;
    pub type QueueEntry = crate::data_uploads::queue::QueueEntry<Job>;
    pub type QueueSnapshot = crate::data_uploads::queue::QueueSnapshot<Job>;
    pub type QueueSnapshotFile = crate::data_uploads::queue::QueueSnapshotFile<Job>;

    // retention/queue.rs — same, with DeleteQueueSnapshot / DeleteQueueSnapshotFile as the two snapshot alias names.

Keeping all four snapshot alias names means `agent/src/app/state.rs`, both `mod.rs` re-export lines, `uploader.rs`, `deleter.rs`, and both integration-test files compile **unchanged**. Keeping the impls in each worker's `queue.rs` (rather than in `job.rs` or `mod.rs`) also keeps the file layout and the `pub use self::queue::{...}` lines stable.

**These two files are edited in two passes, and the split matters.** The `QueueJob` impls land in **M2**, added to `upload/queue.rs` and `retention/queue.rs` *alongside* each file's existing concrete `Queue` implementation, which stays fully in place and still in use by its worker. A bare trait impl for `Job` coexists with a same-named concrete `Queue` struct in the same file without conflict — the impl is on `Job`, not on `Queue` — so nothing else changes at M2. This is required, not cosmetic: M2's test file exercises `Queue<upload::Job>` and `Queue<retention::Job>` against the production job types, which does not compile until both impls exist. The **aliases** land later, in M3 and M4, at the moment each duplicated concrete implementation is deleted.

### Behavior deltas (the complete list)

Only two, both log-only and neither asserted by any test:

1. `retention::Queue::next_ready` starts emitting the `info!("{label}: job dequeued; queue length {n}")` line that upload already emits.
2. Retention's capacity `warn!` text normalizes from `"delete: queue is full (capacity …)"` to upload's `"{label} queue is full (capacity …)"` shape — i.e. `"delete queue is full (capacity …)"`. The `DeleteErr` **`Display` string is unchanged**; only the log line moves.

The persist `warn!` is deliberately **not** on this list — but it is only delta-free if both occurrences of the worker name are substituted as described above. `"{label}: failed to persist {label} queue: {err}"` renders byte-identically to today's text for both workers; substituting only the prefix would make it a third (and wrong) delta.

Everything else — public API, method semantics, error types and messages, JSON — is identical.

### Test strategy

- New `agent/tests/data_uploads/queue.rs`, registered with `pub mod queue;` in `agent/tests/data_uploads/mod.rs`. It exercises the generic against **both production job types** (not a synthetic test job type — the new module's covgate is computed from the production generic, and using both job types also proves the `due_at` hook works in both its states).
- Every generic method must be tested, including `count_ready`, `reset_invalid_deadlines`, and `earliest_next_attempt` for **both** job types, even though each is used by only one worker today. Uncovered methods are uncovered LLVM regions and will drag the new `.covgate` down.
- Cases to move here (they exist today in both suites, ~70% overlapping intent): `empty_snapshot_loads_empty_queue`, `full_queue_returns_queue_full_err`, `rejection_does_not_persist`, `persist_failure_is_swallowed`, `unknown_id_is_ignored`, `removing_one_duplicate_leaves_the_other`, `leaves_the_entry_on_disk_until_removed`, `removes_the_entry_from_disk`, the duplicate-jobs cases, attempts-survive-reload, and next_attempt_at-survives-reload.
- Cases that stay put: upload-specific `earliest_next_attempt` / `reset_invalid_deadlines` scenarios in `agent/tests/data_uploads/upload/queue.rs`; retention's TTL / `due_at` readiness cases, `count_ready` budget cases, and the `durability` mid-sweep-persist test in `agent/tests/data_uploads/retention/queue.rs`.
- The two existing suites use different idioms; pick the tighter one for the shared file. Prefer retention's deterministic `now()` fixture at t=2000 over upload's `Utc::now()`-based `make_job` (upload's helper is non-deterministic, which is why its assertions go on `digest` rather than whole `Job` values). Prefer `dir.file(..)` over `dir.to_dir().file(..)` — `TempDir` derefs to `Dir`, so `to_dir()` is redundant. For the persist-failure case, the directory-at-path trick (retention) is more portable than `chmod 0555` (upload). The chmod variant (`persist_failure_is_swallowed`, `agent/tests/data_uploads/upload/queue.rs:167`) carries no `#[serial]` annotation and needs none: it chmods a per-test subdirectory under a fresh `dirs::temp(...)`, not a fixed global path, and `AGENTS.md:71-75` reserves `#[serial]` for shared OS resources.

## Concrete Steps

All commands run from the repo root `/home/ben/miru/workbench1/repos/agent` unless stated otherwise. Confirm you are on the working branch first:

    git branch --show-current
    # expect: refactor/shared-worker-queue

### M1 — Pin the upload queue's on-disk JSON shape

Retention already has a raw-JSON wire pin (`agent/tests/data_uploads/retention/queue.rs:86-110`); upload does not. Add one **before** touching any code, so it guards every later milestone.

In `agent/tests/data_uploads/upload/queue.rs`, inside the `from_snapshot` module, add a test that writes a literal JSON string to a temp path, opens it as a `QueueSnapshotFile`, and asserts the loaded queue's contents; then add a second test that enqueues one job, reads the file back as `serde_json::Value`, and asserts the exact key set and value shapes:

    {"entries":[{"id":"<uuid>","job":{...},"attempts":0,"next_attempt_at":null}]}

Assert on the top-level key `entries` and on the entry's four keys `id`, `job`, `attempts`, `next_attempt_at`; assert `job` is a JSON object (not flattened into the entry). Mirror the retention test's structure. If you write four or more `assert_eq!` on fields of the same variable in one test, either split the test or add `// lint:allow(field-by-field-assert)` inside the body.

Run:

    ./scripts/test.sh

Expect a clean `test result: ok.` line for the integration test binary. Then:

    git add -A && git commit -m "test(upload): pin upload_queue.json on-disk shape"

### M2 — Add the generic queue module and the two job impls

1. Create `agent/src/data_uploads/queue/mod.rs` implementing the design above.
2. Add `pub mod queue;` to `agent/src/data_uploads/mod.rs`. The current order is `retention`, `scan`, `upload`; insert `queue` as the first line to keep the list alphabetical.
3. Add `impl QueueJob for Job` to `agent/src/data_uploads/upload/queue.rs` and `impl QueueJob for Job` to `agent/src/data_uploads/retention/queue.rs` (the latter's `due_at` delegates to the existing `Job::due_at`; upload's returns `DateTime::<Utc>::MIN_UTC`). **Add only the impls — leave each file's existing concrete `Queue` implementation, its snapshot types, and any inline `mod tests` entirely in place, still compiled and still used by its worker.** No other file changes: `uploader.rs`, `deleter.rs`, `app/state.rs`, both `mod.rs` re-exports, and both per-worker test files are untouched at M2. Without these two impls, step 6 does not compile and step 7 cannot be measured, since the new test file drives the generic with the production job types.
4. Create `agent/tests/data_uploads/queue.rs` and add `pub mod queue;` to `agent/tests/data_uploads/mod.rs`.
5. Create `agent/src/data_uploads/queue/.covgate` containing `0` for now — this makes the gate present but skipped, so the module does not fail the build before its coverage is measured in step 7.

At this point the generic `Queue` is still *unused by production code*, which is fine: both workers keep running on their own concrete queues, and the only thing that lands in their `queue.rs` files is a `QueueJob` impl that no production path calls. The tests in `agent/tests/data_uploads/queue.rs` are the generic queue's only consumer. (Those impls do add regions to the `upload/` and `retention/` gates rather than to the new `queue/` gate; the new test file exercises them, so both should hold — confirm in step 7's `covgate.sh` run.)

6. Run tests and lint:

       ./scripts/test.sh
       ./scripts/lint.sh

   `lint.sh` auto-fixes some findings (notably `cargo fmt` and clippy `--fix`), so re-check `git status` after it runs and include any fixes in the commit.

7. Measure the new module's coverage:

       ./scripts/covgate.sh

   With the threshold at `0` you will see a line like:

       ⏭️  data_uploads/queue: skipped (threshold: 0)

   To read the actual number, temporarily set the file to `1` and re-run; the output line becomes:

       ✅ data_uploads/queue: 99.12% (requires 1%)

   Write the measured value, rounded **down** to two decimals, into `agent/src/data_uploads/queue/.covgate`. If the measured value is below ~95%, add tests for the uncovered methods rather than seeding a low gate — the whole point of the superset method set is that everything is exercised.

8. Commit (files touched: `agent/src/data_uploads/queue/mod.rs`, `agent/src/data_uploads/queue/.covgate`, `agent/src/data_uploads/mod.rs`, `agent/src/data_uploads/upload/queue.rs`, `agent/src/data_uploads/retention/queue.rs`, `agent/tests/data_uploads/queue.rs`, `agent/tests/data_uploads/mod.rs`):

       git add -A && git commit -m "feat(data_uploads): add generic job queue and its upload and retention job impls"

### M3 — Migrate the uploader

1. In `agent/src/data_uploads/upload/queue.rs`, delete the duplicated concrete implementation (the `Queue` struct and its `impl`, the `QueueEntry` / `QueueSnapshot` / `QueueSnapshotFile` definitions, and the `Patch` impl) and add the four type aliases in their place. The `QueueJob` impl for `upload::Job` is **already in this file from M2** — leave it exactly as it is; do not rewrite it. What remains is that impl plus the aliases.
2. Leave `agent/src/data_uploads/upload/mod.rs:12`, `agent/src/data_uploads/upload/uploader.rs`, `agent/src/app/state.rs`, and `agent/tests/data_uploads/upload/queue.rs` **untouched**. If any of them fails to compile, the aliases are wrong — fix the aliases, not the caller. The one exception: if `Queue::new` / `Queue::from_snapshot` need turbofish at a call site, that is a legitimate caller edit; note it in Surprises & Discoveries.
3. Run:

       ./scripts/test.sh
       ./scripts/lint.sh
       ./scripts/covgate.sh

**Decision point.** `./scripts/covgate.sh` must still report `✅` for `data_uploads/upload` at `96.00`. Removing a heavily-tested 193-line file from that directory can push the remaining files below the gate. If it fails:

- **Do**: add tests to the remaining `upload/` files (chiefly `agent/src/data_uploads/upload/uploader.rs`) until the gate passes, and/or reconsider placement of the generic (e.g. nesting it under one of the two worker directories so its coverage counts toward that gate).
- **Do not**: lower `agent/src/data_uploads/upload/.covgate`. This plan's hard constraint (see *Coverage gates* above) forbids it.

Record the outcome and the number in Surprises & Discoveries either way.

4. Commit (files touched: `agent/src/data_uploads/upload/queue.rs` only, unless the covgate decision point above forced test additions):

       git add -A && git commit -m "refactor(upload): back the upload queue with the shared generic queue"

### M4 — Migrate the retention deleter

1. In `agent/src/data_uploads/retention/queue.rs`, delete the duplicated concrete implementation (the `Queue` struct and its `impl`, including `is_ready` / `count_ready` and the two `#[cfg(test)]` accessors, the `QueueEntry` / `DeleteQueueSnapshot` / `DeleteQueueSnapshotFile` definitions, and the `Patch` impl) and add the aliases in their place, keeping the names `DeleteQueueSnapshot` and `DeleteQueueSnapshotFile`. The `QueueJob` impl for `retention::Job` is **already in this file from M2** — leave it exactly as it is; do not rewrite it. What remains is that impl plus the aliases.
2. Confirm the `#[cfg(test)] pub(crate)` accessors on the generic satisfy `deleter.rs`'s inline tests. If visibility does not resolve, the fallback is to rewrite the ~15 call sites in `agent/src/data_uploads/retention/deleter.rs` (lines ~524–1183) — but try the accessors-on-the-generic route first, and record which one worked.
3. Run:

       ./scripts/test.sh
       ./scripts/lint.sh
       ./scripts/covgate.sh

**Decision point.** `data_uploads/retention` must still report `✅` at `98.39`, under the same do / do-not rules as M3 (add tests or relocate the generic; never lower the gate, per this plan's own constraint).

4. Commit (files touched: `agent/src/data_uploads/retention/queue.rs`, plus `agent/src/data_uploads/retention/deleter.rs` only if step 2's fallback was needed):

       git add -A && git commit -m "refactor(retention): back the delete queue with the shared generic queue"

### M5 — Consolidate the tests and settle the gates

1. Move the overlapping cases listed in *Test strategy* out of `agent/tests/data_uploads/upload/queue.rs` and `agent/tests/data_uploads/retention/queue.rs` into `agent/tests/data_uploads/queue.rs`, parameterized over both job types. Delete the now-redundant originals. Keep every worker-specific case where it is.
2. Keep the M1 upload wire pin and the existing retention wire pin **in their per-worker files** — they pin the concrete on-disk artifacts, which is a per-worker property.
3. Re-run everything and re-measure:

       ./scripts/test.sh
       ./scripts/lint.sh
       ./scripts/covgate.sh

   All of `data_uploads/queue`, `data_uploads/upload`, `data_uploads/retention`, and `data_uploads/scan` must be `✅`. Moving tests out of the per-worker files does not change what those files cover (the shared file exercises the same production code via the aliases), but verify rather than assume.
4. Optionally ratchet gates up:

       ./scripts/update-covgates.sh

   This only raises values; review the diff and keep only raises you are confident CI will reproduce.
5. Commit:

       git add -A && git commit -m "test(data_uploads): consolidate shared queue tests"

### M6 — Preflight, push, PR

    ./scripts/preflight.sh

Expect the final line:

    Preflight clean

If it is not clean, fix and re-run; do not proceed. Then:

    git push -u origin refactor/shared-worker-queue
    gh pr create --draft --base main --title "refactor(data_uploads): share one job queue between the upload and retention workers" --body "..."

Note: `gh pr edit` is known to fail silently in this environment; use `gh api` REST if you need to modify the PR after creation.

Watch CI on the pushed branch head:

    gh pr checks --watch

The PR leaves draft only after all checks are green on the pushed head.

## Validation and Acceptance

Acceptance is behavioral, and because this is a pure refactor the behavior in question is "identical to before":

1. **Full suite passes.** From the repo root, `./scripts/test.sh` exits 0 and prints `test result: ok.` for every test binary. Every test that passed at `c1ebf64` still passes, except the ones deliberately relocated in M5 (which pass at their new home in `agent/tests/data_uploads/queue.rs`).

2. **The on-disk format is provably unchanged.** The new upload wire-pin test added in M1 fails if any of `entries` / `id` / `job` / `attempts` / `next_attempt_at` is renamed, reordered into a flattened shape, wrapped in an enum, or joined by a `version` field. It passes at M1 (before any refactor) and still passes at M6. The equivalent retention pin at `agent/tests/data_uploads/retention/queue.rs:86-110` likewise passes throughout. Concretely: writing the literal string `{"entries":[{"id":"...","job":{...},"attempts":0,"next_attempt_at":null}]}` to `upload_queue.json` and constructing the queue from it yields a queue of length 1 with that job — no silent wipe.

3. **Both gates hold at their current values.** `./scripts/covgate.sh` reports `✅` for `data_uploads/upload` at ≥ 96.00 and `data_uploads/retention` at ≥ 98.39, plus `✅` for the new `data_uploads/queue` at its seeded threshold. No `.covgate` value is lowered anywhere in the diff — verify with `git diff main -- '**/.covgate'` and confirm every changed line is a raise or a new file.

4. **Lint is clean.** `LINT_FIX=0 ./scripts/lint.sh` exits 0 — this is what CI's `lint` job runs, and it will not auto-fix for you.

5. **Preflight reports CLEAN, and CI is green on the pushed branch head.** `./scripts/preflight.sh` must print `Preflight clean`, and `gh pr checks` must show all checks passing on the exact commit that is pushed. **The PR does not leave draft, and this task is not reported complete, until both of those are true.** A local pass with an unpushed or stale head does not count.

6. **The duplication is actually gone.** `wc -l agent/src/data_uploads/upload/queue.rs agent/src/data_uploads/retention/queue.rs` shows both files reduced to shims (expect roughly 25–40 lines each, versus 193 and 180 today), and neither contains a `VecDeque`, a `persist`, or a `next_ready` body.

## Idempotence and Recovery

- `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/covgate.sh`, and `./scripts/preflight.sh` are all safe to run repeatedly. `lint.sh` mutates the working tree (fmt, clippy `--fix`); re-run `git status` after it and fold any changes into the current milestone's commit.
- Each milestone is one commit, so recovery from a bad milestone is `git reset --hard HEAD~1` (or `git revert` if already pushed). The milestone order is chosen so that each commit compiles and passes tests on its own, which also makes `git bisect` useful if a later problem appears.
- The riskiest step is M3/M4's covgate outcome. It is discovered by running `./scripts/covgate.sh`, which is read-only, so there is nothing to roll back — only a choice to make. Both recovery options (add tests; relocate the generic) are reversible edits within the same milestone.
- **The one genuinely dangerous failure mode is a wire-format change**, because `SingleThreadStateFile::new_with_default` wipes rather than errors. It is guarded by the M1 pin, which lands before any production edit and is never removed. If that test ever fails during M2–M5, stop and fix the serialization — do not adjust the test to match the new output.
- No migrations, no data backfill, no deploy-time steps. If the branch is abandoned entirely, `git checkout main` restores the pre-change state; nothing outside the repo is touched.
