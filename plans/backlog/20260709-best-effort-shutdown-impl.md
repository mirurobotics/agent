# Make ShutdownManager::shutdown_impl best-effort so a panicked worker cannot skip app-state shutdown

This ExecPlan is a living document. Revise it as work proceeds and record decisions so the work can always be restarted from this file alone.

## Scope

| Repo | Checkout | Access | Files |
|------|----------|--------|-------|
| mirurobotics/agent | /home/user/agent | write | agent/src/app/run.rs (including its inline `mod tests`) |
| mirurobotics/agent | /home/user/agent | read | agent/src/server/errors.rs, agent/src/app/state.rs, agent/tests/app/state.rs, scripts/ |

No other files change. Work happens on the already-checked-out branch `claude/agent-bug-hunt-hujana-shutdown-joins`. Do NOT create branches and do NOT push — commit locally only.

## Purpose / Big Picture

The Miru agent (a Rust binary running on customer devices) shuts down through `ShutdownManager::shutdown_impl` in `agent/src/app/run.rs` (lines 420-485). It joins four background tasks in order — token refresh worker, poller, MQTT worker, socket server — and then, as step 5, shuts down the app state (`app_state.state.shutdown().await` sets the device offline at the backend and flushes stores/events, then `app_state.state_handle.await` waits for storage actors to drain).

Each worker-join step uses `handle.await.map_err(...)?`. Awaiting a `tokio::task::JoinHandle` returns `Err(JoinError)` only when the task panicked or was aborted. The `?` makes `shutdown_impl` return immediately on that error, skipping every later step — including step 5. So exactly when a worker has crashed (when cleanup matters most), the device is never marked offline and stores/events are never flushed.

After this change, shutdown is best-effort: every step is attempted in the existing order, each failure is logged with `error!`, and after all steps have been attempted the FIRST error encountered is returned (same error types as today). Observable outcome: a panicked worker no longer prevents app-state shutdown; a caller of `shutdown()` still sees the original join error.

## Progress

- [ ] Milestone 1: rewrite `shutdown_impl` to best-effort; commit
- [ ] Milestone 2: add unit tests in `run.rs` `mod tests`; commit
- [ ] Milestone 3: full validation (`preflight.sh` reports `Preflight clean`); commit any fixes

## Surprises & Discoveries

Add entries as work proceeds.

## Decision Log

Add entries as work proceeds.

## Outcomes & Retrospective

Fill in when the plan is completed.

## Context and Orientation

All paths are relative to the repo root `/home/user/agent`; run all commands from that directory.

- `agent/src/app/run.rs` — the only source file to modify. `ShutdownManager` (lines 315-326) is private to this file. Fields: `app_state: Option<AppStateShutdownParams>`, `socket_server_handle: Option<JoinHandle<Result<(), ServerErr>>>`, and three worker handles of type `Option<JoinHandle<()>>` (poller, mqtt, token refresh). Note the asymmetry: the socket server task itself returns a `Result`, so its join yields `Result<Result<(), ServerErr>, JoinError>` — the current code uses `??` (line 470) to unwrap both layers.
- `shutdown()` (lines 399-418) broadcasts the shutdown signal and wraps `shutdown_impl()` in a timeout; it is NOT changed by this plan.
- `shutdown_impl` (lines 420-485) opens with an ordering-invariant comment ("the shutdown order is important here. The refresh and server threads rely on the state so the state must be shutdown last."). That comment and the step order (1 refresh, 2 poller, 3 mqtt, 4 server, 5 app state) MUST be preserved. Each step does `if let Some(h) = self.<field>.take() { ... } else { info!("... not found, skipping ...") }` — the `.take()` semantics (second call finds `None` and just logs) must also be preserved.
- Errors: `ServerErr` and `JoinHandleErr` live in `agent/src/server/errors.rs` (imported via `use crate::server::{self, errors::*, ...}` at run.rs line 15). There is no `From<JoinError>` impl; keep the existing manual mapping `ServerErr::JoinHandleErr(JoinHandleErr { source: Box::new(e), trace: trace!() })`. `trace!()` is `crate::trace` (run.rs line 16).
- Logging: `use tracing::{error, info};` (run.rs line 25). Use bare `error!(...)` for failures, matching e.g. `error!("Failed to start server: {}", e);` at line 42.
- Tests: run.rs already has an inline `#[cfg(test)] mod tests` (lines 488-584) with helpers `new_shutdown_manager()` and `spawn_immediate_handle()`. Because `ShutdownManager` is private, all new tests go in this module. Existing tests only cover duplicate registration; none exercise `shutdown_impl`.
- `AppState` (in `agent/src/app/state.rs`) has no trait abstraction, so it cannot be stubbed. A real one can be built in a temp dir; `agent/tests/app/state.rs` lines 249-283 show the template: write a private key file, public key file, and device file into a `Layout`, then `AppState::init(&layout, Capacities::default(), Arc::new(http::Client::new("doesntmatter").unwrap()), fsm::RetryPolicy::default())`. Inside run.rs `mod tests` the same items are reachable as `crate::disk::{Capacities, Layout}`, `crate::filesys::{dirs, files, WriteOptions}`, `crate::deploy::fsm`, `crate::models::Device` (plus `http` already imported by run.rs). `run()` wraps the state handle with `Box::pin(app_state_handle)` (run.rs line 171); tests must do the same before calling `with_app_state`.
- Commands (all from repo root): tests `./scripts/test.sh` (runs `cargo test --package miru-agent --features test`; the feature flag is required), lint `./scripts/lint.sh` (includes `cargo clippy ... -D warnings`), coverage gate `./scripts/covgate.sh` (`agent/src/app/.covgate` requires 90.38% region coverage — new branches must be test-covered), and `./scripts/preflight.sh` which runs everything and prints exactly `Preflight clean` on success.

