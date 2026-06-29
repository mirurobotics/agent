# Refactor the upload subsystem into an Uploader object + thin timing worker

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All code and test changes: new `agent/src/upload/` module, relocated sync helper, rewritten thin worker, syncer push wiring, app wiring. |

This plan lives in `agent/plans/backlog/` because every edit is inside the `agent` repository (the Miru Rust device agent). Work is done on branch `feat/uploads-file-discovery` (PR #93, base `main`) in push mode — stay on that branch; do not open a new branch.

All paths below are relative to the `agent` repo root unless stated otherwise. Run every command from the `agent` repo root.

## Purpose / Big Picture

Today (on this branch) `agent/src/workers/uploads.rs` is a single file that crams together four unrelated jobs: (1) it re-derives the active upload-rule set every loop by traversing Deployed deployment → release → upload rules out of the storage caches, (2) it owns per-rule polling cadence, (3) it runs the filesystem glob + stability readiness state machine, and (4) it logs ready files. This does not match the rest of the agent, where the analogous backend-sync responsibility is split into a stateful `Syncer` actor (`agent/src/sync/syncer.rs`) and a thin timing-only `poller` worker (`agent/src/workers/poller.rs`).

After this change the upload subsystem mirrors that split:

- A new stateful `Uploader` actor (`agent/src/upload/`) owns the active rule set, per-file readiness state, and per-rule cadence, and runs the glob + readiness + placeholder log sink.
- The `Syncer` PUSHES the active rule set to the `Uploader` after it applies deployments, so the `Uploader` never touches storage.
- `agent/src/workers/uploads.rs` becomes a thin timing worker that just calls `uploader.scan()` on a base tick and handles shutdown, exactly like `poller`.

The observable behavior is unchanged: when a deployment is deployed whose release references upload rules, files matching each rule's glob that have been size/mtime-stable for the rule's stability window are logged once as `upload candidate ready (M2 placeholder sink)`. A reviewer confirms success by running `scripts/test.sh` (all tests pass, including new `tests/upload/` tests) and `scripts/preflight.sh` (prints `Preflight clean`).

This refactor is structural only. It is explicitly NOT M3+: no digest, no PUT/confirm, no ledger, no delete_policy enforcement, no outbound HTTP, and no pruning of uploaded files.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) M1 — create `agent/src/upload/` Uploader object (move readiness/cadence/sink in; actor + handle + Ext); unit tests; covgate file.
- [ ] M2 — relocate the active-rule traversal into `agent/src/sync/upload_rules.rs`; wire the syncer push (SyncerArgs gains the Uploader handle); move/extend tests.
- [ ] M3 — rewrite `agent/src/workers/uploads.rs` as a thin timing worker; wire `AppState` + `run.rs` (spawn Uploader before Syncer; enable-gate decision); update worker test.
- [ ] M4 — validation: `scripts/test.sh` green, `scripts/covgate.sh` green, `scripts/preflight.sh` prints `Preflight clean`.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

These design decisions were confirmed before authoring and are recorded here so the plan is self-contained. Add implementation-time decisions below them as work proceeds.

- Decision: The `Uploader` (not the worker) owns per-rule cadence.
  Rationale: Cadence is rule state; it belongs with the rule set and readiness state in the actor. The worker becomes pure timing, mirroring `poller`.
  Date/Author: 2026-06-29 / plan author.

- Decision: First-sync seeding — the `Uploader` starts with an empty rule set and has NO storage dependency. It is populated only when the `Syncer` pushes via `update_rules`.
  Rationale: One owner of the traversal (the syncer) avoids two code paths reading storage. The uploader is inert (idle) until the first successful sync, which is acceptable for M2.
  Date/Author: 2026-06-29 / plan author.

- Decision: `storage::UploadRules` stays append-only and remains the syncer's body source. `Release.upload_rule_ids` and its population in `agent/src/sync/deployments.rs::store_expanded_release` are kept exactly as on this branch.
  Rationale: The traversal (Deployed deployment → release.upload_rule_ids → rule bodies) is unchanged; only its call site moves from the worker to the syncer.
  Date/Author: 2026-06-29 / plan author.

