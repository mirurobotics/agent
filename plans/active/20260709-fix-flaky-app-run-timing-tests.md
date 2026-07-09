# Fix flaky timing-based integration tests in agent/tests/app/run.rs

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent, this repo) | read-write | Test-only edits to `agent/tests/app/run.rs`. No production source changes. |

This plan lives in `plans/backlog/` because that is this repository's established plan directory convention (see `plans/completed/` for prior plans; lifecycle is `plans/backlog/` → `plans/active/` → `plans/completed/`). The work is confined to this single repository.

## Purpose / Big Picture

The integration tests `max_runtime_reached` and `idle_timeout_reached` in `agent/tests/app/run.rs` intermittently fail when the test suite runs under llvm-cov coverage instrumentation (`scripts/covgate.sh`, which `scripts/preflight.sh` and CI both run) on loaded machines, while passing reliably in isolation. The failures are not caused by the behavior under test — the agent's lifecycle exit paths work correctly — but by wall-clock safety-net timeouts in the tests that are too tight to absorb instrumentation and load overhead. One test (`idle_timeout_reached`) additionally risks the production shutdown watchdog calling `std::process::exit(1)` under slowdown, which would kill the entire integration-test binary, not just the one test.

After this change, a developer (or CI) can run `./scripts/covgate.sh` or `./scripts/preflight.sh` repeatedly on a loaded machine and the `app::run` tests pass every time. Nothing the tests verify is weakened: each test still asserts that `run()` returns `Ok` on its own via a specific lifecycle exit path (or, for `is_persistent`, that it does not return). Only the hang-protection margins and exit-path disambiguation change.

## Progress

- [ ] Edit `agent/tests/app/run.rs`: add shared duration constants with rationale comments.
- [ ] Harden `max_runtime_reached` (widen safety net, pin idle timeout away, pin shutdown watchdog above the safety net).
- [ ] Harden `idle_timeout_reached` (widen safety net, pin max runtime away, raise shutdown watchdog above the safety net).
- [ ] Harden `shutdown_signal_received` (widen safety net, pin shutdown watchdog; comment why the 100 ms startup sleep is not correctness-critical).
- [ ] Widen `invalid_app_state_initialization` safety net for consistency.
- [ ] Add comment to `is_persistent` explaining why it is intentionally left unchanged.
- [ ] Run scoped tests (`app::run::`) and full `./scripts/test.sh` — all pass.
- [ ] Run `./scripts/covgate.sh` at least 3 times — all runs pass tests and coverage gates.
- [ ] Run `./scripts/preflight.sh` — reports `Preflight clean`.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: `LifecycleOptions::default()` (in `agent/src/app/options.rs`) sets `max_shutdown_delay` to 15 s, not just the explicit 5 s in `idle_timeout_reached`. This means `max_runtime_reached` and `shutdown_signal_received` are also exposed to the `std::process::exit(1)` watchdog hazard under extreme slowdown, since their default 15 s watchdog is only 10–15 s above their current 5 s safety nets. The plan pins the watchdog in all three tests, not only in `idle_timeout_reached`.

## Decision Log

- Decision: Fix by test-only hardening (wider safety nets, pinned competing timeouts, pinned shutdown watchdog) rather than refactoring `run()` to return a termination reason or restructuring the lifecycle.
  Rationale: The flakiness lives entirely in test-side wall-clock margins; the production behavior is correct. An API change (e.g. returning an exit-reason enum) would touch production code, callers, and the public surface for no production benefit, and carries regression risk. The exit-path ambiguity concern is fully addressed by pinning the competing timeout to a value that cannot fire within the safety-net window.
  Date/Author: 2026-07-09 / plan author

- Decision: Do not use tokio paused time (`#[tokio::test(start_paused = true)]`) for these tests.
  Rationale: `run()` drives real unix-socket I/O (axum server on `/tmp/miru.sock`), spawned worker tasks (token refresh, poller, MQTT), and possibly real network connection attempts. Under paused time, tokio auto-advances the clock whenever the runtime is idle, which interacts nondeterministically with real I/O readiness — it would trade one class of nondeterminism for another and change what the integration test exercises. Real-time generous margins are the appropriate tool for an integration test of this shape.
  Date/Author: 2026-07-09 / plan author