## Plan of Work

Milestone 1 rewrites `shutdown_impl` to accumulate the first error instead of early-returning, logging every failure. Milestone 2 adds unit tests to the inline `mod tests` proving the two mandatory behaviors: (a) a panicked worker does not prevent step 5 from being attempted, and (b) the first join error is still returned after all steps run. Milestone 3 runs the full validation suite and fixes anything it flags. One commit per milestone.

The target shape for the rewrite (adapt as needed, keep it compact):

    let mut first_err: Option<ServerErr> = None;

    // 1. refresh
    if let Some(handle) = self.token_refresh_worker_handle.take() {
        if let Err(e) = handle.await {
            error!("Failed to shutdown token refresh worker: {}", e);
            first_err.get_or_insert(ServerErr::JoinHandleErr(JoinHandleErr {
                source: Box::new(e),
                trace: trace!(),
            }));
        }
    } else {
        info!("Token refresh worker handle not found, skipping ...");
    }
    // steps 2 (poller) and 3 (mqtt) are identical in shape

    // 4. server — two error layers
    if let Some(handle) = self.socket_server_handle.take() {
        match handle.await {
            Err(e) => { /* log + fold JoinHandleErr as above */ }
            Ok(Err(e)) => {
                error!("Failed to shutdown socket server: {}", e);
                first_err.get_or_insert(e);
            }
            Ok(Ok(())) => {}
        }
    } else { /* existing info! skip message */ }

    // 5. app state — still await state_handle even if state.shutdown() errored
    if let Some(app_state) = self.app_state.take() {
        if let Err(e) = app_state.state.shutdown().await {
            error!("Failed to shutdown app state: {}", e);
            first_err.get_or_insert(e);
        }
        app_state.state_handle.await;
    } else { /* existing info! skip message */ }

    match first_err {
        Some(e) => Err(e),
        None => {
            info!("Program shutdown complete");
            Ok(())
        }
    }

Keep the opening ordering comment, all five `info!` skip messages, and `.take()` on every slot. Do not touch `shutdown()`, the builder methods, or anything else in the file outside `shutdown_impl` and `mod tests`.

## Concrete Steps

Milestone 1 — best-effort `shutdown_impl`

1. In `/home/user/agent`, confirm you are on branch `claude/agent-bug-hunt-hujana-shutdown-joins` with `git status`. Do not create a branch.
2. Edit `agent/src/app/run.rs`: replace the body of `shutdown_impl` (lines 420-485) per the shape in Plan of Work. Preserve the ordering comment, step order, skip messages, `.take()` semantics, and error types (JoinError always mapped to `ServerErr::JoinHandleErr`; the socket server's inner `ServerErr` and `state.shutdown()`'s `ServerErr` folded as-is). Log `info!("Program shutdown complete")` only when no error occurred.
3. Compile: `cargo build --package miru-agent` (from repo root). Fix any errors. Watch for clippy-style unused-result issues — never discard a `Result` silently; every failure is either logged-and-folded or (for `state_handle.await`, which returns `()`) has nothing to discard.
4. Run existing tests: `./scripts/test.sh` — all pass.
5. Commit only run.rs, from `/home/user/agent`:

    git add agent/src/app/run.rs
    git commit -m "fix: attempt all shutdown steps before returning first error"

Milestone 2 — unit tests

