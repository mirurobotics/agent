# Add the device-side upload queue module (`upload`)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, root `/home/ben/miru/workbench4/repos/agent`) | read-write | New `upload` module in the Rust agent crate: in-memory FIFO upload queue, actor, executor seam, tests, coverage gate. |

This plan lives in this repo's `plans/` because all code changes land here. No other repository is read or written. All file paths below are relative to the repo root (so `agent/src/upload/mod.rs` means `/home/ben/miru/workbench4/repos/agent/agent/src/upload/mod.rs` — the repo has a nested `agent/` crate directory).

All work happens on the branch `feat/upload-queue-module`, which already exists and is checked out, created off `origin/main` at commit `019555e`.

## Purpose / Big Picture

The agent (the Rust daemon running on customer devices) is gaining a data-upload feature: files matching customer-configured upload rules are detected on disk and uploaded to Miru's cloud. This plan delivers the device-side uploader core: a new `upload` module containing an actor that owns an in-memory FIFO queue of `UploadJob`s and processes exactly one job at a time through a pluggable executor seam.

After this change, a developer can construct an `Uploader` with any `UploadExecutor` implementation, enqueue jobs, and observe FIFO processing with dedup, retry-with-backoff, requeue-at-tail, a global attempt cap, capacity pruning, and responsive shutdown — all proven by the test suite (`./scripts/test.sh` passes with the new `upload` tests, `./scripts/preflight.sh` is clean).

Deliberately out of scope for this PR (deferred to follow-up PRs):

- The file scanner lives in a separate open PR. This module must not reference any `scan` module. A later "bridge worker" PR maps scanner stable-file events into `UploadJob`s.
- The real executor (backend credential fetch via `POST /uploads` returning short-lived S3 STS / GCS OAuth2 credentials, then native-SDK transfer, then confirm) comes later. This PR ships only a placeholder `LogExecutor` that logs and returns `Ok`.
- No wiring into `AppState` or `agent/src/app/run.rs`. Nothing constructs an `Uploader` in production code yet.

## Progress

- [x] Milestone 1: module scaffold, errors, job type, registration. (2026-07-08: all tests pass, 3 new `upload::job::dedup_key` tests)
- [ ] Milestone 2: queue with dedup, prune, reject-on-full, plus unit tests.
- [ ] Milestone 3: executor trait, `LogExecutor`, mock test executor.
- [ ] Milestone 4: `Uploader` actor with interleaved run loop, retry/requeue/cap semantics, plus tests.
- [ ] Milestone 5: coverage ratchet, preflight, plan bookkeeping.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