- Decision: Widen the outer safety-net timeouts to 60 s (constant `HANG_GUARD`) in `max_runtime_reached`, `idle_timeout_reached`, `shutdown_signal_received`, and `invalid_app_state_initialization`.
  Rationale: The outer `tokio::time::timeout` in these tests exists only to prevent a hung `run()` from stalling the suite forever; its exact value is not part of the verified behavior. On success the timeout never elapses, so a generous value costs nothing in suite runtime. 60 s absorbs coverage instrumentation plus heavy machine load with an order-of-magnitude margin over the observed happy path (~100 ms to a few seconds) while still failing fast enough to be usable when something genuinely hangs.
  Date/Author: 2026-07-09 / plan author

- Decision: Pin the competing lifecycle exit path far away (constant `NEVER` = 3600 s): set `idle_timeout: NEVER` in `max_runtime_reached` and `max_runtime: NEVER` in `idle_timeout_reached`.
  Rationale: In non-persistent mode, `run()` races three exits in a `tokio::select!`: shutdown signal, idle timeout, and max runtime. Today `max_runtime_reached` relies on the default 60 s idle timeout not firing before its 100 ms max runtime, and `idle_timeout_reached` relies on the default 15 min max runtime not firing before its ~100 ms idle timeout. Pinning the competing path to one hour makes it impossible for the "wrong" path to fire within the 60 s safety-net window, so each test unambiguously verifies its intended exit path.
  Date/Author: 2026-07-09 / plan author

- Decision: Pin `max_shutdown_delay` (constant `SHUTDOWN_WATCHDOG`, 300 s) strictly above `HANG_GUARD` in the three tests that reach a full shutdown (`max_runtime_reached`, `idle_timeout_reached`, `shutdown_signal_received`).
  Rationale: `ShutdownManager::shutdown` in `agent/src/app/run.rs` calls `std::process::exit(1)` when `shutdown_impl()` exceeds `max_shutdown_delay`. If that watchdog fires inside a test, it kills the whole integration-test binary (all tests, not just the offending one) and produces a confusing abrupt failure. With the watchdog pinned above the hang guard, a genuinely hung shutdown surfaces as a clean per-test `Elapsed` panic from the outer timeout instead. The watchdog behavior itself is production code and is not the subject of these tests; it cannot be unit-covered in-process anyway because `std::process::exit` terminates the test runner.
  Date/Author: 2026-07-09 / plan author

- Decision: Leave `is_persistent` functionally unchanged; add an explanatory comment only.
  Rationale: Its assertion is negative — it expects the outer `timeout(2 * max_runtime, …)` to elapse (`unwrap_err()`), proving that persistent mode ignores `max_runtime`. Slowdown can only delay `run()` further, making the expected outcome (timeout elapses) more likely, never less. It is therefore not flaky under load, and tightening or widening its window would change what it demonstrates.
  Date/Author: 2026-07-09 / plan author

- Decision: Keep the 100 ms startup sleep in `shutdown_signal_received`; document rather than remove it.
  Rationale: The shutdown signal travels over a `tokio::sync::oneshot` channel, which buffers the value: if `tx.send(())` happens while `run()` is still initializing, the `rx.await` inside `run()` completes immediately once it is polled after init. The sleep is a nicety (it usually lets the server reach its steady-state select before the signal arrives) but is not load-bearing for correctness, so slowdown past 100 ms cannot fail the test. Removing it would slightly change the scenario exercised (signal-during-init vs. signal-during-steady-state) for no reliability gain.
  Date/Author: 2026-07-09 / plan author

- Decision: Also widen `invalid_app_state_initialization`'s 5 s safety net to `HANG_GUARD`, even though it was not reported flaky.
  Rationale: Same failure class (outer wall-clock net around `run()` under coverage instrumentation), same file, zero-risk one-line change; leaving one tight 5 s net in the file invites the identical flake later.
  Date/Author: 2026-07-09 / plan author

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

