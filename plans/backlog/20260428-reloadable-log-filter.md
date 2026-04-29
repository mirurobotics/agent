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

(Add entries as you go.)

- Decision: …
  Rationale: …
  Date/Author: …

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

2. Define `LoggingGuard`:

       pub struct LoggingGuard {
           _worker: tracing_appender::non_blocking::WorkerGuard,
           reload_handle: tracing_subscriber::reload::Handle<
               tracing_subscriber::EnvFilter,
               // Inner subscriber type is the fmt Layered stack; we hide it behind a generic
               // bound on Subscriber to keep the public type small.
               tracing::Subscriber + Send + Sync,
           >,
           // record whether RUST_LOG was set at init time so reload_level can preserve precedence
           env_filter_locked: bool,
       }

   Implementation note: the concrete generic parameters on `reload::Handle` are awkward to spell. The pragmatic approach is to build a `Registry`-based stack — `tracing_subscriber::registry().with(reload_layer).with(fmt_layer)` — which gives a fully spellable type, OR to store the handle as `reload::Handle<EnvFilter, Registry>` because the `S` type parameter on `Handle` is the **subscriber the layer is layered onto**, which (for our composition) is `Registry`. Resolve the exact type signature during implementation; if the spelled-out type is ergonomically prohibitive, use a type alias near the struct definition. The handle does NOT need to expose the inner subscriber type to callers — only `LoggingGuard::reload_level` consumes it.

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

4. Add `LoggingGuard::reload_level`:

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

3. After the `settings` is read successfully (current line 129), add:

       if let Err(e) = log_guard.reload_level(settings.log_level.clone()) {
           tracing::warn!("Failed to apply settings.log_level to running logger: {e}");
       }

   `LogLevel` is `Clone`; if reload fails the agent still runs with the early default — this is intentional (logging-config errors are non-fatal at runtime).

4. Confirm `run_provision` continues to compile. Its call becomes `let _guard = logs::init(options)?` style — but `run_provision` returns `Result<.., ProvisionErr>`. Adapt: convert via `let _guard = logs::init(options).map_err(|e| ProvisionErr::from(...))?`, or — simpler — change to `let _guard = logs::init(options).expect("failed to initialize provisioning logger");` since provisioning is an interactive one-shot where panicking on logger setup is acceptable. **Decision to confirm during implementation; default is `expect` with a clear message.** Either path is acceptable — record the choice in the Decision Log.

### Milestone 3 — Tests

Edit `agent/tests/logs/mod.rs`:

1. Update the existing `test_init_stdout` and `test_init_file_only` to handle the new `Result` return:

       let guard = logs::init(options).expect("init should succeed");
       drop(guard);

   Note that because `set_global_default` succeeds at most once per process, only the *first* test of these two that runs in a given test binary will actually install. Both tests still exercise the construction and worker-guard lifecycle. Add `#[serial]` (`use serial_test::serial;`) if interleaving with other init tests becomes flaky.

2. Add `test_reload_level_changes_filter`:

   This test does NOT use `logs::init` (which installs a global subscriber). Instead, it builds the same composition `logs::init` builds — `registry().with(reload_layer).with(fmt_layer)` — using a custom in-memory writer, and confirms that `reload_handle.reload(EnvFilter::new("debug"))` causes `tracing::debug!` events to start appearing in the captured buffer.

   Approach (sketch):

       use std::sync::{Arc, Mutex};
       use tracing_subscriber::{fmt, prelude::*, reload, EnvFilter};

       #[derive(Clone, Default)]
       struct CapturingWriter(Arc<Mutex<Vec<u8>>>);

       impl std::io::Write for CapturingWriter { ... }

       // build subscriber with reload layer, capturing writer
       let (filter_layer, handle) = reload::Layer::new(EnvFilter::new("warn"));
       let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
       let writer = ...; // make_writer cloning a CapturingWriter wrapper around buf
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

   Use `set_default` (thread-scoped) rather than `set_global_default` so this test is hermetic against other tests' subscribers. The reload-handle API is the unit under test; the surrounding subscriber wiring mirrors what `logs::init` does.