1. In the existing `#[cfg(test)] mod tests` at the bottom of `agent/src/app/run.rs`, add tests (all `#[tokio::test]`, no `#[serial]` needed). A panicking handle is `tokio::spawn(async { panic!("boom") })` — awaiting it yields `Err(JoinError)`; the panic backtrace on stderr is expected noise. Add any needed `use crate::...` imports inside the test module, keeping the standard/internal/external grouping convention.
2. Test A (mandatory behavior a) — `shutdown_impl_runs_app_state_shutdown_after_worker_panic`: build a `ShutdownManager` via `new_shutdown_manager()`; register a panicking token refresh handle, healthy poller/mqtt handles (`spawn_immediate_handle()`), and a socket handle `tokio::spawn(async { Ok(()) })`; build a real `AppState` in a temp dir following the template in `agent/tests/app/state.rs` lines 249-283 and register it with `with_app_state(state, Box::pin(state_handle))`. Call `shutdown_impl().await`; assert the result is `Err(ServerErr::JoinHandleErr(_))` and that ALL five slots are `None` afterwards (`assert!(mgr.app_state.is_none())` etc.) — drained slots prove every step, including step 5, was attempted, and `shutdown_impl` returning at all proves `state_handle` resolved (it only completes after app-state shutdown). If constructing a real `AppState` proves infeasible in the unit context, record why in Surprises & Discoveries and fall back to asserting drainage of the poller/mqtt/socket slots plus a separate integration-style test in `agent/tests/app/` exercising step 5 — but attempt the real-AppState route first.
3. Test B (mandatory behavior b) — `shutdown_impl_returns_first_error`: register a panicking poller handle AND a socket handle whose task returns `Err(ServerErr::ShutdownMngrDuplicateArgErr(ShutdownMngrDuplicateArgErr { arg_name: "sentinel".to_string(), trace: trace!() }))`. Assert the returned error is `ServerErr::JoinHandleErr(_)` (the earlier, first error — not the socket's variant) and that the socket slot was still drained. Distinguishing by variant avoids depending on tokio's panic-message formatting.
4. Test C (inner-error branch, keeps covgate happy) — `shutdown_impl_returns_socket_server_inner_error`: register only a socket handle returning `Err(ServerErr::ShutdownMngrDuplicateArgErr(...))`; assert that variant is returned.
5. Test D (happy path of new code) — `shutdown_impl_ok_when_all_steps_succeed`: healthy handles in all four worker/server slots, no app state; assert `Ok(())` and all slots `None`.
6. Note for the record: tests A and B fail against the pre-fix code (the early `?` return leaves later slots populated); tests C and D pass before and after. No need to actually revert to demonstrate this, but it explains what the tests guard.
7. Run `./scripts/test.sh` — all tests pass.
8. Commit from `/home/user/agent`:

    git add agent/src/app/run.rs
    git commit -m "test: cover best-effort shutdown_impl behavior"

Milestone 3 — validation

1. From `/home/user/agent` run, in order: `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/covgate.sh`, then `./scripts/preflight.sh`.
2. Fix anything flagged (fmt, clippy `-D warnings`, coverage below the 90.38 gate for `agent/src/app/` — add a targeted test if a new branch is uncovered). Re-run the failing script after each fix until all pass and preflight prints exactly `Preflight clean`.
3. If (and only if) fixes changed files, commit them from `/home/user/agent` with an appropriate Conventional Commit, e.g.:

    git add agent/src/app/run.rs
    git commit -m "test: cover remaining shutdown_impl branches for covgate"

Do not push. The caller handles publishing.

## Validation and Acceptance

All commands run from `/home/user/agent`.

- `./scripts/test.sh` — the full `miru-agent` test suite passes, including the four new `shutdown_impl` tests.
- `./scripts/preflight.sh` — MUST print `Preflight clean` before the changes are published. Any `Preflight FAILED (...)` output means the work is not done.

Acceptance criteria (behavioral):

1. A panicked worker handle does not prevent app-state shutdown (step 5) from being attempted — proven by Test A: with a panicking token refresh worker, `shutdown_impl` still drains every slot including `app_state`, and its `state_handle` resolves.
2. The first join error is still returned after all steps are attempted — proven by Test B: with a panicking poller (earlier) and a failing socket server (later), the returned error is the poller's `ServerErr::JoinHandleErr`, and the later slots were still processed.
3. Error types are unchanged (`JoinError` → `ServerErr::JoinHandleErr`; inner socket/app-state `ServerErr` values pass through), the shutdown ordering comment and step order are intact, and each failure is logged with `error!`.
4. Tests A and B fail on the pre-fix code and pass after the fix (tests C and D are regression guards that pass in both states).

## Idempotence and Recovery

Every step is safe to re-run. The edit is confined to `shutdown_impl` and `mod tests` in `agent/src/app/run.rs`; if a step goes wrong before its commit, `git diff` shows the current state and `git checkout -- agent/src/app/run.rs` restores the last committed version of the file. `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/covgate.sh`, and `./scripts/preflight.sh` are read-only checks and can be repeated freely. If restarting mid-plan, `git log --oneline -5` shows which milestone commits already exist (`fix: attempt all shutdown steps...`, `test: cover best-effort shutdown_impl...`); resume at the first milestone whose commit is missing. Never create branches, never push, never amend published history — stay on `claude/agent-bug-hunt-hujana-shutdown-joins`.