(Add implementation-time entries as you go. Design decisions locked at authoring time are embedded in Context and Orientation below; they are not open questions.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

### The repo in one paragraph

This repo is a Cargo workspace. The main crate is `miru-agent` in `agent/` (source in `agent/src/`, integration tests in `agent/tests/`, config in `agent/Cargo.toml`). `agent/src/lib.rs` lists all public modules alphabetically. Test files in `agent/tests/` mirror the `agent/src/` module structure and are registered in `agent/tests/mod.rs`. Shared test mocks live in `agent/tests/mocks/` (registered in `agent/tests/mocks/mod.rs`). Every source module directory carries a `.covgate` file containing a single line with a minimum region-coverage percentage (cargo llvm-cov region coverage — see `scripts/lib/covgate.sh`) (e.g. `93.06`), enforced by `./scripts/covgate.sh`. Repo conventions live in `AGENTS.md`; read its "Import ordering" and "Error handling" sections if anything below is unclear.

Key commands, all run from the repo root `/home/ben/miru/workbench4/repos/agent`:

    ./scripts/test.sh       # RUST_LOG=off cargo test --features test
    ./scripts/lint.sh       # import linter, cargo fmt --check, clippy -D warnings, machete, audit
    ./scripts/covgate.sh    # cargo llvm-cov per-module vs .covgate floors
    ./scripts/preflight.sh  # all of the above in parallel (~2-5 min)

The `--features test` flag matters: test-only helpers are gated behind `#[cfg(feature = "test")]` and `agent/Cargo.toml` already declares `[features] test = []`.

Every source file uses three import groups separated by blank lines, in this order, each introduced by a comment line:

    // standard crates
    use std::sync::Arc;

    // internal crates
    use crate::filesys::File;

    // external crates
    use tokio::sync::mpsc;

The custom import linter (part of `lint.sh`) enforces this. It also flags "field-by-field assert" in tests: 4+ `assert_eq!` calls on fields of the same variable in one test function. Prefer asserting whole structs; where a struct compare is vacuous, keep field asserts under that threshold or suppress with `// lint:allow(field-by-field-assert)` inside the test body.

### Terms

- Actor: a tokio task that owns mutable state and communicates via an mpsc `Command` channel; callers hold a cheap handle wrapping the `Sender` and get replies over per-command `oneshot` channels. `TokenManager` in `agent/src/authn/token_mngr.rs` is the house pattern (detailed below).
- Dedup key: the triple `(upload_rule_id, file path, digest)` identifying a job. Two jobs with the same key are the same upload.
- Stale job: a queued job whose source file no longer exists, or whose current size or mtime no longer match what the job recorded. Stale jobs are safe to drop because the bytes they described are gone.
- Round: up to 3 consecutive in-place executor attempts on the same job (with backoff sleeps between them) before the job is requeued at the tail.
- In flight: the single job currently being processed by the actor, including its backoff sleeps between in-place attempts.

### Locked design decisions

These were decided before authoring; implement them as written.

1. This PR/branch ships the uploader module alone. Bridge worker and app wiring are follow-up PRs.
2. The queue is in-memory only — no persistence. Restart recovery comes later from scanner re-observation plus backend digest dedup; do not build any on-disk state.
3. Retry policy: on executor failure, retry in place with exponential backoff for 3 total in-place attempts per round (backoff base 1s, growth factor 2, max 30s, via the existing `cooldown::Backoff` and `cooldown::calc`), then requeue at tail. A global per-job attempt cap of 9 total attempts (3 rounds) bounds retries across rounds; when exhausted, drop the job with a `warn!` log. The backoff exponent is the job's total attempt count so far minus one, so sleeps between in-place retries follow the sequence 1s, 2s (round 1), 8s, 16s (round 2), 30s, 30s (round 3, capped from 64/128). No sleep occurs on the round-ending failure that triggers a requeue.
4. Queue capacity defaults to 1024. When full at enqueue: prune stale jobs first; if still full after pruning, reject the enqueue with the distinct error variant `UploadErr::QueueFullErr`. Pruned and rejected outcomes are logged (`warn!`).
5. Dedup: enqueue is a no-op success returning `EnqueueOutcome::Duplicate` when a job with the same dedup key is already pending in the queue or currently in flight. The in-flight key stays claimed for the whole round including backoff sleeps; when the job is requeued at tail its key returns to the queue, so there is no window where a duplicate can slip in (requeue happens synchronously in the actor loop before further commands are processed).
6. Module name `upload`; job type exactly:

        UploadJob {
            file: filesys::File,
            size: u64,
            digest: String,          // "sha256:<hex>" as produced by files::hash
            mtime: chrono::DateTime<Utc>,
            upload_rule_id: String,
            deployment_id: String,
            release_id: String,
        }

7. Executor seam: the trait is declared in the repo's `http::ClientI` RPITIT style (`agent/src/http/client.rs` lines 30–38): `pub trait UploadExecutor: Send + Sync { fn upload(&self, job: &UploadJob) -> impl Future<Output = Result<(), UploadErr>> + Send; }`. The explicit `+ Send` on the returned future is what lets the generic `Uploader::spawn` hand `worker.run()` to `tokio::spawn` (exactly as `ClientI`'s `+ Send` does for `TokenManager::spawn`); plain `#[allow(async_fn_in_trait)]` leaves the future without a Send bound and fails E0277 at `tokio::spawn` in generic code — that AFIT style is reserved for handle-side ext traits (`TokenManagerExt`, `UploaderExt`), which are awaited at concrete call sites. Implementations still write `async fn upload(...)`, as `impl ClientI for Client` does (`client.rs:45`). This PR ships a placeholder `LogExecutor` (logs the job at `info!`, returns `Ok`). Tests use a scripted mock executor.
8. The actor mirrors the TokenManager pattern (spawn returning `(handle, JoinHandle<()>)`, `Command` enum with `oneshot` respond_to channels, public `UploaderExt: Send + Sync` trait) with one critical difference spelled out under "The run loop" in Plan of Work: the run loop interleaves command intake with the in-flight upload using `tokio::select!`, so `enqueue`/`shutdown` stay responsive during a long upload. On shutdown the in-flight upload future is dropped (cancelled); future real executors must be cancel-safe (an interrupted transfer is re-driven later via scanner re-observation and backend digest dedup — document this on the trait). A `sleep_fn` (workers-style `Fn(Duration) -> Fut`) is injected into the actor for backoff so tests never real-sleep.

### Existing building blocks (verified paths and signatures)

Actor template — `agent/src/authn/token_mngr.rs`:

- `TokenManager::spawn(buffer_size, ...) -> Result<(Self, JoinHandle<()>), AuthnErr>` (lines 139-158): creates `mpsc::channel(buffer_size)`, builds a `Worker` owning the `Receiver`, `tokio::spawn(worker.run())`, returns a handle struct wrapping `mpsc::Sender<Command>`.
- `Command` enum (lines 87-97): one variant per operation, each carrying `respond_to: oneshot::Sender<Result<..., AuthnErr>>`.
- Worker run loop (lines 105-131): `while let Some(cmd) = self.receiver.recv().await { match cmd { ... } }`; `Shutdown` responds `Ok(())` then `break`s. A local `dispatch!` macro (lines 16-23) sends a response and logs an `error!` if the receiver hung up.
- Handle helper `send_command` (lines 160-181): creates the oneshot pair, sends the command mapping send failure to `SendActorMessageErr` and receive failure to `ReceiveActorMessageErr`, both built with `trace!()`.
- Public ext trait (lines 28-33):

        #[allow(async_fn_in_trait)]
        pub trait TokenManagerExt: Send + Sync {
            async fn shutdown(&self) -> Result<(), AuthnErr>;
            ...
        }

  plus a blanket-style `impl TokenManagerExt for Arc<TokenManager>` delegating to `self.as_ref()`.