This repository is the Miru agent, a Rust workspace whose main crate is `miru-agent` (sources in `agent/src/`, integration tests in `agent/tests/`). All commands below run from the repo root (`/home/user/agent` in the current checkout; equivalently, wherever the repo is cloned).

Key terms and files:

- **`run()`** — `agent/src/app/run.rs`, `pub async fn run(options: AppOptions, shutdown_signal: impl Future…) -> Result<(), ServerErr>`. This is the agent's main lifecycle: it initializes app state (`AppState::init` — disk layout, caches), spawns a token-refresh worker, an axum HTTP server on a unix socket, a poller worker, and an MQTT worker, then waits for an exit condition, then runs a graceful shutdown. In **non-persistent** mode (`LifecycleOptions.is_persistent == false`) it races three exits in a `tokio::select!` (lines ~50–67): the caller-supplied `shutdown_signal` future, `await_idle_timeout(...)` (polls an activity tracker every `idle_timeout_poll_interval`, exits when idle longer than `idle_timeout`), and `await_max_runtime(...)` (a plain sleep of `max_runtime`). In **persistent** mode only the `shutdown_signal` exit exists.
- **`LifecycleOptions`** — `agent/src/app/options.rs`. Defaults: `is_persistent: true`, `max_runtime: 15 min`, `idle_timeout: 60 s`, `idle_timeout_poll_interval: 5 s`, `max_shutdown_delay: 15 s`.
- **`ShutdownManager::shutdown`** — `agent/src/app/run.rs` (~line 399). Wraps the orderly teardown (`shutdown_impl`: join token-refresh, poller, MQTT, socket server, then app state) in `tokio::time::timeout(max_shutdown_delay, …)`. On timeout it logs and calls `std::process::exit(1)` — in a test context this kills the entire test binary.
- **The tests** — `agent/tests/app/run.rs`, module path `app::run` inside the single integration-test target named `mod` (from `agent/tests/mod.rs`; cargo auto-discovers it, so `--test mod` selects it). Five tests: `invalid_app_state_initialization`, `max_runtime_reached`, `is_persistent`, `idle_timeout_reached`, `shutdown_signal_received`. All but the first are `#[serial]` (from the `serial_test` crate) because they bind the fixed unix socket `/tmp/miru.sock`.
- **Coverage instrumentation** — `scripts/covgate.sh` runs the whole suite via `cargo llvm-cov --json` and enforces per-module minimum coverage from `.covgate` files (e.g. `agent/src/app/.covgate` requires 90.38). `scripts/preflight.sh` runs lint + covgate + tools lint/tests in parallel and prints `Preflight clean` on success. CI (`.github/workflows/ci.yml`) runs `./scripts/covgate.sh` too. llvm-cov instrumentation plus a loaded machine slows every step of `run()` (init, socket bind, worker spawn, shutdown joins) by an unpredictable factor.

Why the tests flake, concretely:

- `max_runtime_reached` sets `is_persistent: false`, `max_runtime: 100 ms` (other lifecycle fields default) and wraps `run()` in `tokio::time::timeout(Duration::from_secs(5), …)` whose result is `unwrap()`ed. The verified behavior is "non-persistent `run()` returns `Ok` on its own via the max-runtime path; the caller's shutdown signal never fires". The 5 s outer timeout is only a hang guard, but under coverage + load, init + 100 ms runtime + full shutdown can exceed 5 s, so the `unwrap()` panics with `Elapsed` even though the behavior under test succeeded. There are also two latent hazards: the default `idle_timeout` (60 s) is a theoretically competing exit path, and the default `max_shutdown_delay` (15 s) can `std::process::exit(1)` the binary if shutdown crawls.
- `idle_timeout_reached` has the same shape with `idle_timeout: 100 ms`, `idle_timeout_poll_interval: 10 ms`, an explicit `max_shutdown_delay: 5 s`, and a 15 s outer net. Its explicit 5 s watchdog is the nastier hazard: a shutdown slower than 5 s under instrumentation kills the whole test binary via `std::process::exit(1)`. The default `max_runtime` (15 min) is its competing exit path.
- `shutdown_signal_received` (persistent mode) spawns `run()` in a task, sleeps 100 ms, fires a oneshot shutdown signal, then waits for the task with a 5 s timeout — the same too-tight hang guard around init + shutdown.
- `is_persistent` asserts the *absence* of self-termination: `timeout(200 ms, run(...))` must elapse (`unwrap_err()`). Slowdown only reinforces that outcome, so it is not flaky (see Decision Log).