3. Add `test_reload_level_no_op_when_env_filter_locked`:

   Set `std::env::set_var("RUST_LOG", "warn")` before calling `logs::init` (using a fresh test process is impractical; instead, expose the `env_filter_locked` flag via a `pub fn` on `LoggingGuard` such as `pub fn env_filter_locked(&self) -> bool` and assert it returns `true` when `RUST_LOG` is set). Skip end-to-end emission in this test — the locked-flag check is the contract.

   Note: `RUST_LOG` is set process-wide for the test binary by `scripts/test.sh` to `off`. That means the locked branch is what runs by default. The locked-flag assertion will be true. To confirm the *unlocked* path, also add a unit-level test inside `agent/src/logs/mod.rs` (under `#[cfg(test)] mod tests`) that constructs the same logic by calling a private helper `decide_initial_filter(options: &Options) -> (EnvFilter, bool)` directly, with controlled env (`std::env::remove_var` then `std::env::set_var`). Run such tests with `#[serial]` because env mutation is process-global. Alternatively (simpler), refactor `init` to take an `env_lookup: impl Fn() -> Option<String>` and unit-test the helper without env mutation.

   **Pick the env-injection approach during implementation; record in Decision Log.**

4. Add `test_init_returns_error_on_double_install`:

   Call `logs::init(Options::default())` twice in the same test (use a fresh tokio runtime test like the existing ones). The second call must return `Err(LogsErr::SetGlobalDefault(_))`. Mark `#[serial]`.

   Caveat: other tests may already have installed a subscriber. To make this test reliable, gate it behind a check that detects installed subscriber state, OR run it as the only test in its own integration test binary. The straightforward path: put it in its own integration test file `agent/tests/logs_init_double.rs` so it runs in a dedicated test binary where it's guaranteed to be the only `init` caller.

### Milestone 4 — Validation and finalize

Run `scripts/preflight.sh`. Address any findings, re-run until output ends with `Preflight clean`. Commit per milestone (see Concrete Steps).

## Concrete Steps

All commands run from the agent repo root: `/home/ben/miru/workbench2/repos/agent` (or wherever the repo is checked out — use `git rev-parse --show-toplevel`).

### Milestone 1 — Refactor `logs::init`

1. Edit `agent/src/logs/mod.rs` per "Plan of Work / Milestone 1": add `LogsErr`, `LoggingGuard`, change `init` to return `Result<LoggingGuard, LogsErr>`, add `reload_level`. Remove all `let _ =` around `set_global_default`.
2. Compile-check just the lib:

       cargo check --package miru-agent --features test

   Expected: clean compile. If type errors on the reload handle's generic parameters appear, introduce a `type ReloadHandle = ...` alias near the struct.

3. Run logs-only tests to catch the immediate breakage in `tests/logs/mod.rs` from changing the return type:

       RUST_LOG=off cargo test --features test --package miru-agent --test mod logs::

   (existing tests will fail to compile until step 4 of Milestone 3 is done; that is fine for now — proceed to Milestone 2 and update tests in Milestone 3.)

4. Commit:

       git add agent/src/logs/mod.rs
       git commit -m "feat(logs): add reloadable filter handle and propagate init errors"

### Milestone 2 — Wire main.rs

1. Edit `agent/src/main.rs::run_agent`: keep the early `logs::init(Options::default())`, handle the new `Result`, delete the second init block, add `log_guard.reload_level(settings.log_level.clone())` after settings read. Adjust `run_provision` to handle the new `Result` (use `expect` with a clear message; record decision).
2. Compile:

       cargo check --package miru-agent --features test

3. Commit:

       git add agent/src/main.rs
       git commit -m "refactor(main): init logs early and reload level after settings load"

### Milestone 3 — Tests

1. Update `agent/tests/logs/mod.rs` to match the new `Result` return, add `test_reload_level_changes_filter`, add `test_reload_level_no_op_when_env_filter_locked` (or its decided alternative), add unit tests for the env-filter helper if that path is chosen.
2. Add `agent/tests/logs_init_double.rs` if needed for the double-install test.
3. Run:

       ./scripts/test.sh

   Expected final line includes `test result: ok.` for the agent test target.