- Decision: A single config gate, `enable_uploads_worker`, stays and gates ONLY the thin timing worker. The `Uploader` actor is always spawned as part of `AppState` (like the `Syncer`).
  Rationale: The `Uploader` is inert without a worker ticking `scan()` (nothing is globbed), so always-spawning it is harmless and avoids an `Option<Uploader>` in `SyncerArgs` and avoids a borrow/cycle. Splitting into two gates is unnecessary config surface for M2. See "enable-gate decision" below.
  Date/Author: 2026-06-29 / plan author.

- Decision: Spawn ordering — the `Uploader` is spawned in `AppState::init` (`agent/src/app/state.rs`) BEFORE the `Syncer`, because that is where `SyncerArgs` is constructed. Its handle (`Arc<Uploader>`) is cloned into `SyncerArgs` and stored on `AppState`; its actor `JoinHandle` is joined in the `AppState` shutdown future alongside `syncer_handle`. The thin worker is wired in `run.rs` and its `JoinHandle` tracked in `ShutdownManager` (unchanged slot). No cycle: the syncer holds the uploader handle (one direction); the uploader never references the syncer.
  Rationale: `SyncerArgs` is built in `state.rs`, not `run.rs`, so the uploader must exist before the syncer is spawned. Cloning a cheap `mpsc::Sender`-backed handle has no borrow issues.
  Date/Author: 2026-06-29 / plan author.

