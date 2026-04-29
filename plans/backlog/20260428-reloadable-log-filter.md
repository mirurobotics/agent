# Reloadable log filter for early-then-settings logging

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Modify `agent/src/logs/mod.rs` (new guard + reload handle, error type, init returns `Result`), `agent/src/main.rs` (init early, reload after settings load), and `agent/tests/logs/mod.rs` (cover new behavior). |

This plan lives in `agent/plans/backlog/` because all code edits are in the agent repo.

## Purpose / Big Picture

Logs emitted between agent process start and settings-file load currently disappear: today `run_agent` initializes logging twice and the second `set_global_default` silently fails (`let _ =`), so the first install — done with default `Info` level — is the one that sticks, but only after the change in commit `077760d`. Before that commit logs were silently dropped until settings were parsed. We want one durable subscriber installed at process start (so reconciliation, settings-read, and any other early code logs visibly), and then the level updated in place once settings are read.

After this change, an operator running the agent binary sees:

- Reconciliation and pre-settings log lines in the configured destination from the first instant of process startup (currently `Info`-level by default).
- Once settings are loaded, the active filter changes to whatever `settings.log_level` is, **without a second subscriber install** and without losing the file appender / non-blocking writer.
- `RUST_LOG`, when set, continues to override both the early default and the post-settings reload (preserving today's precedence).
- If subscriber installation fails, `run_agent` returns an error rather than silently soldiering on with no logging.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Milestone 1 — Refactor `logs::init` to return a reloadable guard.
- [ ] (YYYY-MM-DD HH:MMZ) Milestone 2 — Wire `main::run_agent` to init early and reload after settings.
- [ ] (YYYY-MM-DD HH:MMZ) Milestone 3 — Add tests for reload behavior and `RUST_LOG` precedence.
- [ ] (YYYY-MM-DD HH:MMZ) Milestone 4 — Preflight clean and finalize.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

- Decision: Use `tracing_subscriber::reload` to wrap the `EnvFilter` layer.
  Rationale: It is the standard mechanism in `tracing-subscriber` for runtime-mutable filtering, already part of the locked workspace dependency, and lets us keep the existing `WorkerGuard`/non-blocking writer alive across a level change. Alternatives (atomic `LevelFilter` swap, rebuilding the entire subscriber) either restrict expressiveness or require re-entering `set_global_default`, which is one-shot per process.
  Date/Author: 2026-04-28, ben@miruml.com.

- Decision: `RUST_LOG` wins over both `options.log_level` at init and `settings.log_level` at reload.
  Rationale: Preserves today's environment-overrides-everything precedence and matches operator expectation. `reload_level` becomes a no-op when the env filter was used at init.
  Date/Author: 2026-04-28, ben@miruml.com.

- Decision: Reload changes the filter/level only — not the writer/destination.
  Rationale: `tracing_subscriber::reload` is per-layer; swapping destinations requires reinstalling the subscriber, which we cannot do once `set_global_default` has succeeded. Destination reload is out of scope.
  Date/Author: 2026-04-28, ben@miruml.com.

- Decision: `run_provision` keeps its single-`init` shape; reload is wired into `run_agent` only.
  Rationale: Provisioning has no settings-file load step that would warrant a level change; complicating it offers no observable benefit.
  Date/Author: 2026-04-28, ben@miruml.com.

(Add further entries as work proceeds.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Key files:

- `agent/src/logs/mod.rs` — defines `LogLevel`, `Options { stdout, log_level, log_dir }`, and `pub fn init(options: Options) -> WorkerGuard`. Uses `tracing_subscriber::fmt` + `EnvFilter` and discards `set_global_default` errors with `let _ =`. Same file for both stdout and file-only paths.
- `agent/src/main.rs` — entry point. `run_agent()` (around line 87) currently calls `logs::init(Options::default())` early, then re-calls `logs::init(...)` after settings load (lines 132–136). The second call's `set_global_default` is silently a no-op because a subscriber is already installed. `run_provision()` (line 43) calls `logs::init` once and is unaffected by this work.
- `agent/src/storage/settings.rs` — `Settings { log_level: LogLevel, .. }`; default is `LogLevel::Info`.
- `agent/tests/logs/mod.rs` — existing tests cover serialize/deserialize and trivially exercise `logs::init` (stdout and file-only) with `drop(guard)`.
- `Cargo.toml` (workspace) — `tracing-subscriber = { version = "0.3.18", features = ["env-filter"] }` (locked at 0.3.23). The `tracing_subscriber::reload` module is part of the crate by default; no new feature is needed.
- `scripts/test.sh` — runs `RUST_LOG=off cargo test --features test --package miru-agent`. Because `RUST_LOG` is forced to `off` for tests, any test that depends on the filter actually emitting events must either (a) build a subscriber locally that does NOT consult `RUST_LOG`, or (b) explicitly clear `RUST_LOG` for the test. We choose (a): the reload test constructs a subscriber via the public reload-handle API in isolation, decoupled from the global subscriber that `logs::init` installs.
- `scripts/preflight.sh` — runs lint + tests + tools lint + tools tests in parallel, prints `Preflight clean` on success.
- `agent/src/logs/.covgate` — current threshold `93.47`. New code must keep the module at or above this; if necessary the threshold can be updated in the same change.
- Error convention: errors derive `thiserror::Error` and implement `crate::errors::Error` (in `agent/src/errors/mod.rs`). For `logs::init` we add a tiny `LogsErr` enum (single variant, install failure) following the same pattern.

Defined terms:

- "Reload handle" — a `tracing_subscriber::reload::Handle<L, S>` returned by wrapping a layer in `reload::Layer::new`. Calling `handle.reload(new_layer)` swaps the inner layer in place; the global subscriber installed by `set_global_default` keeps working.
- "Filter" — an `EnvFilter` (from `tracing_subscriber::EnvFilter`); it decides which events get routed to the formatting layer.
- "Worker guard" — `tracing_appender::non_blocking::WorkerGuard`. Dropping it stops the background log-writer thread and may lose buffered events. The guard must outlive the process logic.
- "Preflight clean" — the literal final line printed by `scripts/preflight.sh` when lint, tests, tools lint, and tools tests all succeed.

## Plan of Work

### Milestone 1 — Reloadable `logs::init`

In `agent/src/logs/mod.rs`:

1. Add a `LogsErr` enum (in the same file or a sibling `errors.rs`; one file is fine here since the surface is small):

       use thiserror::Error;

       #[derive(Debug, Error)]
       pub enum LogsErr {
           #[error("failed to install global tracing subscriber: {0}")]
           SetGlobalDefault(#[from] tracing::subscriber::SetGlobalDefaultError),
       }

   Implementing `crate::errors::Error` for it (with default trait method bodies) keeps it consistent with repo conventions.

2. Define `LoggingGuard`. Anchor the subscriber stack on `Registry` so the reload-handle type is fully spellable; alias it for readability:

       use tracing_subscriber::{registry::Registry, reload, EnvFilter};

       type ReloadHandle = reload::Handle<EnvFilter, Registry>;

       pub struct LoggingGuard {
           _worker: tracing_appender::non_blocking::WorkerGuard,
           reload_handle: ReloadHandle,
           // True if RUST_LOG provided the initial filter; reload_level becomes a no-op.
           env_filter_locked: bool,
       }

   The handle's second type parameter is the subscriber the reload layer is layered onto. With `tracing_subscriber::registry().with(reload_layer).with(fmt_layer)`, that anchor is `Registry`. The fmt layer composes on top and does not appear in the handle's type.

3. Change `init`:

       pub fn init(options: Options) -> Result<LoggingGuard, LogsErr> { ... }

   Steps inside:
   - Build `file_appender` and `(non_blocking, worker_guard)` exactly as today.
   - Decide initial filter:
     - If `RUST_LOG` is set (`EnvFilter::try_from_default_env().is_ok()`), use that filter and remember `env_filter_locked = true`.
     - Otherwise, construct `EnvFilter::new(options.log_level.to_string())` and `env_filter_locked = false`.
   - Wrap the filter in `reload::Layer::new(filter)`, capture `(reload_layer, reload_handle)`.
   - Build the formatting layer with the same options as today (stdout vs non-blocking writer, file/line/thread, ansi off when file-only).
   - Compose `tracing_subscriber::registry().with(reload_layer).with(fmt_layer)` and call `tracing::subscriber::set_global_default(...)`. Propagate errors via `?` (no more `let _ =`).
   - Return `LoggingGuard { _worker: worker_guard, reload_handle, env_filter_locked }`.

4. Update the existing tests in `agent/tests/logs/mod.rs` (`test_init_stdout`, `test_init_file_only`) so the test target keeps compiling at the M1 commit — switch each `let guard = logs::init(options);` to `let guard = logs::init(options).expect("init should succeed");`.

5. Add `LoggingGuard::reload_level`:

       impl LoggingGuard {
           pub fn reload_level(&self, level: LogLevel) -> Result<(), LogsErr> {
               if self.env_filter_locked { return Ok(()); }
               let new_filter = tracing_subscriber::EnvFilter::new(level.to_string());
               self.reload_handle
                   .reload(new_filter)
                   .map_err(|e| LogsErr::ReloadFailed(e.to_string()))?;
               Ok(())
           }
       }

   Add a `ReloadFailed(String)` variant to `LogsErr` for `reload::Error`, since `reload::Error` is non-`Send + 'static`-friendly historically; stringify is fine.

   Doc-comment `reload_level` explicitly: "If `RUST_LOG` was set at process startup, this is a no-op; the env filter wins. This method only adjusts the filter level — it cannot change the log destination (stdout vs file)."

### Milestone 2 — Caller integration in `main.rs`

In `agent/src/main.rs::run_agent`:

1. Replace the single early `let _guard = logs::init(logs::Options::default());` with:

       let log_guard = match logs::init(logs::Options::default()) {
           Ok(g) => g,
           Err(e) => {
               eprintln!("Failed to initialize logging: {e}");
               return;
           }
       };

   `eprintln!` is intentional here because tracing is not yet installed if init fails.

2. Delete the second `logs::init(log_options)` block (current lines 131–136).

3. After `settings` is read successfully (current line 129), add a non-fatal reload call. Reload failure leaves the agent on the early default level — intentional, since logging-config errors should not crash a running agent.

       if let Err(e) = log_guard.reload_level(settings.log_level.clone()) {
           tracing::warn!("Failed to apply settings.log_level to running logger: {e}");
       }

4. In `agent/src/provision/errors.rs`, add a `From<logs::LogsErr>` impl on `ProvisionErr` (mirroring the other `From` impls in that file). Then in `run_provision` change `let _guard = logs::init(options);` to `let _guard = logs::init(options)?;` — propagating the error consistently with the rest of the structured-error codebase.

### Milestone 3 — New tests

Existing tests were updated in M1 step 4. This milestone adds three new tests.

1. Add `test_reload_level_changes_filter` to `agent/tests/logs/mod.rs`. This test does NOT call `logs::init` (which installs a global subscriber). Instead, it rebuilds the same composition (`registry().with(reload_layer).with(fmt_layer)`) with a captured-buffer writer, scopes it via `tracing::subscriber::set_default` (thread-local — hermetic against other tests' globals), then asserts that `handle.reload(EnvFilter::new("debug"))` flips a `debug!` event from filtered-out to emitted. Sketch:

       use std::sync::{Arc, Mutex};
       use tracing_subscriber::{fmt, prelude::*, reload, EnvFilter};

       #[derive(Clone, Default)]
       struct CapturingWriter(Arc<Mutex<Vec<u8>>>);

       impl std::io::Write for CapturingWriter { ... }

       let (filter_layer, handle) = reload::Layer::new(EnvFilter::new("warn"));
       let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
       let writer = /* MakeWriter that clones CapturingWriter(buf.clone()) */;
       let subscriber = tracing_subscriber::registry()
           .with(filter_layer)
           .with(fmt::layer().with_writer(writer));
       let _guard = tracing::subscriber::set_default(subscriber);

       tracing::debug!("before-reload");
       handle.reload(EnvFilter::new("debug")).unwrap();
       tracing::debug!("after-reload");

       let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
       assert!(!captured.contains("before-reload"));
       assert!(captured.contains("after-reload"));

2. Add `test_reload_level_no_op_when_env_filter_locked`. Add a `pub fn env_filter_locked(&self) -> bool` accessor on `LoggingGuard` (in M1 if not already there), call `logs::init(Options::default())`, and assert `guard.env_filter_locked() == true`. `scripts/test.sh` sets `RUST_LOG=off` process-wide, so the locked branch is naturally exercised. The locked-flag check is the contract — emission semantics are covered by the previous test.

3. Add `test_init_returns_error_on_double_install` in a dedicated integration-test binary `agent/tests/logs_init_double.rs` (separate file = separate test binary, so it cannot collide with subscribers installed by other tests). Inside, call `logs::init(Options::default())` twice and assert the second call returns `Err(LogsErr::SetGlobalDefault(_))`. Mark `#[serial]` for safety.

### Milestone 4 — Validation and finalize

Run `scripts/preflight.sh`. Address any findings, re-run until output ends with `Preflight clean`. Commit per milestone (see Concrete Steps).

## Concrete Steps

All commands run from the agent repo root: `/home/ben/miru/workbench2/repos/agent` (or wherever the repo is checked out — use `git rev-parse --show-toplevel`).

### Milestone 1 — Refactor `logs::init`

1. Edit `agent/src/logs/mod.rs` per "Plan of Work / Milestone 1": add `LogsErr`, `LoggingGuard`, change `init` to return `Result<LoggingGuard, LogsErr>`, add `reload_level`. Remove all `let _ =` around `set_global_default`.
2. Update the two existing test calls in `agent/tests/logs/mod.rs` (`test_init_stdout`, `test_init_file_only`) to `.expect(...)` the `Result`, so the test target keeps compiling.

3. Compile-check the package including tests:

       cargo check --package miru-agent --features test --tests

   Expected: clean compile.

4. Commit:

       git add agent/src/logs/mod.rs agent/tests/logs/mod.rs
       git commit -m "feat(logs): add reloadable filter handle and propagate init errors"

### Milestone 2 — Wire main.rs

1. Edit `agent/src/main.rs::run_agent`: handle the new `Result` from the early `logs::init`, delete the second init block, add `log_guard.reload_level(settings.log_level.clone())` after settings read.
2. Edit `agent/src/provision/errors.rs` to add `From<logs::LogsErr> for ProvisionErr`. Edit `agent/src/main.rs::run_provision` to use `?` on the `logs::init` call.
3. Compile:

       cargo check --package miru-agent --features test --tests

4. Commit:

       git add agent/src/main.rs agent/src/provision/errors.rs
       git commit -m "refactor(main): init logs early and reload level after settings load"

### Milestone 3 — New tests

1. Add the three new tests as described in Plan of Work / Milestone 3:
   - `test_reload_level_changes_filter` in `agent/tests/logs/mod.rs`
   - `test_reload_level_no_op_when_env_filter_locked` in `agent/tests/logs/mod.rs` (requires `pub fn env_filter_locked` accessor; add it to `agent/src/logs/mod.rs` if not added in M1)
   - `test_init_returns_error_on_double_install` in a new file `agent/tests/logs_init_double.rs`

2. Run:

       ./scripts/test.sh

   Expected: final line includes `test result: ok.` for the agent test target.

3. Commit:

       git add agent/tests/logs/mod.rs agent/tests/logs_init_double.rs agent/src/logs/mod.rs
       git commit -m "test(logs): cover reload_level filter change and double-install error"

   (Include `agent/src/logs/mod.rs` only if the `env_filter_locked` accessor was added in this milestone.)

### Milestone 4 — Preflight and validation

1. Run preflight:

       ./scripts/preflight.sh

   Expected: exit 0 and final line `Preflight clean`. If any sub-job (lint / tests / tools lint / tools tests) fails, read the printed output, fix, and re-run. **Do not advance past this gate until preflight is clean.**

2. If `agent/src/logs/.covgate` threshold trips, prefer adding a covering test over loosening the number; if the number must move, edit the file and re-run `./scripts/covgate.sh` to confirm. Commit any covgate change separately:

       git add agent/src/logs/.covgate
       git commit -m "chore(logs): adjust covgate threshold after reload changes"

3. Plan-file lifecycle moves are handled by the implement skill — do not pre-move the file during authoring.

## Validation and Acceptance

Acceptance is verified by behaviors, not implementation details. From the agent repo root:

1. **Preflight is clean** before changes are published. Run:

       ./scripts/preflight.sh

   Last line must read exactly `Preflight clean`. This gate is mandatory.

2. **Existing logs tests still pass:**

       ./scripts/test.sh 2>&1 | tail -40

   Expected: all tests under the `logs` integration test target pass; no regressions in the rest of the suite.

3. **Reload behavior is observable:** `test_reload_level_changes_filter` passes after this change. The test exercises `reload::Handle::reload(...)` end-to-end against a captured-buffer writer and asserts the captured output transitions from filtered-out to emitted across the reload call.

4. **Manual smoke (optional but recommended):** Set `RUST_LOG=info,miru_agent=debug` in a shell, run the agent binary against a test layout up to settings-load, observe that pre-settings reconciliation logs at `debug` already appear (because env filter beats the default `Info`), and that they continue at `debug` after settings parse even if `settings.log_level = "warn"`. Then unset `RUST_LOG`, re-run, and observe that `settings.log_level = "warn"` is honored after reload (only `warn`+ events appear after the settings-read line).

5. **Error path:** Calling `logs::init` twice in the same process produces a real `Err`. Verified by `test_init_returns_error_on_double_install`.

6. **`RUST_LOG` precedence is preserved:** Verified by `test_reload_level_no_op_when_env_filter_locked` — under `scripts/test.sh`'s `RUST_LOG=off`, `LoggingGuard::env_filter_locked()` returns `true`, so `reload_level` will short-circuit.

## Idempotence and Recovery

- Editing `agent/src/logs/mod.rs` and `agent/src/main.rs` is fully idempotent — re-running the edits produces identical results. If a step compiles partially, fix the offending lines and re-run `cargo check` before moving on.
- Test additions are append-only; if a test is flaky in CI, mark it `#[serial]` and re-run. None of the changes touch on-disk state, network, or external systems, so there is no rollback hazard beyond a normal `git revert`.
- The double-install test mutates global subscriber state. Isolating it into its own integration test binary (`agent/tests/logs_init_double.rs`) is the rollback for any cross-test contamination it might cause; revert that file and rerun the suite if interactions surface.
- If covgate threshold is bumped down (loosened) in Milestone 4, that is a non-destructive numeric change in `agent/src/logs/.covgate`; revert with `git checkout -- agent/src/logs/.covgate`. Prefer adding tests over loosening the threshold whenever practical.
- Commits are atomic per milestone and each one leaves the tree compiling: M1 includes the trivial test-call updates needed to keep `agent/tests/logs/mod.rs` building. Reverts work in reverse order (M3, M2, M1).
