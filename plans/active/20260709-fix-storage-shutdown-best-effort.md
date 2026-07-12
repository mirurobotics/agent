# Make Storage::shutdown and AppState::shutdown best-effort so one failed store cannot hang shutdown

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds. Revise it as discoveries occur so the work can always be restarted from this file alone.

## Scope

| Repo | Checkout | Access | Files |
|------|----------|--------|-------|
| mirurobotics/agent | /home/user/agent | write | agent/src/disk/mod.rs, agent/src/app/state.rs, agent/tests/disk/caches.rs, agent/tests/app/state.rs |
| mirurobotics/agent | /home/user/agent | read | agent/src/app/run.rs, agent/src/cache/concurrent.rs, agent/src/filesys/cached_file.rs, agent/src/disk/errors.rs, agent/src/server/errors.rs, agent/tests/app/run.rs, scripts/ |

This plan lives in `plans/backlog/` of the agent repo because all code changes are confined to that repository. Work happens on the already-checked-out branch `claude/agent-bug-hunt-vrn2ih-shutdown` (created from `main` at 028354b). Do NOT create or switch branches and do NOT push — commit locally only.

## Purpose / Big Picture

The Miru agent (a Rust binary running on customer devices) persists state through actor-style stores: each store (device file, config-instance metadata, config-instance content, deployments, releases, upload rules, git commits) owns a background tokio task ("worker") that processes commands from an mpsc channel. A worker exits only when it receives a `Shutdown` command or when every command sender is dropped (see `Worker::run` in `agent/src/cache/concurrent.rs` lines 153-161 and in `agent/src/filesys/cached_file.rs` line 154).

`Storage::shutdown` (`agent/src/disk/mod.rs` lines 182-206) shuts these workers down with seven sequential `shutdown().await?` calls, preceded by a device read/patch that marks the device offline. The `?` operator short-circuits on the first failure, so any store after the failing one never receives its `Shutdown` command and its worker blocks in `recv()` forever. `AppState::shutdown` (`agent/src/app/state.rs` lines 107-124) has the same pattern one level up: syncer → event hub → storage → token manager, each with `?` (except the event hub, which is already logged-and-continued).

This matters because of how the app joins those workers. PR #125 (commit bcfa3cb, "fix(app): attempt all shutdown steps before returning first error") made `ShutdownManager::shutdown_impl` in `agent/src/app/run.rs` best-effort: when `state.shutdown()` errors, it logs the error, remembers it, and still unconditionally awaits `app_state.state_handle`. That handle is a `join_all` over ALL worker task handles (`AppState::init` lines 88-92 joins token manager/syncer/event hub handles with `storage_handle`, which itself `join_all`s the seven store worker handles — `agent/src/disk/mod.rs` lines 152-164). Because `app_state.state` (holding every command sender) is still alive during that await, any worker that never got its `Shutdown` command never exits, and `state_handle.await` hangs forever. Net effect since #125: a partial failure inside `AppState::shutdown` or `Storage::shutdown` converts an error into a permanent shutdown hang.

The fix extends #125's best-effort idiom one level down: attempt every shutdown step in the existing order, log each failure, and return the FIRST error only after all steps have been attempted. After this change, a pre-failed store can no longer strand its siblings' workers, so `state_handle.await` always completes and the agent process actually exits (with the first error still reported). Observable outcome: with one store's worker already dead, `Storage::shutdown` returns that store's error AND the storage join handle still completes; same for `AppState::shutdown` and its state handle.

## Progress