## Plan of Work

All edits are in one file: `agent/tests/app/run.rs`. No production source changes.

1. Near the top of the file (after the imports, before `prepare_valid_server_storage`), add three module-level constants with short rationale comments so future maintainers do not re-tighten them:

       // Outer wall-clock net around run() in each test. Purely hang
       // protection -- its value is NOT part of the verified behavior. It
       // must absorb coverage-instrumented, loaded-machine runs, so keep it
       // generous; on success it never elapses and costs nothing.
       const HANG_GUARD: Duration = Duration::from_secs(60);

       // Pins a competing lifecycle exit path so far away it cannot fire
       // within HANG_GUARD, making each test's intended exit path
       // unambiguous.
       const NEVER: Duration = Duration::from_secs(3600);

       // ShutdownManager::shutdown calls std::process::exit(1) if teardown
       // exceeds max_shutdown_delay, which would kill the whole test binary.
       // Keep it above HANG_GUARD so a hung shutdown fails only the
       // offending test via the outer timeout instead.
       const SHUTDOWN_WATCHDOG: Duration = Duration::from_secs(300);

2. `max_runtime_reached`: in `LifecycleOptions`, add `idle_timeout: NEVER` and `max_shutdown_delay: SHUTDOWN_WATCHDOG` (keep `is_persistent: false`, `max_runtime: Duration::from_millis(100)`, keep `..Default::default()` for the poll interval). Change the outer `tokio::time::timeout(Duration::from_secs(5), …)` to `tokio::time::timeout(HANG_GUARD, …)`. Update the comment above it to say the run self-terminates via max_runtime (~100 ms) and that the outer timeout is only hang protection.

3. `idle_timeout_reached`: in `LifecycleOptions`, add `max_runtime: NEVER` and change `max_shutdown_delay: Duration::from_secs(5)` to `max_shutdown_delay: SHUTDOWN_WATCHDOG` (keep `idle_timeout: 100 ms`, `idle_timeout_poll_interval: 10 ms`). Change the outer `tokio::time::timeout(Duration::from_secs(15), …)` to `tokio::time::timeout(HANG_GUARD, …)`. Update the comment accordingly.

4. `shutdown_signal_received`: in `LifecycleOptions`, add `max_shutdown_delay: SHUTDOWN_WATCHDOG` (keep `is_persistent: true`). Change the final `tokio::time::timeout(Duration::from_secs(5), server_handle)` to `tokio::time::timeout(HANG_GUARD, server_handle)`. Above the existing 100 ms sleep, extend the comment to note the sleep is best-effort only: the oneshot channel buffers the signal, so the test stays correct even if startup takes longer than 100 ms.

5. `invalid_app_state_initialization`: change the outer `tokio::time::timeout(Duration::from_secs(5), …)` to `tokio::time::timeout(HANG_GUARD, …)` (same hang-guard rationale; no lifecycle changes needed since `run()` fails during init before any workers start).

6. `is_persistent`: no functional change. Add a comment above the `timeout(2 * max_runtime, …)` explaining that this test's assertion is negative (the timeout MUST elapse because persistent mode ignores `max_runtime`), so machine slowdown can only reinforce the expected outcome and the short window is intentionally left alone.

Nothing else changes. The imports already include `tokio::time::Duration`. Keep the file's import-group comment style intact (`// standard crates` / `// internal crates` / `// external crates`) so the import linter stays clean.