- Decision: The worker ticks at a fixed base interval and calls `uploader.scan()` each tick; the uploader internally skips rules whose `next_scan_at` has not elapsed.
  Rationale: Keeps the worker pure timing (mirrors `poller`'s `sleep_fn` loop). Tradeoff: the worker wakes every base tick even when the next rule is far in the future, but the per-tick due-check is cheap (no glob unless a rule is due). A future optimization could let `scan()` return a next-due hint; out of scope for M2.
  Date/Author: 2026-06-29 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent is an actor-based Rust binary (crate `miru-agent`, package dir `agent/`). Modules are listed in `agent/src/lib.rs`. Tests live under `agent/tests/`, mirroring `agent/src/` structure, and are registered in `agent/tests/mod.rs`. Run tests with `scripts/test.sh` (which runs `RUST_LOG=off cargo test --features test`). The `--features test` flag is mandatory — many mocks/helpers are behind `#[cfg(feature = "test")]`.

Key terms:

- "Actor": a struct that owns state and runs a `run(mut self)` loop receiving a `Command` enum over a `tokio::sync::mpsc` channel. A cloneable handle holds only an `mpsc::Sender<Command>` and talks to the actor via `send_command` (one `oneshot` reply per command). See `agent/src/sync/syncer.rs`.
- "Thin worker": a free `async fn run(...)` with an injected `sleep_fn` and a `shutdown_signal` future, structured as `tokio::select! { shutdown | run_impl }`. See `agent/src/workers/poller.rs`.
- "Active upload rule set": the upload rules referenced by the currently-Deployed deployment(s). Resolved by traversal: Deployed deployment → its release (`release_id`) → `release.upload_rule_ids` → rule bodies from the append-only `storage::UploadRules` store, unioned and deduped across all Deployed deployments.
- "Readiness" / `decide_ready`: a pure sync function that globs `rule.source.glob`, stats each file, tracks per-file `(size, mtime, stable_since)` in an `observations` map, and returns files that have been unchanged for at least `rule.source.stability_window_secs`. Each newly-ready file is recorded in `already_reported` and returned only once.

### The pattern to mirror (Syncer / poller), with verified signatures

`agent/src/sync/syncer.rs`:

- `pub struct SyncerArgs<HTTPClientT, TokenManagerT: TokenManagerExt> { pub storage: Arc<storage::Storage>, pub http_client: Arc<HTTPClientT>, pub token_mngr: Arc<TokenManagerT>, pub deploy_opts: apply::DeployOpts, pub backoff: cooldown::Backoff, pub event_hub: events::EventHub }` — construction site is `agent/src/app/state.rs::AppState::init` (lines ~66-82).
- `pub struct SingleThreadSyncer<HTTPClientT> { ... state: State, ... }` — the actor holding state. `pub fn new(args: SyncerArgs<...>) -> Self`.
- `pub enum Command { Shutdown { respond_to }, GetSyncState { respond_to }, #[cfg(feature="test")] SetSyncState { state, respond_to }, SyncIfNotInCooldown { respond_to }, Sync { respond_to }, Subscribe { respond_to } }`.
- `pub struct Worker<HTTPClientT: Send> { syncer: SingleThreadSyncer<HTTPClientT>, receiver: mpsc::Receiver<Command> }` with `pub async fn run(mut self) { while let Some(cmd) = self.receiver.recv().await { match cmd { ... } } }`.
- `pub struct Syncer { sender: mpsc::Sender<Command> }` with `pub fn spawn(buffer_size: usize, args: SyncerArgs<http::Client, authn::TokenManager>) -> Result<(Self, JoinHandle<()>), SyncErr>` and the private `async fn send_command<R>(&self, cmd: impl FnOnce(oneshot::Sender<R>) -> Command) -> Result<R, SyncErr>` helper.
- `pub trait SyncerExt { ... }` (annotated `#[allow(async_fn_in_trait)]`) implemented for `Syncer`.
- A `dispatch!` macro at the top forwards an op result over a `respond_to` oneshot.

`agent/src/workers/poller.rs`:

- `pub struct Options { pub poll_interval_secs: i64 }` + `Default`.
- `pub async fn run<F, Fut, SyncerT: SyncerExt>(options: &Options, syncer: &SyncerT, device_stor: &storage::Device, sleep_fn: F, mut shutdown_signal: Pin<Box<impl Future<Output=()> + Send + 'static>>)` where `F: Fn(Duration) -> Fut, Fut: Future<Output=()> + Send`, structured as `tokio::select! { _ = shutdown_signal.as_mut() => {...} _ = run_impl(...) => {} }`.
- `run_impl` loops, derives a wait, `sleep_fn(wait).await`, then calls the syncer via `SyncerExt`.

### The current cram to dismantle (`agent/src/workers/uploads.rs`)

The file currently contains:

- `pub struct Options { pub min_poll_interval_secs: i64 }` + `Default { 1 }` — a floor / fallback for cadence.
- `pub struct FileObservation { pub size: u64, pub mtime: SystemTime, pub stable_since: DateTime<Utc> }` — per-file stability state.
- `pub struct ReadyFile { pub path: PathBuf, pub modified_at: DateTime<Utc> }`.
- `pub async fn run<SleepF, SleepFut, NowF>(options, deployments: &storage::Deployments, releases: &storage::Releases, upload_rules: &storage::UploadRules, sleep_fn, now_fn, shutdown_signal)` — the `tokio::select!` shell.
- `async fn run_impl(...)` — owns `next_scan_at: HashMap<UploadRuleID, DateTime<Utc>>`, `observations: HashMap<PathBuf, FileObservation>`, `already_reported: HashSet<PathBuf>`; each loop calls `active_upload_rules`, iterates rules, checks `next_scan_at` due, runs `decide_ready` under `tokio::task::spawn_blocking` (moving the two maps in/out), logs ready files, updates `next_scan_at`, prunes stale ids, computes the min-remaining wait, and `sleep_fn(wait).await`.
- `pub async fn active_upload_rules(deployments, releases, upload_rules) -> Vec<UploadRule>` — the storage traversal (union/dedupe, skip+debug missing ids, cache-errors-as-empty).
- `pub fn decide_ready(rule: &UploadRule, observations: &mut HashMap<PathBuf, FileObservation>, already_reported: &mut HashSet<PathBuf>, now: DateTime<Utc>) -> Vec<ReadyFile>` — the pure readiness seam.

Existing tests: `agent/tests/workers/uploads.rs` has a `pure` module (decide_ready tests), a `run_loop` module (worker cadence/idle/cache-error tests), and a nested `run_loop::active_set` module (traversal tests). The mocks `crate::mocks::clock::Clock` and `crate::mocks::error::SleepController` provide an injected clock (`Clock::new(epoch)`, `.advance(secs)`, `.now_fn()`) and a controllable sleep (`SleepController::new()`, `.sleep_fn()`, `.await_sleep()`, `.release()`, `.get_last_attempted_sleep()`).

### The syncer-push point

`agent/src/sync/syncer.rs::SingleThreadSyncer::sync_impl` builds a `deployments::Storage { deployments, cfg_insts, releases, git_commits, upload_rules }` from `self.storage` and calls `deployments::sync(&deployments::SyncArgs { ... }).await`. Inside `agent/src/sync/deployments.rs::sync`, the order is: `pull_deployments` → `pull_content_for_cfg_insts` → `apply_deployments` (the point AFTER which the Deployed set in the local cache is current) → `push_deployments`. The cleanest push point is in `sync_impl`, immediately AFTER `deployments::sync(...)` returns: the local deployment cache already reflects what was applied, and the syncer already holds references to all three stores (`releases`, `deployments`, `upload_rules`) via `sync_storage`.

### Component map: where each current responsibility goes

| Current responsibility in `workers/uploads.rs` | Destination |
|---|---|
| `Options { min_poll_interval_secs }` (cadence floor) | `Uploader` config (`UploaderArgs.min_poll_interval_secs`), set in `app/state.rs`. The worker gets a NEW `Options { tick_interval_secs }`. |
| `FileObservation`, `ReadyFile` structs | `agent/src/upload/uploader.rs` (moved). |
| `decide_ready` pure seam (glob + stability) | `agent/src/upload/uploader.rs` (moved, still `pub` for unit tests). |
| `next_scan_at`, `observations`, `already_reported` in-memory state | `Uploader` actor (`SingleThreadUploader` fields). |
| per-rule due check + `next_scan_at` update + stale-id prune | `Uploader::scan` (actor method). |
| `spawn_blocking` orchestration around `decide_ready` | `Uploader::scan`. |
| placeholder ready-file log sink + dedupe | `Uploader::scan` (dedupe already inside `decide_ready` via `already_reported`). |
| min-remaining wait computation | REMOVED. Worker ticks a fixed base interval; uploader skips not-due rules internally. |
| `active_upload_rules` traversal (union/dedupe, missing-skip, cache-error) | `agent/src/sync/upload_rules.rs` (moved); called by the syncer. |
| `run`/`run_impl` `tokio::select!` timing shell | `agent/src/workers/uploads.rs` rewritten as thin worker calling `uploader.scan()`. |
| holding `&storage::*` references | GONE from the uploader/worker. Storage is read only by the relocated sync helper. |

## Plan of Work

### Milestone 1 — the `Uploader` object

Create the module `agent/src/upload/` with three files, mirroring `agent/src/sync/` layout.

`agent/src/upload/errors.rs`: define `UploadErr` using the repo's error conventions (derive `thiserror::Error` + `impl crate::errors::Error`, aggregate via `crate::impl_error!`). It must carry at least the actor-channel errors reused from `crate::cache::errors` exactly as `sync/errors.rs` does:

    pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
    pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

    #[derive(Debug, thiserror::Error)]
    pub enum UploadErr {
        #[error(transparent)]
        SendActorMessageErr(SendActorMessageErr),
        #[error(transparent)]
        ReceiveActorMessageErr(ReceiveActorMessageErr),
    }
    // + From impls and crate::impl_error!(UploadErr { SendActorMessageErr, ReceiveActorMessageErr });

`agent/src/upload/uploader.rs`: mirror `syncer.rs` structurally.

- Move `FileObservation`, `ReadyFile`, and `decide_ready` here verbatim (keep `decide_ready` and the structs `pub` so unit tests use them as a seam). Keep the `spawn_blocking` requirement note in the doc comment.
- `pub struct UploaderArgs { pub min_poll_interval_secs: i64, pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync> }`. The boxed `now_fn` injects the clock (tests pass a mock clock; production passes a closure wrapping `chrono::Utc::now`). Default `min_poll_interval_secs` is provided by the caller (see Decision Log: set in `state.rs`).
- `pub struct SingleThreadUploader { rules: Vec<UploadRule>, next_scan_at: HashMap<UploadRuleID, DateTime<Utc>>, observations: HashMap<PathBuf, FileObservation>, already_reported: HashSet<PathBuf>, min_poll_interval_secs: i64, now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync> }` with `pub fn new(args: UploaderArgs) -> Self`.
- `fn update_rules(&mut self, rules: Vec<UploadRule>)`: replace `self.rules = rules`; prune `next_scan_at` to retain only present ids (mirrors the current loop's prune). Do NOT prune `observations`/`already_reported` (current code never does; lingering entries are harmless). New rules get no `next_scan_at` entry, so they are due on the next scan.
- `async fn scan(&mut self)`: the body of the current `run_impl` loop minus the wait computation. For each rule in `self.rules`: compute `now = (self.now_fn)()`; if `next_scan_at.get(&rule.id).is_none_or(|next| *next <= now)`, run `decide_ready` under `spawn_blocking` (move `observations`/`already_reported` in and back out exactly as today), log each `ReadyFile` with the existing `upload candidate ready (M2 placeholder sink)` message, then set `next_scan_at[rule.id] = now + max(rule.source.poll_interval_secs, self.min_poll_interval_secs) seconds`. After the loop, prune `next_scan_at` to present ids. Return `Result<(), UploadErr>` (Ok unless a panic in the blocking task, which today is logged and swallowed — keep that behavior and return Ok).
- Actor plumbing mirroring syncer:
  - `pub enum Command { UpdateRules { rules: Vec<UploadRule>, respond_to: oneshot::Sender<Result<(), UploadErr>> }, Scan { respond_to: oneshot::Sender<Result<(), UploadErr>> }, Shutdown { respond_to: oneshot::Sender<Result<(), UploadErr>> }, #[cfg(feature="test")] GetRules { respond_to: oneshot::Sender<Result<Vec<UploadRule>, UploadErr>> } }`. The `#[cfg(feature="test")] GetRules` inspector lets the syncer-push test observe the pushed set.
  - `pub struct Worker { uploader: SingleThreadUploader, receiver: mpsc::Receiver<Command> }` with `pub async fn run(mut self) { while let Some(cmd) = self.receiver.recv().await { match cmd { Shutdown => break (after replying), ... } } }` using a local `dispatch!` macro like syncer's.
  - `pub struct Uploader { sender: mpsc::Sender<Command> }` with `pub fn spawn(buffer_size: usize, args: UploaderArgs) -> Result<(Self, JoinHandle<()>), UploadErr>`, `pub fn new(sender)`, and the private `send_command` helper (identical shape to syncer's, mapping send/recv failures to `UploadErr::SendActorMessageErr` / `ReceiveActorMessageErr`).
  - `#[allow(async_fn_in_trait)] pub trait UploaderExt { async fn update_rules(&self, rules: Vec<UploadRule>) -> Result<(), UploadErr>; async fn scan(&self) -> Result<(), UploadErr>; async fn shutdown(&self) -> Result<(), UploadErr>; }` implemented for `Uploader`. Add a `#[cfg(feature="test")] pub async fn get_rules(&self)` inspector on `Uploader`.

`agent/src/upload/mod.rs`:

    pub mod errors;
    pub mod uploader;

    pub use self::errors::UploadErr;
    pub use self::uploader::{Uploader, UploaderExt};

Register the module in `agent/src/lib.rs` (add `pub mod upload;` in alphabetical position — after `telemetry`/before `version`... actually alphabetical order places `upload` after `telemetry` and before `version`). Add `agent/src/upload/.covgate` (see Validation for the threshold).

### Milestone 2 — relocate the traversal and wire the syncer push

Create `agent/src/sync/upload_rules.rs` and MOVE `active_upload_rules` there verbatim (signature unchanged: `pub async fn active_upload_rules(deployments: &storage::Deployments, releases: &storage::Releases, upload_rules: &storage::UploadRules) -> Vec<UploadRule>`). Add `pub mod upload_rules;` to `agent/src/sync/mod.rs`.

Wire the push in `agent/src/sync/syncer.rs`:

- Add `pub uploader: Arc<crate::upload::Uploader>` to `SyncerArgs` (concrete type — the syncer pushes to a single uploader handle; no new generic needed because we observe via the test inspector rather than a mock). Add `uploader: Arc<crate::upload::Uploader>` to `SingleThreadSyncer` and set it in `new`.
- In `sync_impl`, after `deployments::sync(...).await` returns (regardless of Ok/Err — the local cache reflects what was applied), compute `let rules = crate::sync::upload_rules::active_upload_rules(storage_ref.deployments.as_ref(), storage_ref.releases.as_ref(), storage_ref.upload_rules.as_ref()).await;` then `if let Err(e) = self.uploader.update_rules(rules).await { error!("failed to push upload rules to uploader: {e:?}"); }`. Use `UploaderExt` (import it). Pushing on every sync attempt keeps the uploader converged even if `push_deployments` to the backend failed.

This requires the `Uploader` to exist before the `Syncer` is constructed — handled in M3's `state.rs` wiring.

### Milestone 3 — thin worker + app wiring

Rewrite `agent/src/workers/uploads.rs` to be timing-only, mirroring `poller.rs`:

- `pub struct Options { pub tick_interval_secs: i64 }` + `Default { tick_interval_secs: 1 }` (base tick; see Decision Log).
- `pub async fn run<F, Fut, UploaderT: UploaderExt>(options: &Options, uploader: &UploaderT, sleep_fn: F, mut shutdown_signal: Pin<Box<impl Future<Output=()> + Send + 'static>>)` where `F: Fn(Duration) -> Fut, Fut: Future<Output=()> + Send`, structured as `tokio::select! { _ = shutdown_signal.as_mut() => info!("Uploads worker shutdown complete") _ = run_impl(options, uploader, sleep_fn) => {} }`.
- `run_impl`: `loop { let _ = uploader.scan().await; sleep_fn(Duration::from_secs(options.tick_interval_secs.max(1) as u64)).await; }`. No storage, no clock, no readiness logic.
- Delete `FileObservation`, `ReadyFile`, `decide_ready`, and `active_upload_rules` from this file (they moved in M1/M2).

Wire `AppState` in `agent/src/app/state.rs::AppState::init`:

- Spawn the uploader BEFORE the syncer: `let (uploader, uploader_handle) = crate::upload::Uploader::spawn(64, crate::upload::uploader::UploaderArgs { min_poll_interval_secs: 1, now_fn: Arc::new(chrono::Utc::now) })?; let uploader = Arc::new(uploader);`.
- Pass `uploader: uploader.clone()` into the `SyncerArgs { ... }` literal.
- Add `pub uploader: Arc<crate::upload::Uploader>` to the `AppState` struct and set it in the returned value.
- Add `uploader_handle` to the `handles` vec in the `shutdown_handle` future so the actor is joined on shutdown.
- In `AppState::shutdown`, call `self.syncer.shutdown().await?` first (unchanged), then `self.uploader.shutdown().await` (log on error) — syncer pushes to the uploader, so the uploader is shut down after the syncer.

Wire `agent/src/app/run.rs::init_uploads_worker`:

- Change its body to spawn the thin worker with the uploader handle instead of stores: `let uploader = app_state.uploader.clone();` then `uploads::run(&options, uploader.as_ref(), tokio::time::sleep, Box::pin(async move { let _ = shutdown_rx.recv().await; })).await;`. Keep registering the resulting `JoinHandle` in `ShutdownManager.uploads_worker_handle` (the slot, the `register_handle` call, and the shutdown_impl step 3 are unchanged).
- The `if options.enable_uploads_worker { init_uploads_worker(...) }` gate in `init` is unchanged: it now gates only the thin worker. The uploader actor is always spawned (in `state.rs`).

Note `agent/src/app/options.rs`: `AppOptions.uploads: uploads::Options` stays; its field now means the worker's `tick_interval_secs` rather than `min_poll_interval_secs`. The uploader's cadence floor is set in `state.rs` (mirrors how the syncer's `cooldown::Backoff` is hardcoded there).

### Milestone 4 — validation

Run the full suite and preflight (see Validation and Acceptance).

## Concrete Steps

All commands run from the `agent` repo root. Stay on branch `feat/uploads-file-discovery`.

### M1

1. Create `agent/src/upload/errors.rs`, `agent/src/upload/uploader.rs`, `agent/src/upload/mod.rs` as in Plan of Work. Move `FileObservation`/`ReadyFile`/`decide_ready` out of `workers/uploads.rs` into `uploader.rs`.
2. Add `pub mod upload;` to `agent/src/lib.rs`.
3. Add `agent/src/upload/.covgate` containing the threshold (start at `90.00`; raise to the measured value once tests are in — see Validation).
4. Create tests: `agent/tests/upload/mod.rs` (`pub mod uploader;`) and `agent/tests/upload/uploader.rs`; add `pub mod upload;` to `agent/tests/mod.rs`. Port the `pure` decide_ready tests from `agent/tests/workers/uploads.rs` (glob match/miss/recursive/invalid, stability state machine, dedupe-once, no-match-empty) to exercise the moved `decide_ready`. Add actor tests against the `Uploader` handle using `crate::mocks::clock::Clock` for `now_fn`: (a) `update_rules` replaces the active set (push set A, `get_rules` == A; push set B, `get_rules` == B); (b) `scan` honors per-rule cadence (seed two rules with poll 5 and 30 via temp-fs globs that match files; advance the mock clock and assert a rule is only re-globbed once its interval elapses — observe via newly-ready file logs or by checking a file becomes ready exactly once after its window); (c) readiness state machine end-to-end through `scan` (file becomes ready after stability window); (d) dedupe (a ready file is reported once across repeated `scan`s); (e) empty rule set => `scan` is a no-op (no panic).
5. Build and run the new tests: `cargo test --features test --test mod upload 2>&1 | tail -n 20` (expect the upload tests to pass). If `--test mod` filtering is awkward, run `RUST_LOG=off cargo test --features test upload:: 2>&1 | tail -n 30`.
6. Commit: `git add -A && git commit` with message `refactor(upload): extract Uploader object from uploads worker` (sign-off per repo convention).

### M2

7. Create `agent/src/sync/upload_rules.rs` with the moved `active_upload_rules`; add `pub mod upload_rules;` to `agent/src/sync/mod.rs`; remove `active_upload_rules` from `workers/uploads.rs`.
8. Edit `agent/src/sync/syncer.rs`: add `uploader: Arc<crate::upload::Uploader>` to `SyncerArgs` and `SingleThreadSyncer`; push in `sync_impl` after `deployments::sync(...)`.
9. Move the traversal tests: create `agent/tests/sync/upload_rules.rs` and move the `run_loop::active_set` test cases (resolves_active_set, stale_rule_not_acted_on, no_deployed_is_empty, missing_rule_id_skipped, missing_release_is_empty, union_and_dedupe_across_deployments, and the three cache-error cases) there, updating the import to `miru_agent::sync::upload_rules::active_upload_rules`. Add `pub mod upload_rules;` to `agent/tests/sync/mod.rs`.
10. Add a syncer-push test in `agent/tests/sync/syncer.rs`: construct `SyncerArgs` with a real `Uploader` (spawned in the test), seed Deployed deployment → release → rule bodies in storage, drive a sync, and assert `uploader.get_rules()` equals the expected active set. Cover at least the resolved case and the no-deployed=>empty case at the syncer boundary (the exhaustive traversal cases live in `upload_rules.rs`).
11. Build and run: `RUST_LOG=off cargo test --features test sync:: 2>&1 | tail -n 30` (expect pass).
12. Commit: `refactor(upload): push active rule set from syncer; relocate traversal to sync`.

### M3

13. Rewrite `agent/src/workers/uploads.rs` as the thin timing worker (Options + run + run_impl calling `uploader.scan()`).
14. Edit `agent/src/app/state.rs`: spawn the uploader before the syncer, add `uploader` to `AppState`, inject into `SyncerArgs`, join `uploader_handle` in the shutdown future, shut it down after the syncer in `AppState::shutdown`.
15. Edit `agent/src/app/run.rs::init_uploads_worker` to spawn the thin worker with `app_state.uploader.clone()`.
16. Rewrite `agent/tests/workers/uploads.rs`: replace the storage-driven `run_loop` cadence tests with a thin-worker test. Define a minimal test `UploaderExt` mock that counts `scan()` calls (and is a no-op for `update_rules`/`shutdown`), drive `uploads::run` with a `SleepController`, assert `scan()` is called each tick at `tick_interval_secs` cadence, and assert the worker exits when the shutdown signal fires. Keep `#[serial]` only if a fixed path/global is used (these tests use temp dirs, so it is not needed).
17. Build and run: `RUST_LOG=off cargo test --features test workers::uploads 2>&1 | tail -n 30` and `RUST_LOG=off cargo test --features test app:: 2>&1 | tail -n 20` (expect pass).
18. Commit: `refactor(upload): make uploads worker timing-only; wire Uploader into app`.

### M4

19. Run the full test suite: `./scripts/test.sh` (expect `test result: ok` for all binaries; 0 failed).
20. Measure coverage and set thresholds: `./scripts/covgate.sh`. Read the reported coverage for `agent/src/upload`, `agent/src/sync`, `agent/src/workers`, `agent/src/app`, `agent/src/models`. Set `agent/src/upload/.covgate` to the measured percentage (truncated down to two decimals, e.g. if measured 94.7 set `94.00`+ headroom is unnecessary — set just at/below measured so the gate passes and is meaningful). Confirm `agent/src/models/.covgate` stays `100` and the touched modules (`sync` 93.63, `workers` 83.21, `app` 90.38) still pass at or above their existing thresholds. If a touched module dropped below its threshold, add targeted tests until it passes (do NOT lower an existing threshold).
21. Run preflight: `./scripts/preflight.sh` and confirm the final line is `Preflight clean`.
22. Commit any threshold/coverage follow-ups: `test(upload): set covgate threshold and restore touched-module coverage`.

## Validation and Acceptance

Behavioral acceptance (unchanged from this branch): with a Deployed deployment whose release references an upload rule, a file under the rule's glob that has been size/mtime-stable for `stability_window_secs` is logged exactly once as `upload candidate ready (M2 placeholder sink)`. This is verified by the ported readiness/dedupe tests now running through `Uploader::scan` in `agent/tests/upload/uploader.rs`.

Structural acceptance:

- `agent/src/upload/` exists with `Uploader`, `UploaderExt`, `Uploader::spawn`, and `Command { UpdateRules, Scan, Shutdown }`; the uploader holds no `storage` reference.
- `agent/src/workers/uploads.rs` contains only `Options`, `run`, and `run_impl`; it imports `UploaderExt` and references neither storage nor `decide_ready`.
- `active_upload_rules` lives in `agent/src/sync/upload_rules.rs` and is called from `SingleThreadSyncer::sync_impl`.

Test commands and expected results (run from `agent` repo root):

- `./scripts/test.sh` → all test binaries report `test result: ok` with `0 failed`. The new `tests/upload/uploader.rs` and `tests/sync/upload_rules.rs` tests pass; the syncer-push test in `tests/sync/syncer.rs` passes; the thin-worker test in `tests/workers/uploads.rs` passes. A quick before/after check: the syncer-push test fails to compile/pass before M2 wiring exists and passes after.
- `./scripts/covgate.sh` → exits 0. `agent/src/models/.covgate` remains `100`; `agent/src/upload/.covgate` exists and passes; `sync`, `workers`, `app` pass at/above their existing thresholds.
- `./scripts/preflight.sh` → runs lint + tests + tools lint/tests in parallel and prints `Preflight clean` as its final line. **Do not consider the work done, and do not publish/mark the PR ready, until `scripts/preflight.sh` reports `Preflight clean`.**

## Idempotence and Recovery

- All edits are source/test changes on an existing branch; re-running any `cargo`/`scripts` command is safe and repeatable. There are no migrations, no data files, and no destructive operations.
- Module moves (decide_ready, active_upload_rules) are pure relocations; if a build breaks mid-move, `cargo build --features test` points at the dangling references to fix. Recover by completing the move or `git checkout -- <file>` to revert a single file.
- The covgate thresholds are the only "tuned" values: if `covgate.sh` fails because the new threshold is too high, lower the NEW `agent/src/upload/.covgate` to the measured value (never lower a pre-existing module's threshold; instead add tests).
- Commits are per-milestone, so `git revert` or `git reset --hard <commit>` cleanly rolls back a milestone if needed. The branch is push-mode; push only after `Preflight clean`.