4. Commit:

       git add agent/tests/logs/ agent/tests/logs_init_double.rs agent/src/logs/mod.rs
       git commit -m "test(logs): cover reload_level filter change and double-install error"

   (Include `agent/src/logs/mod.rs` in this commit only if private helpers like `decide_initial_filter` were added for testability.)

### Milestone 4 — Preflight and validation

1. Refresh deps if Cargo.lock changed:

       ./scripts/update-deps.sh

2. Run preflight:

       ./scripts/preflight.sh

   Expected: process exits 0 and final line is `Preflight clean`. If any of the four sub-jobs fails (lint / tests / tools lint / tools tests), read the corresponding section of the printed output, fix, and re-run. **Do not advance past this gate until preflight is clean.**

3. If `agent/src/logs/.covgate` threshold needs adjustment (because new code is uncovered or because added tests bumped coverage), edit it in the same milestone-4 commit and re-run `./scripts/covgate.sh` to confirm.

4. Final commit (only if preflight produced changes):

       git add -- <files>
       git commit -m "chore(logs): adjust covgate threshold after reload changes"

5. Move plan file:

       git mv plans/backlog/20260428-reloadable-log-filter.md plans/active/20260428-reloadable-log-filter.md
       # ... or to plans/completed/ at the end

   (The implement skill drives this lifecycle; do not pre-move during authoring.)

## Validation and Acceptance

Acceptance is verified by behaviors, not implementation details. From the agent repo root:

1. **Preflight is clean** before changes are published. Run:

       ./scripts/preflight.sh

   Last line must read exactly `Preflight clean`. This gate is mandatory.

2. **Existing logs tests still pass:**

       ./scripts/test.sh 2>&1 | tail -40

   Expected: all tests under the `logs` integration test target pass; no regressions in the rest of the suite.

3. **Reload behavior is observable in the test added in Milestone 3:** The test `test_reload_level_changes_filter` fails on `main` (function does not exist) and passes after this change. Confirm by reverting just that test in a scratch worktree and re-running — it should not compile (because `LoggingGuard::reload_level` does not yet exist on `main`).

4. **Manual smoke (optional but recommended):** Set `RUST_LOG=info,miru_agent=debug` in a shell, run the agent binary against a test layout up to settings-load, observe that pre-settings reconciliation logs at `debug` already appear (because env filter beats the default `Info`), and that they continue at `debug` after settings parse even if `settings.log_level = "warn"`. Then unset `RUST_LOG`, re-run, and observe that `settings.log_level = "warn"` is honored after reload (only `warn`+ events appear after the settings-read line).

5. **Error path:** Calling `logs::init` twice in the same process produces a real `Err`. Verified by `test_init_returns_error_on_double_install`.

6. **`RUST_LOG` precedence is preserved:** Verified by `test_reload_level_no_op_when_env_filter_locked` (or the env-helper unit test variant chosen during implementation).

## Idempotence and Recovery

- Editing `agent/src/logs/mod.rs` and `agent/src/main.rs` is fully idempotent — re-running the edits produces identical results. If a step compiles partially, fix the offending lines and re-run `cargo check` before moving on.
- Test additions are append-only; if a test is flaky in CI, mark it `#[serial]` and re-run. None of the changes touch on-disk state, network, or external systems, so there is no rollback hazard beyond a normal `git revert`.
- The double-install test mutates global subscriber state. Isolating it into its own integration test binary (`agent/tests/logs_init_double.rs`) is the rollback for any cross-test contamination it might cause; revert that file and rerun the suite if interactions surface.
- If covgate threshold is bumped down (loosened) in Milestone 4, that is a non-destructive numeric change in `agent/src/logs/.covgate`; revert with `git checkout -- agent/src/logs/.covgate`. Prefer adding tests over loosening the threshold whenever practical.
- All commits are atomic per milestone; `git revert <sha>` of any single milestone leaves the tree in a working state (Milestone 1's revert undoes the lib change but `main.rs` would no longer compile — in that recovery case revert M2 and M3 too, in reverse order).