## Concrete Steps

All commands run from the repo root.

1. Make the edits described in Plan of Work to `agent/tests/app/run.rs`.

2. Run the scoped tests (the wrapper `./scripts/test.sh` hardcodes empty test args, so use cargo directly with the same flags it sets — `--features test` is required or mocks/#[cfg(feature = "test")] helpers are missing):

       RUST_LOG=off cargo test --package miru-agent --features test --test mod app::run::

   Expected transcript tail:

       running 5 tests
       test app::run::invalid_app_state_initialization ... ok
       test app::run::max_runtime_reached ... ok
       test app::run::is_persistent ... ok
       test app::run::idle_timeout_reached ... ok
       test app::run::shutdown_signal_received ... ok
       test result: ok. 5 passed; 0 failed; ... filtered out; ...

   (The `#[serial]` tests run serialized relative to each other; ordering in the output may vary.)

3. Run the full suite the standard way:

       ./scripts/test.sh

   Expect all tests to pass (exit 0).

4. Run the suite under coverage instrumentation — the environment where the flake manifests — at least three times, and expect every run to pass both the tests and the per-module coverage gates:

       ./scripts/covgate.sh
       ./scripts/covgate.sh
       ./scripts/covgate.sh

   Each run should end with per-module lines like `✅ app: 9x.xx% (requires 90.38%)` and the final line `✅ All modules meet minimum coverage requirement`. Each run takes several minutes (instrumented build + full suite). Coverage percentages should be unchanged from before this change (the tests exercise exactly the same production paths; only wall-clock margins moved).

5. Run preflight:

       ./scripts/preflight.sh

   Expect the final line `Preflight clean`. (This runs `scripts/lint.sh`, `scripts/covgate.sh`, and the tools lint/covgate in parallel; the import linter and rustfmt must also pass, which the edits preserve.)

## Validation and Acceptance

Acceptance is behavioral:

- `RUST_LOG=off cargo test --package miru-agent --features test --test mod app::run::` reports 5 passed, 0 failed.
- `./scripts/test.sh` passes.
- `./scripts/covgate.sh` passes on at least 3 consecutive runs — this is the flake reproduction environment (llvm-cov instrumentation); before this change, `max_runtime_reached` / `idle_timeout_reached` could fail there with a panic on the outer timeout's `unwrap()` (an `Elapsed` error) on loaded machines, or in the worst case the whole test binary could die with exit code 1 from the shutdown watchdog.
- **`./scripts/preflight.sh` must report `Preflight clean` before these changes are published (committed to a PR/pushed).** This is the gate.

What the tests still verify (unchanged semantics):

- `max_runtime_reached`: non-persistent `run()` returns `Ok` on its own via the max-runtime exit — the caller's shutdown signal never fires, and the idle-timeout path is now pinned to 1 h so it provably cannot be the exit taken.
- `idle_timeout_reached`: non-persistent `run()` returns `Ok` on its own via the idle-timeout exit — max runtime is now pinned to 1 h so it provably cannot be the exit taken.
- `shutdown_signal_received`: persistent `run()` returns `Ok` after the caller-supplied shutdown signal fires.
- `is_persistent`: persistent `run()` does NOT self-terminate via `max_runtime` (outer timeout must elapse) — untouched.
- `invalid_app_state_initialization`: `run()` returns `Err` when storage is invalid — untouched except the wider hang guard.

## Idempotence and Recovery

Every step is safely repeatable: the edits are plain text changes to one test file, and all test/coverage/preflight commands are read-only with respect to the working tree (they write only to `target/`). If a covgate run is interrupted, just rerun it. If the edits go wrong, recover with `git checkout -- agent/tests/app/run.rs` (or `git diff` to inspect first) and reapply from Plan of Work. Leftover `/tmp/miru.sock` files from interrupted runs are harmless — the server rebinds and the affected tests are `#[serial]`.

There is no migration, no destructive step, and no production behavior change to roll back.