Actor-message errors — `agent/src/cache/errors.rs` defines the canonical structs; other modules alias them (see `agent/src/events/errors.rs` lines 4-5):

    pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
    pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

Both have shape `{ source: Box<dyn std::error::Error + Send + Sync>, trace: Box<Trace> }`.

Error conventions — `agent/src/errors/mod.rs` defines `pub trait Error` (all methods defaulted: `code()`, `http_status()`, `params()`, `is_network_conn_err()`), the `Trace` struct, the `trace!()` macro (returns `Box<Trace>`), and the `impl_error!` macro which implements `crate::errors::Error` for an aggregating enum by delegating to each variant. The house pattern (exemplar `agent/src/events/errors.rs`): per-error structs deriving `#[derive(Debug, thiserror::Error)]` with an `#[error("...")]` message and a `pub trace: Box<Trace>` field, each with `impl crate::errors::Error for X {}`; an aggregating enum (`EventsErr`) with `#[error(transparent)]` variants; `#[from]` or manual `From` impls; finally `crate::impl_error!(EventsErr { Variant1, Variant2, ... });`.

Filesystem — `agent/src/filesys/` (module root re-exports `Dir`, `File`, `FileSysErr`, `PathExt`):

- `File` (`file.rs` lines 14-17): `#[derive(Clone, Debug, PartialEq, Eq)] pub struct File { path: PathBuf }`; `File::new(path)`; `file.path() -> &PathBuf` and `file.exists() -> bool` come from the `PathExt` trait (`path.rs`), so `use crate::filesys::PathExt;` is needed to call them.
- `files::hash(&File) -> Result<String, FileSysErr>` (`files.rs` line 47): streams the file through sha256 and returns `format!("sha256:{digest:x}")`.
- `files::metadata(&File) -> Result<Metadata, FileSysErr>` (line 432): NotFound maps to `FileSysErr::PathDoesNotExistErr`; other IO errors map to `FileSysErr::FileMetadataErr`.
- `files::last_modified(&File) -> Result<SystemTime, FileSysErr>` (line 453) and `files::size(&File) -> Result<u64, FileSysErr>` (line 460), both thin wrappers over `metadata`.
- Convert `SystemTime` to `chrono::DateTime<Utc>` via `DateTime::<Utc>::from(system_time)` (exact, nanosecond-preserving). Jobs must record `mtime` through this same conversion so staleness equality comparison is reliable.
- Test helpers: `dirs::create_temp(prefix) -> Result<Dir, FileSysErr>` (`dirs.rs` line 44), `dir.file(name) -> File`, `files::write_string(&File, &str, WriteOptions) -> Result<(), FileSysErr>` with `WriteOptions::OVERWRITE_ATOMIC` (usage exemplar: `agent/tests/disk/device.rs`).

Backoff — `agent/src/cooldown/mod.rs` (entire file):

    pub struct Backoff { pub base_secs: i64, pub growth_factor: i64, pub max_secs: i64 }
    pub fn calc(backoff: &Backoff, exp: u32) -> i64   // min(base * growth^exp, max), saturating

`sleep_fn` injection — `agent/src/workers/token_refresh.rs` lines 34-42 (also `poller.rs`): functions take `sleep_fn: F` where `F: Fn(Duration) -> Fut, Fut: Future<Output = ()> + Send`. Production passes `tokio::time::sleep`; tests pass a no-op or recording closure.

Test style exemplar — `agent/tests/events/hub.rs`: file-level helper constructors, then nested `mod <method> { use super::*; #[tokio::test] async fn <behavior>() { ... } }` blocks. Tests import the crate as `miru_agent` (crate name `miru-agent`). Test files register in a `mod.rs` chain: `agent/tests/upload/mod.rs` lists its submodules (`pub mod queue;` etc.) and `agent/tests/mod.rs` lists `pub mod upload;`.

Coverage — each `agent/src/<module>/` directory has a `.covgate` file containing a single float line (existing floors for reference: `events` 93.06, `workers` 83.13). `./scripts/covgate.sh` enforces; `./scripts/update-covgates.sh` recomputes.

Naming check — there is no existing `upload` module. `agent/src/models/upload_rule.rs` and `agent/src/disk/upload_rules.rs` exist but are unrelated to this module: do not touch them, and this PR needs no imports from them.

## Plan of Work

New files: `agent/src/upload/{mod.rs, errors.rs, job.rs, queue.rs, executor.rs, uploader.rs, .covgate}` and `agent/tests/upload/{mod.rs, job.rs, queue.rs, executor.rs, uploader.rs}` plus `agent/tests/mocks/upload_executor.rs`. Edits: `agent/src/lib.rs`, `agent/tests/mod.rs`, `agent/tests/mocks/mod.rs`. The five milestones below each end in one commit.

### Milestone 1 — scaffold, errors, job type, registration

`agent/src/upload/mod.rs` declares submodules and re-exports the public surface. In this milestone only `errors` and `job` exist; extend the lists in later milestones:

    pub mod errors;
    pub mod job;

    pub use self::errors::UploadErr;
    pub use self::job::{DedupKey, UploadJob};