- [x] Milestone 1: rewrite `Storage::shutdown` in `agent/src/disk/mod.rs` to best-effort (2026-07-12).
- [x] Milestone 2: rewrite `AppState::shutdown` in `agent/src/app/state.rs` to best-effort (2026-07-12).
- [x] Milestone 3: tests added in `agent/tests/disk/caches.rs` (new `pub mod shutdown`, 3 tests) and `agent/tests/app/state.rs` (existing `pub mod shutdown`, 4 tests + `setup()` helper) (2026-07-12). Deliberate-revert check performed: with `agent/src/disk/mod.rs` temporarily reverted to the pre-fix `?` version (tests kept), `disk::caches::shutdown::attempts_all_stores_after_early_failure` and `returns_first_error_with_multiple_failures` both hang until `HANG_GUARD` (60s) and fail on the timeout `unwrap` — proving the tests guard the fix. Fix restored; all 9 shutdown tests green in <1s.
- [x] Milestone 4 (partial per caller instruction): `./scripts/test.sh` fully green under `HOME=/home/user unshare --user --map-user=1000 --map-group=1000` (284 lib + 1407 integration + 2 log-init tests, 0 failures) (2026-07-12). Remaining: `./scripts/preflight.sh` is deliberately deferred — the caller runs preflight as a separate step. All milestones folded into a single commit per caller instruction (see Decision Log).

Use timestamps when completing steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation (pre-implementation, from #125's retrospective): this work environment runs as root, which breaks two pre-existing permission-based tests in `deploy::filesys` (chmod-based failure injection is bypassed by CAP_DAC_OVERRIDE). These failures exist on `main` and are unrelated to this change — ignore them, or run under `HOME=/home/user unshare --user --map-user=1000 --map-group=1000` to emulate the supported non-root environment.
- Observation (pre-implementation): `agent/tests/disk/caches.rs` already contains `Storage::shutdown` failure tests (`shutdown_twice_returns_error`, `shutdown_with_pre_closed_substore`, `shutdown_with_pre_closed_releases`, `shutdown_with_pre_closed_upload_rules`, lines 63-125). They only assert `unwrap_err()` and never join the workers, so they pass unchanged before and after this fix. Keep them; the new tests strengthen them with join-handle assertions.
- Discovery (2026-07-12): a prior root-mode test run had created `/nonexistent/dir` on the sandbox filesystem (several tests use `/nonexistent/...` as an "unreachable path" for failure injection). With that directory existing, 3 integration tests (`s3::get::dest_unwritable::get_to_missing_parent_dir_maps_to_local_io_err`, both `sync::deployments::apply_error_isolation` tests) failed even under the unshare emulation — and on the clean tree too, so it is pure environmental pollution unrelated to this change. Removing `/nonexistent` restored a fully green unshare run. Note: any root-mode test run re-creates it, so clean it before an unshare validation run.
- Observation (2026-07-12): a raw root-mode `--no-fail-fast` run shows 2 lib + 12 integration failures, all in the same category — chmod/permission failure-injection defeated by root's CAP_DAC_OVERRIDE (plus the `$HOME`-dependent `filesys::dirs::new_home_dir::success`). All pre-existing and environmental; the unshare run is the authoritative green signal.

## Decision Log

(Add entries as you go.)

- Decision: mirror the best-effort idiom established by commit bcfa3cb (#125) — `let mut first_err: Option<Err> = None;` … log each failure with `error!` … `first_err.get_or_insert(...)` … return `first_err` at the end. Read `git show bcfa3cb` before implementing and match its style and naming (`first_err`).
  Rationale: repo precedent; reviewers already accepted this shape.
  Date/Author: 2026-07-09 / plan author.
- Decision: in `Storage::shutdown`, use a small private helper (`fn record<E: Into<StorErr>>(first_err: &mut Option<StorErr>, target: &str, result: Result<(), E>)`) instead of seven copies of the same `if let Err` block.
  Rationale: seven identical four-line blocks would bloat the function past the repo's function-length preference; the helper also unifies the two error types involved (`CacheErr` from the six cache-backed stores, `FileSysErr` from the device store — both have `From` impls into `DiskErr`, see `agent/src/disk/errors.rs` lines 72-88).
  Date/Author: 2026-07-09 / plan author.
- Decision: fold the event hub error in `AppState::shutdown` into the same first-error pattern instead of leaving it swallowed.
  Rationale: uniformity — all four steps get identical treatment. This is a small intentional behavior change: an event-hub-only failure previously produced `Ok(())`; now it surfaces as the returned error. Its only production caller (`ShutdownManager::shutdown_impl` step 5, `agent/src/app/run.rs` ~lines 497-506) already logs-and-folds that error and proceeds, so nothing downstream breaks.
  Date/Author: 2026-07-09 / plan author.
- Decision: place `Storage::shutdown` tests in `agent/tests/disk/caches.rs` (a new `pub mod shutdown`) rather than a new file, and `AppState::shutdown` tests in the existing `pub mod shutdown` of `agent/tests/app/state.rs`.
  Rationale: caches.rs already hosts the existing `Storage::shutdown` tests; state.rs already hosts the `AppState::shutdown` tests. Unlike #125 (whose `ShutdownManager` is private to run.rs, forcing inline `mod tests`), `Storage` and `AppState` are public, so the integration-test tree is the conventional home.
  Date/Author: 2026-07-09 / plan author.
- Decision: assert "the remaining workers actually shut down" by awaiting the join handle returned by `Storage::init` / `AppState::init` under a generous hang-protection timeout, mirroring the `HANG_GUARD` pattern in `agent/tests/app/run.rs` lines 16-20 (60s, value not part of verified behavior).
  Rationale: handle completion is the strongest observable — it is exactly the thing that hangs today. A second `shutdown()` returning an error only proves the channel closed, not that the worker exited.
  Date/Author: 2026-07-09 / plan author.
- Decision: deliver all milestones as ONE commit (`fix(disk): attempt all store shutdowns before returning first error`, covering source + tests + this plan) instead of the per-milestone commits sketched in Concrete Steps, and defer `./scripts/preflight.sh` to the caller.
  Rationale: explicit instruction from the orchestrating session for this execution run; preflight runs as a separate follow-up step.
  Date/Author: 2026-07-12 / implementer.

## Outcomes & Retrospective

Completed 2026-07-12. Both shutdown functions now use the #125 best-effort idiom: `Storage::shutdown` (agent/src/disk/mod.rs) attempts the device read/offline-patch preamble and all seven store shutdowns in the original order via a private `record` helper, logging each failure and returning the first error at the end; `AppState::shutdown` (agent/src/app/state.rs) does the same across syncer → event hub → storage → token manager, and the event hub error is no longer swallowed. Signatures, ordering, and error types are unchanged.

Seven integration tests were added (3 in `agent/tests/disk/caches.rs` `shutdown` mod, 4 in `agent/tests/app/state.rs` `shutdown` mod), each asserting both the returned error variant and that the init join handle completes under a 60s `HANG_GUARD`. The deliberate-revert verification confirmed the tests bite: reverting only `agent/src/disk/mod.rs` to the pre-fix version made both disk hang-guard tests hang for the full 60s and fail on the timeout unwrap; restoring the fix returned them to sub-second green.

Validation: full `./scripts/test.sh` under the unshare non-root emulation is entirely green (284 lib + 1407 integration + 2 log-init tests). Raw root-mode failures are all pre-existing permission-injection environmental issues (see Surprises). `./scripts/preflight.sh` deferred to the caller per instruction.

What went well: the plan's target shapes compiled nearly verbatim; the existing `From` impls made `e.into()` folding seamless. What surprised: sandbox filesystem pollution (`/nonexistent/dir` left by earlier root runs) masqueraded as 3 unrelated test failures until diagnosed against the clean tree.

## Context and Orientation

All paths are relative to the repo root `/home/user/agent`; run all commands from that directory. The crate under test is `miru-agent` (source in `agent/src/`, integration tests in `agent/tests/` mirroring the source module tree). Read `AGENTS.md` at the repo root for conventions (import grouping with `// standard crates` / `// internal crates` / `// external crates` comments, error-handling rules, `scripts/test.sh` usage).

Key pieces:

- `agent/src/disk/mod.rs` — `Storage` (line 80) holds `Arc`s to seven stores: `device` (`Arc<DeviceStorage>`, a `ConcurrentCachedFile` from `agent/src/filesys/cached_file.rs`), `cfg_insts.meta`, `cfg_insts.content`, `deployments`, `releases`, `upload_rules`, `git_commits` (all `cache::FileCache`/`cache::DirCache` aliases over `ConcurrentCache` from `agent/src/cache/concurrent.rs`). `Storage::init` (line 90) spawns one worker task per store and returns `(Storage, impl Future<Output = ()>)` where the future (lines 152-164) is a `join_all` over the seven worker `JoinHandle`s. `Storage::shutdown` (lines 182-206) is the function to rewrite. `StorErr` is an alias for `DiskErr` (line 29). The file currently imports only `tracing::info` (line 34) — `error` must be added.
- Worker exit semantics: `ConcurrentCache::shutdown` (`agent/src/cache/concurrent.rs` line 395) sends `Command::Shutdown` and the worker `break`s out of its `recv()` loop (lines 156-161). Calling `shutdown()` on an already-stopped store fails inside `send_command` with `CacheErr::SendActorMessageErr` (line 381) because the receiver is dropped. The device store behaves the same via `ConcurrentCachedFile` with `FileSysErr::SendActorMessageErr` (`agent/src/filesys/cached_file.rs` line 234); its `read()` and `patch()` fail the same way once the worker is gone. This "shut a store down once, then everything on it errors" behavior is the failure-injection hook the tests use.
- `agent/src/disk/errors.rs` — `DiskErr` variants include `CacheErr(cache::CacheErr)` and `FileSysErr(filesys::FileSysErr)` with `From` impls (lines 72-88), so `e.into()` converts either worker error into `DiskErr`.
- `agent/src/app/state.rs` — `AppState::init` (line 28) builds storage, token manager, event hub, and syncer, and returns `(AppState, impl Future<Output = ()>)` whose future (lines 88-92) joins the token manager/syncer/event hub handles together with the storage handle. `AppState::shutdown` (lines 107-124) is the function to rewrite. All four step errors already convert into `server::ServerErr` via `From` impls (see `agent/src/server/errors.rs` lines 85-123: variants `SyncErr(Box<sync::SyncErr>)`, `EventsErr`, `DiskErr`, `AuthnErr`) — today's `?` relies on exactly those impls, so `e.into()` preserves error types.
- `agent/src/app/run.rs` — `ShutdownManager::shutdown_impl` step "5. app state" (~lines 497-506, post-#125): logs a `state.shutdown()` error, folds it into its own `first_err`, then unconditionally `app_state.state_handle.await`. This is the await that hangs today when `AppState::shutdown`/`Storage::shutdown` bails early. It needs NO changes; read it (and `git show bcfa3cb`) to mirror the idiom.
- Tests and harnesses:
  - `agent/tests/disk/caches.rs` — existing `Storage::shutdown` tests. Harness: `dirs::temp("testing")` → `Layout::new(dir.to_dir())` → `Storage::init(&layout, Capacities::default(), "test_device".to_string())`. Registered in `agent/tests/disk/mod.rs`.
  - `agent/tests/app/state.rs` — existing `AppState::shutdown` tests (`pub mod shutdown`, lines 246-343). Harness: temp dir + write `layout.auth().private_key()`, `layout.auth().public_key()` (any string content), and `layout.device()` (a default `Device` as JSON) via `files::write_string`/`files::write_json`, then `AppState::init(&layout, Capacities::default(), Arc::new(http::Client::new("doesntmatter").unwrap()), fsm::RetryPolicy::default())`. The existing success tests already call `state.shutdown().await.unwrap()` then `state_handle.await` — they prove the handle completes on the happy path.
  - `agent/tests/app/run.rs` lines 16-20 — the `HANG_GUARD` constant pattern (`const HANG_GUARD: Duration = Duration::from_secs(60);` with a comment stating it is hang protection only, generous for coverage-instrumented runs).
  - `miru_agent::sync::SyncerExt` and `miru_agent::authn::TokenManagerExt` are public traits (already imported by other tests, e.g. `agent/tests/sync/syncer.rs`) that must be in scope to call `syncer.shutdown()` / `token_mngr.shutdown()` from tests.
- Commands (all from repo root): tests `./scripts/test.sh` (runs `RUST_LOG=off cargo test --features test` — the `test` feature is REQUIRED; without it mocks/test helpers are missing and failures are misleading), lint `./scripts/lint.sh` (import linter, fmt, machete/diet, audit, clippy `-D warnings`), coverage `./scripts/covgate.sh` (per-module `.covgate` gates: `agent/src/disk/.covgate` = 96.79, `agent/src/app/.covgate` = 90.38), and `./scripts/preflight.sh` which runs all of the above plus tools lint/tests and prints exactly `Preflight clean` on success.
- Environment caveat: 2 pre-existing root-permission test failures in `deploy::filesys` are environmental (sandbox runs as root); ignore them or run under `HOME=/home/user unshare --user --map-user=1000 --map-group=1000` as #125 did.

## Plan of Work

Milestone 1 rewrites `Storage::shutdown` so every store shutdown is attempted; Milestone 2 gives `AppState::shutdown` the same treatment; Milestone 3 adds tests proving that an early failure no longer strands later workers and that the first error is returned; Milestone 4 validates. One commit per milestone. Return types (`Result<(), StorErr>`, `Result<(), server::ServerErr>`), shutdown ORDER, and error types must not change.

Milestone 1 — `agent/src/disk/mod.rs`. Change `use tracing::info;` (line 34) to `use tracing::{error, info};`. Add a private helper above or below `impl Storage` and replace the body of `shutdown` (lines 182-206). Target shape (adapt as needed, keep it compact):

    fn record<E: Into<StorErr>>(
        first_err: &mut Option<StorErr>,
        target: &str,
        result: Result<(), E>,
    ) {
        if let Err(e) = result {
            let e = e.into();
            error!("failed to shutdown {target}: {e}");
            first_err.get_or_insert(e);
        }
    }

    pub async fn shutdown(&self) -> Result<(), StorErr> {
        // best-effort: attempt every step, return the first error at the end
        let mut first_err: Option<StorErr> = None;

        // if the device is online, set it to offline before shutting down
        match self.device.read().await {
            Ok(device_data) => match device_data.status {
                models::DeviceStatus::Online => {
                    info!("Shutting down device storage, setting device to offline");
                    record(
                        &mut first_err,
                        "device offline patch",
                        self.device
                            .patch(models::device::Updates::disconnected())
                            .await,
                    );
                }
                models::DeviceStatus::Offline => {
                    info!("Shutting down device storage, device is already offline");
                }
            },
            Err(e) => {
                error!("failed to read device data during shutdown: {e}");
                first_err.get_or_insert(e.into());
            }
        }

        record(&mut first_err, "device store", self.device.shutdown().await);
        record(
            &mut first_err,
            "config instance metadata store",
            self.cfg_insts.meta.shutdown().await,
        );
        record(
            &mut first_err,
            "config instance content store",
            self.cfg_insts.content.shutdown().await,
        );
        record(&mut first_err, "deployments store", self.deployments.shutdown().await);
        record(&mut first_err, "releases store", self.releases.shutdown().await);
        record(&mut first_err, "upload rules store", self.upload_rules.shutdown().await);
        record(&mut first_err, "git commits store", self.git_commits.shutdown().await);

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

Preserve the two existing `info!` messages and the seven-store order exactly (device, cfg_insts.meta, cfg_insts.content, deployments, releases, upload_rules, git_commits). The device read/patch preamble no longer prevents store shutdowns: on read or patch error, log it, fold it as first error, and continue.

Milestone 2 — `agent/src/app/state.rs`. Replace the body of `shutdown` (lines 107-124) with the same idiom over the four steps, preserving order (syncer first — it uses storage during sync; the existing comments should survive):

    pub async fn shutdown(&self) -> Result<(), server::ServerErr> {
        // best-effort: attempt every step, return the first error at the end
        let mut first_err: Option<server::ServerErr> = None;

        // shutdown the syncer first (it uses storage during sync)
        if let Err(e) = self.syncer.shutdown().await {
            tracing::error!("failed to shutdown syncer: {e}");
            first_err.get_or_insert(e.into());
        }

        // shutdown the event hub
        if let Err(e) = self.event_hub.shutdown().await {
            tracing::error!("failed to shutdown event hub: {e}");
            first_err.get_or_insert(e.into());
        }

        // shutdown storage (sets device offline + shuts down all stores)
        if let Err(e) = self.storage.shutdown().await {
            tracing::error!("failed to shutdown storage: {e}");
            first_err.get_or_insert(e.into());
        }

        // shutdown the token manager
        if let Err(e) = self.token_mngr.shutdown().await {
            tracing::error!("failed to shutdown token manager: {e}");
            first_err.get_or_insert(e.into());
        }

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

The event hub error, previously logged and swallowed, is now folded into `first_err` (see Decision Log). The file uses fully-qualified `tracing::error!` today (line 113); either keep that or add `use tracing::error;` in the external-crates import group — be consistent within the file.

Milestone 3 — tests. Mirror #125's test approach (`git show bcfa3cb` — failure injection on an early step, then assert the later steps still ran and the first error is returned), adapted to public-API integration tests. Failure injection = shut a component down directly first, so the shutdown sequence hits an already-dead worker. "Later steps ran" = the init-returned join handle completes under a hang-protection timeout. Define in each test file (or a shared spot within it):

    const HANG_GUARD: Duration = Duration::from_secs(60);

with the run.rs-style comment that the value is hang protection only, not verified behavior (`use tokio::time::Duration;` in the external-crates group).

In `agent/tests/disk/caches.rs`, add a new `pub mod shutdown` (keep all existing tests; existing `init` mod tests still pass unchanged):

1. `all_workers_exit_after_success` — `let (storage, storage_handle) = Storage::init(...)`; `storage.shutdown().await.unwrap()`; `tokio::time::timeout(HANG_GUARD, storage_handle).await.unwrap()`. Guards the happy path end-to-end (all seven workers exit).
2. `attempts_all_stores_after_early_failure` — pre-close an early store: `storage.cfg_insts.meta.shutdown().await.unwrap()` (second of the seven). Then `let err = storage.shutdown().await.unwrap_err()` and `assert!(matches!(err, DiskErr::CacheErr(_)))`. Then `tokio::time::timeout(HANG_GUARD, storage_handle).await.unwrap()` — this is the assertion that fails (times out) on pre-fix code, because cfg_insts.content, deployments, releases, upload_rules, and git_commits never received `Shutdown`.
3. `returns_first_error_with_multiple_failures` — pre-close `storage.device` AND `storage.deployments`. `Storage::shutdown` now fails first at the device read preamble (a `FileSysErr`) and later at deployments (a `CacheErr`). Assert the returned error is `DiskErr::FileSysErr(_)` (the FIRST error), NOT `DiskErr::CacheErr(_)`, and that `timeout(HANG_GUARD, storage_handle)` completes. Distinguishing by variant avoids depending on error message text, exactly as #125's `shutdown_impl_returns_first_error` did. (Import `miru_agent::disk::DiskErr` for the `matches!` assertions.)

In `agent/tests/app/state.rs`, extend the existing `pub mod shutdown`. Add a small local setup helper to avoid repeating the key/device file boilerplate a seventh time (returns the temp dir guard, `AppState`, and the state handle; keep the temp dir binding alive in each test). Add `use miru_agent::sync::SyncerExt;` and `use miru_agent::authn::TokenManagerExt;` (extend the existing `miru_agent::authn` import) in the internal-crates group. Tests:

4. `attempts_all_steps_after_syncer_failure` — pre-close the syncer: `state.syncer.shutdown().await.unwrap()`. Then `let err = state.shutdown().await.unwrap_err()`; `assert!(matches!(err, ServerErr::SyncErr(_)))`; `tokio::time::timeout(HANG_GUARD, state_handle).await.unwrap()`. On pre-fix code the timeout fires: the early `?` return skips event hub, storage, and token manager shutdown, so their workers (and hence `state_handle`) never finish. This is the direct regression test for the reported bug.
5. `returns_first_error_with_multiple_failures` — pre-close the syncer AND the token manager (`state.token_mngr.shutdown().await.unwrap()`). Assert the returned error is `ServerErr::SyncErr(_)` (first), not `ServerErr::AuthnErr(_)` (last), and the handle completes under `HANG_GUARD`.
6. `surfaces_event_hub_failure` — pre-close the event hub (`state.event_hub.shutdown().await.unwrap()`). Assert `state.shutdown()` returns `ServerErr::EventsErr(_)` and the handle completes. This pins the Decision-Log behavior change (previously this returned `Ok`), and covers the event hub error branch.
7. `continues_past_storage_failure` — pre-close one storage substore (`state.storage.git_commits.shutdown().await.unwrap()`). Assert `state.shutdown()` returns `ServerErr::DiskErr(_)` and the handle completes — proving a storage failure no longer skips the token manager (pre-fix: hang). Covers the storage error branch.

Notes for the tests: no `#[serial]` is needed (each test uses its own `dirs::temp` directory, no fixed paths). Follow the three-group import convention with comments. Avoid 4+ field-by-field `assert_eq!` on one variable (the import linter flags it); these tests only use `matches!` and `unwrap`s. Tests 1, and the existing caches.rs/state.rs tests, pass before and after the fix; tests 2, 4, 5 (timeout assertions) and 7 fail (hang until `HANG_GUARD`, then panic) on pre-fix code; test 6's error assertion fails on pre-fix code (it returned `Ok`). Test 3's variant assertion passes pre-fix but its handle assertion does not.

Coverage: the new branches are all exercised — `record`'s error arm via tests 2/3 (both its `CacheErr` and `FileSysErr` monomorphizations), the preamble `Err` arm via test 3, the preamble `Online` arm via the existing `shutdown_while_online` (caches.rs) and `success_device_online` (state.rs) tests, and all four state.rs error branches via tests 4-7. If `./scripts/covgate.sh` still flags `disk` (gate 96.79) or `app` (gate 90.38), inspect its uncovered-region output and add a targeted test rather than lowering the gate.

## Concrete Steps

All commands run from `/home/user/agent`.

Milestone 1 — best-effort `Storage::shutdown`

1. Confirm the branch: `git status` shows `On branch claude/agent-bug-hunt-vrn2ih-shutdown` and a clean tree (besides this plan file). Do not create or switch branches.
2. Read the idiom source: `git show bcfa3cb -- agent/src/app/run.rs` (skim the `shutdown_impl` rewrite and its tests).
3. Edit `agent/src/disk/mod.rs` per Plan of Work (tracing import, `record` helper, `shutdown` body).
4. Build and test:

    cargo build --package miru-agent
    ./scripts/test.sh

   Expected: build clean; all tests pass except the 2 known environmental `deploy::filesys` root-permission failures (see Context). The existing caches.rs shutdown tests still pass.
5. Commit:

    git add agent/src/disk/mod.rs
    git commit -m "fix(disk): attempt all store shutdowns before returning first error"

Milestone 2 — best-effort `AppState::shutdown`

1. Edit `agent/src/app/state.rs` per Plan of Work.
2. `cargo build --package miru-agent && ./scripts/test.sh` — same expectations as above.
3. Commit:

    git add agent/src/app/state.rs
    git commit -m "fix(app): attempt all app state shutdown steps before returning first error"

Milestone 3 — tests

1. Edit `agent/tests/disk/caches.rs`: add the `HANG_GUARD` const (with comment) and the new `pub mod shutdown` with tests 1-3. Keep the join-handle binding from `Storage::init` instead of discarding it (`let (storage, storage_handle) = ...`).
2. Edit `agent/tests/app/state.rs`: add the setup helper, trait imports, `HANG_GUARD`, and tests 4-7 inside the existing `pub mod shutdown`.
3. Run `./scripts/test.sh` — all new tests pass; expected transcript ends with the usual `test result: ok` lines for the `miru-agent` integration binary (modulo the 2 environmental failures).
4. Optional sanity check that the tests guard the fix: in a scratch worktree (`git worktree add /tmp/claude-0/-home-user/ef99a84c-0588-53b8-bd76-a84d6e698e08/scratchpad/prefix-check <M1-commit>^`), copy the two edited test files in, run the four differentiating tests, and confirm they fail (time out at `HANG_GUARD` / `unwrap_err` panics); then `git worktree remove --force` it. Skip if time-constrained — the Plan of Work reasoning documents the expected pre-fix failures.
5. Commit:

    git add agent/tests/disk/caches.rs agent/tests/app/state.rs
    git commit -m "test: cover best-effort storage and app state shutdown"

Milestone 4 — validation

1. Run, in order: `./scripts/update-deps.sh` (refreshes Cargo.lock if needed for lint), `./scripts/lint.sh`, `./scripts/covgate.sh`, `./scripts/preflight.sh`.
2. Fix anything flagged (fmt, clippy `-D warnings`, import-group ordering, coverage below the `disk` 96.79 / `app` 90.38 gates). Re-run the failing script after each fix until `./scripts/preflight.sh` prints exactly `Preflight clean`.
3. If fixes changed files, commit them with an appropriate Conventional Commit (e.g. `test: cover remaining shutdown branches for covgate` or `style: appease clippy in shutdown paths`).

Do not push. The caller handles publishing.

## Validation and Acceptance

All commands run from `/home/user/agent`.

- `./scripts/test.sh` — the full `miru-agent` suite passes, including the seven new tests (2 pre-existing `deploy::filesys` root-permission failures are environmental and excluded from acceptance; they also fail on `main` in this sandbox).
- `./scripts/preflight.sh` — MUST print `Preflight clean` before these changes are published. Any `Preflight FAILED (...)` output means the work is not done.

Acceptance criteria (behavioral):

1. A dead store early in `Storage::shutdown` no longer strands later store workers: with `cfg_insts.meta` pre-closed, `Storage::shutdown` returns `Err(DiskErr::CacheErr(_))` AND the join future from `Storage::init` completes within the hang guard (test `attempts_all_stores_after_early_failure`; on pre-fix code this times out).
2. The FIRST error wins: with the device store and deployments store both pre-closed, `Storage::shutdown` returns the device read's `DiskErr::FileSysErr(_)`, not the later `DiskErr::CacheErr(_)`, and all workers still exit (test `returns_first_error_with_multiple_failures`).
3. A syncer failure no longer skips event hub/storage/token manager shutdown: with the syncer pre-closed, `AppState::shutdown` returns `Err(ServerErr::SyncErr(_))` AND the state handle completes within the hang guard (test `attempts_all_steps_after_syncer_failure`; on pre-fix code this times out). Same continuation guarantee for a storage failure (test `continues_past_storage_failure`).
4. Order, signatures, and error types are unchanged: syncer → event hub → storage → token manager; device preamble then device, cfg_insts.meta, cfg_insts.content, deployments, releases, upload_rules, git_commits; `Result<(), StorErr>` / `Result<(), server::ServerErr>`; every failure logged with `error!`.
5. All pre-existing shutdown tests (`agent/tests/disk/caches.rs` init mod, `agent/tests/app/state.rs` shutdown mod successes) pass unchanged.

## Idempotence and Recovery

Every step is safe to re-run. Edits are confined to four files; before a milestone's commit, `git diff` shows the current state and `git checkout -- <file>` restores the last committed version. `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/covgate.sh`, and `./scripts/preflight.sh` are read-only checks and can be repeated freely (tests create their own temp dirs and clean up via the `TempDir` guard). If restarting mid-plan, `git log --oneline -5` shows which milestone commits exist (`fix(disk): ...`, `fix(app): ...`, `test: ...`); resume at the first milestone whose commit is missing. A hang-guard test that times out does not leave stray state — the leaked worker tasks die with the test process. Never create branches, never push, never amend published history — stay on `claude/agent-bug-hunt-vrn2ih-shutdown`.