`agent/src/upload/errors.rs` follows the `agent/src/events/errors.rs` pattern exactly. Contents:

- Aliases: `pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;` and the same for `ReceiveActorMessageErr`.
- `QueueFullErr { pub capacity: usize, pub file: String, pub trace: Box<Trace> }` with message like `"upload queue is full (capacity {capacity}); rejected job for file {file}"`, and `impl crate::errors::Error for QueueFullErr {}`. This is the decision-4 "distinct error variant" for reject-on-full.
- `ExecutorErr { pub source: Box<dyn std::error::Error + Send + Sync>, pub trace: Box<Trace> }` with message `"upload executor error: {source}"` and empty `Error` impl. This is the wrapper future real executors (and the test mock) use to surface failures.
- Aggregating enum:

        #[derive(Debug, thiserror::Error)]
        pub enum UploadErr {
            #[error(transparent)]
            FileSysErr(#[from] crate::filesys::FileSysErr),
            #[error(transparent)]
            QueueFullErr(QueueFullErr),
            #[error(transparent)]
            ExecutorErr(ExecutorErr),
            #[error(transparent)]
            SendActorMessageErr(SendActorMessageErr),
            #[error(transparent)]
            ReceiveActorMessageErr(ReceiveActorMessageErr),
        }

        crate::impl_error!(UploadErr { FileSysErr, QueueFullErr, ExecutorErr, SendActorMessageErr, ReceiveActorMessageErr });

  No `JobAttemptsExhaustedErr`: attempt exhaustion is a warn-and-drop inside the actor, never returned to a caller, so an error type would be dead weight.

`agent/src/upload/job.rs` defines the job and its dedup key. `UploadJob` has exactly the fields from locked decision 6, all `pub`, deriving `Clone, Debug, PartialEq` (plain derives — this is an internal type with no wire contract, so no serde). `DedupKey` is:

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub struct DedupKey {
        pub upload_rule_id: String,
        pub path: PathBuf,
        pub digest: String,
    }

with `impl UploadJob { pub fn dedup_key(&self) -> DedupKey { ... } }` cloning `self.file.path()` (requires `use crate::filesys::PathExt;`). Document on the struct that `digest` is the `files::hash` format `"sha256:<hex>"` and `mtime` must come from `DateTime::<Utc>::from(files::last_modified(...))`.

Registration: add `pub mod upload;` to `agent/src/lib.rs` alphabetically (between `telemetry` and `version`). Create `agent/src/upload/.covgate` containing the single line `0.00` (provisional; ratcheted in Milestone 5 — a real floor now would fail covgate before tests exist). Create `agent/tests/upload/mod.rs` containing `pub mod job;` and add `pub mod upload;` to `agent/tests/mod.rs` alphabetically (between `test_utils` and `version` — the tests list also contains `mocks` and `test_utils`, keep it alphabetical). Create `agent/tests/upload/job.rs` with a `mod dedup_key` block: same-key jobs produce equal `DedupKey`s; changing any of rule id, path, or digest produces unequal keys; equal size/mtime alone do not make keys equal.

### Milestone 2 — the queue

`agent/src/upload/queue.rs` defines the FIFO queue as a plain (non-actor) struct so it is unit-testable in isolation. Public surface:

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum EnqueueOutcome { Enqueued, Duplicate }

    #[derive(Clone, Debug)]
    pub struct PendingJob {
        pub job: UploadJob,
        pub attempts: u32,   // total executor attempts so far, across rounds
    }

    pub struct UploadQueue { jobs: VecDeque<PendingJob>, capacity: usize }

    impl UploadQueue {
        pub fn new(capacity: usize) -> Self;
        pub fn len(&self) -> usize;
        pub fn is_empty(&self) -> bool;
        pub fn contains(&self, key: &DedupKey) -> bool;
        pub async fn enqueue(&mut self, job: UploadJob) -> Result<EnqueueOutcome, UploadErr>;
        pub async fn requeue(&mut self, pending: PendingJob) -> Result<(), UploadErr>;
        pub fn pop_front(&mut self) -> Option<PendingJob>;
    }

`EnqueueOutcome` has no `Rejected` variant: rejection-on-full is `Err(UploadErr::QueueFullErr)` per locked decision 4, while `Duplicate` is the decision-5 no-op success. `PendingJob` carries the cross-round attempt count so a requeued job keeps its history and the global cap of 9 holds.

`enqueue` logic, in order: (1) dedup — if `self.contains(&job.dedup_key())`, `info!` and return `Ok(Duplicate)` (dedup wins even when full); (2) capacity — if `self.jobs.len() >= self.capacity`, run `prune_stale` (below); if still full, `warn!` and return `Err(QueueFullErr { ... })`; (3) push `PendingJob { job, attempts: 0 }` at the tail, return `Ok(Enqueued)`. `requeue` is the same minus the dedup check (the requeued job's key cannot be in the queue: it was claimed as in-flight the entire time) and pushes the given `PendingJob` (preserved attempts) at the tail.

`prune_stale` (private async helper): for each queued job, call `files::metadata(&job.file).await`. `Err(FileSysErr::PathDoesNotExistErr(_))` means the file is gone — stale. Any other `Err` is a transient IO problem — keep the job (dropping work on a hiccup is worse than a full queue). On `Ok(md)`, the job is stale if `md.len() != job.size` or `DateTime::<Utc>::from(md.modified().unwrap_or(SystemTime::now())) != job.mtime`. Remove stale jobs and `warn!` each with rule id, path, digest, and the reason. (Note `VecDeque::retain` cannot await; collect stale indices or rebuild the deque instead.)

Update `agent/src/upload/mod.rs`: add `pub mod queue;` and `pub use self::queue::{EnqueueOutcome, PendingJob, UploadQueue};`.

Tests in `agent/tests/upload/queue.rs` (register `pub mod queue;` in `agent/tests/upload/mod.rs`). File-level helpers: `make_file(dir, name, contents)` writing a real temp file via `dirs::create_temp` + `files::write_string(..., WriteOptions::OVERWRITE_ATOMIC)`, and `make_job(&file)` building a valid `UploadJob` from `files::hash`, `files::size`, `files::last_modified` (through the `DateTime::<Utc>::from` conversion). Test list is in Validation and Acceptance.

### Milestone 3 — executor seam

`agent/src/upload/executor.rs`:

    pub trait UploadExecutor: Send + Sync {
        fn upload(
            &self,
            job: &UploadJob,
        ) -> impl std::future::Future<Output = Result<(), UploadErr>> + Send;
    }

    pub struct LogExecutor;

    impl UploadExecutor for LogExecutor {
        async fn upload(&self, job: &UploadJob) -> Result<(), UploadErr> {
            info!("LogExecutor: pretending to upload {job:?}");
            Ok(())
        }
    }

Doc-comment the trait with the cancel-safety contract: the actor may drop an in-progress `upload` future at shutdown, so implementations must tolerate being cancelled at any await point (interrupted transfers are re-driven after restart via scanner re-observation plus backend digest dedup). Also note the future real executor's job (credential fetch via `POST /uploads`, native-SDK transfer, confirm) so the seam's purpose is clear.

Update `agent/src/upload/mod.rs`: add `pub mod executor;` and `pub use self::executor::{LogExecutor, UploadExecutor};`.

Mock in `agent/tests/mocks/upload_executor.rs` (register `pub mod upload_executor;` alphabetically in `agent/tests/mocks/mod.rs`). Suggested shape — the required capabilities are scripted per-call results, recorded calls, an "upload started" signal, and a hold-until-released call for in-flight tests:

    pub enum MockStep {
        Ok,
        Err,
        Hang(tokio::sync::oneshot::Receiver<Result<(), UploadErr>>),
    }

    pub struct MockUploadExecutor {
        script: std::sync::Mutex<VecDeque<MockStep>>,
        pub calls: std::sync::Mutex<Vec<UploadJob>>,
        started_tx: tokio::sync::mpsc::UnboundedSender<()>,
    }

`MockUploadExecutor::new() -> (Arc<Self>, UnboundedReceiver<()>)` returns the started-notification receiver to the test. `upload` records the job in `calls`, pops the next `MockStep` (treat an empty script as `Ok`), drops the mutex guard, sends `()` on `started_tx`, then: `Ok` returns `Ok(())`; `Err` returns a scripted `UploadErr::ExecutorErr` built from `Box::new(std::io::Error::other("scripted failure"))` and `miru_agent::trace!()`; `Hang(rx)` awaits `rx` and returns the sent result (or `Ok(())` if the sender was dropped). `Hang` doubles as a test-controlled result: the test holds the `oneshot::Sender`, knows the upload started via the notification channel, and releases it with `Ok` or `Err` when ready. The mock implements the trait with a plain `async fn upload`, and its future is `Send` because no `std::sync::Mutex` guard is held across an await — never hold one across an await.

Milestone tests: tests live in `agent/tests/upload/executor.rs` (registered as `pub mod executor;` in `agent/tests/upload/mod.rs`, mirroring `agent/src/upload/executor.rs`), covering `LogExecutor` returning `Ok` and the mock following its script (Ok, Err, empty-script default).

### Milestone 4 — the Uploader actor

`agent/src/upload/uploader.rs`, mirroring `token_mngr.rs` structurally (handle + `Command` + `Worker` + ext trait + `Arc` delegation + `send_command` helper + `dispatch!`-style responses):

    #[derive(Clone, Debug)]
    pub struct UploaderOptions {
        pub queue_capacity: usize,     // default 1024
        pub in_place_attempts: u32,    // default 3, attempts per round
        pub max_total_attempts: u32,   // default 9, global cap (3 rounds)
        pub backoff: cooldown::Backoff, // default base 1s, growth 2, max 30s
    }
    // impl Default with those values

    pub(crate) enum Command {
        Enqueue { job: UploadJob, respond_to: oneshot::Sender<Result<EnqueueOutcome, UploadErr>> },
        Len { respond_to: oneshot::Sender<Result<usize, UploadErr>> },
        Shutdown { respond_to: oneshot::Sender<Result<(), UploadErr>> },
    }

    #[allow(async_fn_in_trait)]
    pub trait UploaderExt: Send + Sync {
        async fn enqueue(&self, job: UploadJob) -> Result<EnqueueOutcome, UploadErr>;
        async fn len(&self) -> Result<usize, UploadErr>;
        async fn shutdown(&self) -> Result<(), UploadErr>;
    }

    pub struct Uploader { sender: mpsc::Sender<Command> }

    impl Uploader {
        pub fn spawn<ExecutorT, F, Fut>(
            buffer_size: usize,
            executor: Arc<ExecutorT>,
            options: UploaderOptions,
            sleep_fn: F,
        ) -> Result<(Self, JoinHandle<()>), UploadErr>
        where
            ExecutorT: UploadExecutor + 'static,
            F: Fn(Duration) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = ()> + Send + 'static,
        { ... }
    }

`len()` reports queued jobs only, excluding any in-flight job — document this on the trait method. Production callers will pass `tokio::time::sleep` as `sleep_fn`; tests pass no-op or recording closures.

The run loop — the critical design point. Unlike TokenManager's strictly sequential `while recv()` loop, the worker must keep serving `enqueue`/`len`/`shutdown` while an upload (or a backoff sleep) is in progress. The shape that avoids borrow conflicts: the in-flight future is built from an `Arc` clone of the executor and a clone of the job, so it borrows nothing from `self`, and `self` stays free for command handling inside the `select!` arms.

Worker state: `receiver`, `queue: UploadQueue`, `executor: Arc<ExecutorT>`, `options`, `sleep_fn`, `in_flight: Option<DedupKey>`. Outer loop:

    loop {
        match self.queue.pop_front() {
            None => match self.receiver.recv().await {         // idle: nothing to interleave
                None => break,                                  // all senders dropped
                Some(cmd) => if shutdown was handled { break },
            },
            Some(pending) => if self.process(pending).await requested shutdown { break },
        }
    }

`process(pending)` sets `self.in_flight = Some(pending.job.dedup_key())` for its whole duration (uploads and backoff sleeps — this is what makes decision-5 in-flight dedup airtight), clears it before returning, and runs the round state machine. For each in-place attempt `1..=options.in_place_attempts`: increment `pending.attempts`, then drive the upload interleaved:

    let exec = self.executor.clone();
    let job = pending.job.clone();
    let mut upload_fut = Box::pin(async move { exec.upload(&job).await });
    loop {
        tokio::select! {
            res = &mut upload_fut => break res,               // attempt finished
            cmd = self.receiver.recv() => match cmd {
                None => return Shutdown,
                Some(cmd) => { /* handle_command(cmd).await; on Shutdown: respond Ok then return Shutdown */ }
            }
        }
    }

`mpsc::Receiver::recv` is cancel-safe, so losing the race in `select!` never drops a command. Awaiting `handle_command` inside an arm body merely pauses polling of `upload_fut`; the upload resumes on the next loop iteration. Returning `Shutdown` drops `upload_fut`, cancelling the in-flight upload — that is the documented shutdown contract. On attempt results:

- `Ok(())`: `info!` success, return (job done).
- `Err(e)` with `pending.attempts >= options.max_total_attempts`: `warn!` (include attempts, rule id, path, digest, and `{e:?}`) and drop the job; return.
- `Err(e)`, more in-place attempts left in this round: compute `secs = cooldown::calc(&options.backoff, pending.attempts - 1)`, drive `(self.sleep_fn)(Duration::from_secs(secs.max(0) as u64))` through the same `select!` interleave pattern as the upload, then continue to the next attempt.
- `Err(e)` on the round's last attempt (cap not reached): `self.queue.requeue(pending).await`; on `Err(QueueFullErr)` `warn!` and drop the job; return.

`handle_command`: `Enqueue` first checks `self.in_flight` — a matching key responds `Ok(Duplicate)` without touching the queue; otherwise it delegates to `self.queue.enqueue(job).await` and responds with the result. `Len` responds `Ok(self.queue.len())`. `Shutdown` responds `Ok(())` and signals the caller loop to break; the queue's remaining jobs are simply dropped (in-memory only, by design). Copy TokenManager's `dispatch!` macro for send-or-log-error responses and its `send_command` helper (mapping to `SendActorMessageErr`/`ReceiveActorMessageErr` with `trace!()`) for the handle side. Implement `UploaderExt` for `Uploader` and for `Arc<Uploader>` exactly as `token_mngr.rs` does.

Update `agent/src/upload/mod.rs`: add `pub mod uploader;` and `pub use self::uploader::{Uploader, UploaderExt, UploaderOptions};`.

Tests live in a new `agent/tests/upload/uploader.rs` (registered as `pub mod uploader;` in `agent/tests/upload/mod.rs`) using the mock, the started-notification channel for deterministic sequencing, no-op sleeps (`|_: Duration| async {}`) except in the backoff test (a closure pushing each `Duration` into an `Arc<std::sync::Mutex<Vec<Duration>>>` before returning an immediately-ready future), and `tokio::time::timeout` to fail fast instead of hanging. Test list in Validation and Acceptance. Tests use only temp files (`dirs::create_temp`), so no `#[serial]` is needed.

### Milestone 5 — coverage ratchet, preflight, bookkeeping

Measure coverage, replace the provisional `0.00` in `agent/src/upload/.covgate` with a floor just under the measured value (e.g. measured 91.4 → set `91.00`), run the full preflight, update this plan's living sections, and move the plan file from `plans/active/` to `plans/completed/`.

## Concrete Steps

All commands run from the repo root `/home/ben/miru/workbench4/repos/agent` unless stated otherwise. Confirm the branch first:

    git status --short --branch
    # expect: ## feat/upload-queue-module

Then promote this plan to active before implementing (plans follow the backlog → active → completed lifecycle; `plans/active/` does not yet exist in the repo):

    mkdir -p plans/active
    git mv plans/backlog/20260708-upload-queue-module.md plans/active/
    git commit -m "docs(plans): promote upload-queue-module plan to active"

### Milestone 1

1. Create `agent/src/upload/mod.rs`, `agent/src/upload/errors.rs`, `agent/src/upload/job.rs`, and `agent/src/upload/.covgate` (single line `0.00`) as described in Plan of Work.
2. Edit `agent/src/lib.rs`: insert `pub mod upload;` between `pub mod telemetry;` and `pub mod version;`.
3. Create `agent/tests/upload/mod.rs` (`pub mod job;`) and `agent/tests/upload/job.rs`; edit `agent/tests/mod.rs` to add `pub mod upload;` alphabetically.
4. Run the tests:

        ./scripts/test.sh
        # expect: all tests pass, including the new upload::job::dedup_key tests; 0 failed

5. Commit:

        git add agent/src/upload agent/src/lib.rs agent/tests/upload agent/tests/mod.rs
        git commit -m "feat(upload): add module scaffold with job type and errors"

### Milestone 2

1. Create `agent/src/upload/queue.rs`; add `pub mod queue;` plus re-exports to `agent/src/upload/mod.rs`.
2. Create `agent/tests/upload/queue.rs` with the helpers and tests from Validation and Acceptance; add `pub mod queue;` to `agent/tests/upload/mod.rs`.
3. Run `./scripts/test.sh` — expect all tests pass, including the new `upload::queue` tests.
4. Commit:

        git add agent/src/upload agent/tests/upload
        git commit -m "feat(upload): add FIFO queue with dedup, prune, and reject-on-full"

### Milestone 3

1. Create `agent/src/upload/executor.rs`; add `pub mod executor;` plus re-exports to `agent/src/upload/mod.rs`.
2. Create `agent/tests/mocks/upload_executor.rs`; add `pub mod upload_executor;` to `agent/tests/mocks/mod.rs` alphabetically.
3. Create `agent/tests/upload/executor.rs` with the executor tests; add `pub mod executor;` to `agent/tests/upload/mod.rs`.
4. Run `./scripts/test.sh` — expect all tests pass.
5. Commit:

        git add agent/src/upload agent/tests/mocks agent/tests/upload
        git commit -m "feat(upload): add UploadExecutor seam with LogExecutor placeholder"

### Milestone 4

1. Create `agent/src/upload/uploader.rs`; add `pub mod uploader;` plus re-exports to `agent/src/upload/mod.rs`.
2. Create `agent/tests/upload/uploader.rs` with the actor tests from Validation and Acceptance; add `pub mod uploader;` to `agent/tests/upload/mod.rs`.
3. Run `./scripts/test.sh` — expect all tests pass; none of the new tests real-sleep, so the suite stays fast.
4. Commit:

        git add agent/src/upload agent/tests/upload
        git commit -m "feat(upload): add Uploader actor with retry, requeue, and shutdown"

### Milestone 5

1. Measure coverage and ratchet the gate:

        ./scripts/covgate.sh
        # note the measured percentage for agent/src/upload, e.g. 91.43

   Edit `agent/src/upload/.covgate` to a floor just below the measured value (round down to the nearest whole-ish figure, e.g. `91.00`). Re-run `./scripts/covgate.sh` — expect PASS for `upload` and all other modules. If `upload` coverage is far below neighboring floors (events 93.06, workers 83.13), add tests for the uncovered branches (typically error-response paths) rather than accepting a low floor.

2. Full preflight:

        ./scripts/preflight.sh
        # expect: test, lint, covgate all green (~2-5 min)

   If lint complains about import ordering or fmt, fix and re-run — see Idempotence and Recovery.

3. Update this plan: check off Progress items with timestamps, fill Surprises & Discoveries, Decision Log, and Outcomes & Retrospective; then move the plan file:

        git mv plans/active/20260708-upload-queue-module.md plans/completed/

4. Commit:

        git add -A agent/src/upload/.covgate plans/
        git commit -m "chore(upload): ratchet coverage gate and complete plan"

## Validation and Acceptance

All validation is behavioral, via `./scripts/test.sh` from the repo root. Expected final outcome: all tests pass, with roughly 20 new tests under `upload::` (exact count may vary slightly; every test named below must exist and pass), and `./scripts/preflight.sh` exits clean.

`agent/tests/upload/job.rs` — `mod dedup_key`:

- `equal_for_same_rule_path_digest`: two jobs sharing rule id, path, digest (but different deployment/release ids) have equal keys.
- `differs_when_any_component_differs`: changing rule id, path, or digest each yields an unequal key.

`agent/tests/upload/queue.rs`:

- `mod enqueue`:
  - `returns_enqueued_for_new_job`: fresh queue, `enqueue` returns `Ok(Enqueued)`, `len() == 1`.
  - `returns_duplicate_for_same_key`: enqueuing a key-equal job returns `Ok(Duplicate)` and `len()` stays 1.
  - `full_queue_with_stale_job_prunes_and_accepts`: capacity-2 queue holding one fresh job and one job whose file was deleted (or rewritten so size/mtime changed); a third enqueue prunes the stale job, returns `Ok(Enqueued)`, and `len() == 2` with the stale key absent.
  - `full_queue_of_fresh_jobs_returns_queue_full_err`: capacity-1 queue with a fresh job rejects a new key with `Err(UploadErr::QueueFullErr(_))`.
  - `changed_content_makes_job_stale`: overwriting a queued job's file with different contents (size change) makes it prunable; missing file likewise (may be a second test, `missing_file_makes_job_stale`).
- `mod pop_front`:
  - `returns_jobs_in_fifo_order`: three enqueued jobs pop in insertion order with `attempts == 0`.
- `mod requeue`:
  - `preserves_attempts_and_appends_at_tail`: requeue a `PendingJob { attempts: 3 }` behind an existing job; pops return the other job first, then the requeued one with `attempts == 3`.

`agent/tests/upload/executor.rs`:

- `log_executor_returns_ok`: `LogExecutor.upload(&job)` returns `Ok`.
- `mock_follows_script`: mock scripted `[Ok, Err]` returns `Ok` then `Err(ExecutorErr)`, records both calls; an empty script defaults to `Ok`.

`agent/tests/upload/uploader.rs`:

- `mod uploader` (nested per-behavior mods or flat, matching hub.rs style):
  - `processes_enqueued_job`: enqueue one job → `Ok(Enqueued)`; await the started notification; mock recorded exactly that job; `len()` returns 0 afterwards.
  - `duplicate_while_in_flight_returns_duplicate`: script `Hang`; enqueue job A; await started; enqueue key-equal A' → `Ok(Duplicate)`; release the hang with `Ok`; shutdown; mock recorded A exactly once.
  - `failing_round_requeues_at_tail_behind_later_job`: script `[Hang, Err, Err, Ok, Ok]`; enqueue A; await started; enqueue B (goes behind A's requeue slot — B is queued while A is in flight); release the hang with `Err`; await the remaining four started notifications; recorded call order is exactly `[A, A, A, B, A]` (three in-place attempts for A, then B, then A's second round succeeding).
  - `global_attempt_cap_drops_job`: script 9 `Err` steps; enqueue A; await 9 started notifications (three rounds); enqueue B; await 1 more started; recorded calls are 9 A's then B — proving A was dropped at the cap with the actor still healthy. (The drop's `warn!` is not asserted; behavior is.)
  - `retry_backoff_follows_expected_sequence`: recording `sleep_fn`; script `[Err, Err, Err, Err, Err, Err, Ok]`; after the 7th started notification, recorded sleeps are exactly `[1s, 2s, 8s, 16s]` — in-place sleeps only, none around the two requeues.
  - `shutdown_during_in_flight_upload_returns_promptly`: script `Hang` (never released); enqueue; await started; `tokio::time::timeout(Duration::from_secs(1), uploader.shutdown())` returns `Ok(Ok(()))` — the in-flight future was dropped rather than awaited.
  - `len_reports_queued_jobs`: script `Hang`; enqueue A (goes in flight), enqueue B and C; `len()` returns 2 (in-flight A excluded); release, shutdown.

Coverage acceptance: `./scripts/covgate.sh` passes with `agent/src/upload/.covgate` set to a real floor (no longer `0.00`) that is at or just below measured coverage. Lint acceptance: `./scripts/lint.sh` reports no violations (import groups ordered, fmt clean, clippy clean with `-D warnings`, no unused deps).

## Idempotence and Recovery

- All test/lint/coverage commands are read-only and safely repeatable.
- Each milestone is one commit; if a milestone goes sideways before its commit, `git status` + `git checkout -- <file>` (or `git stash`) restores the last good state. After a bad commit, `git reset --soft HEAD~1` preserves the work for fixing. Do not rebase or force-push once the branch is pushed for review.
- File creation and `mod` registration edits are idempotent: re-applying them is a no-op if already present. Duplicate `pub mod` lines fail compilation loudly — just remove the duplicate.
- If `./scripts/test.sh` panics flakily on shared resources, re-run in isolation; the new upload tests use only per-test temp dirs and injected sleeps, so they should be deterministic. A hang in an uploader test indicates a sequencing bug — every await on notifications or handle calls should be wrapped in `tokio::time::timeout` so failures surface as panics, not CI timeouts.
- If `covgate.sh` fails for `upload` after the ratchet, either add tests for uncovered lines or lower the floor to just below the measured value; both are safe to iterate.
- If `lint.sh` fails on formatting, run `cargo fmt -p miru-agent` from the repo root and re-run. If `cargo machete` flags an unused dependency, no new dependencies should have been added by this plan — investigate rather than suppress.
- The provisional `.covgate` of `0.00` (Milestone 1) means an interrupted implementation between Milestones 1 and 5 leaves the module under-enforced but never blocks other work; Milestone 5 closes the gap.
