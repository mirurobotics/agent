# Factor logs::init into pure build_layers + thin install wrapper

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Refactor `agent/src/logs/mod.rs` to expose a pure `build_layers` helper plus a thin `init` wrapper that installs the global subscriber. Move test coverage from the five `agent/tests/logs_init_*.rs` integration-test binaries into the shared integration binary at `agent/tests/logs/mod.rs`, deleting binaries that no longer carry unique coverage. |

This plan lives in `agent/plans/backlog/` because all code edits are in the agent repo. The agent crate root is `agent/agent/` (the workspace crate is named `miru-agent`); the `agent/` segment in path references below is the repo root and the inner `agent/` segment is the crate directory.

## Purpose / Big Picture

`tracing::subscriber::set_global_default` is one-shot per process. Today every test that wants to validate `logs::init`'s layer-construction logic must therefore be its own integration-test binary, because each binary is a fresh process. There are five such binaries (`logs_init_stdout.rs`, `logs_init_file_only.rs`, `logs_init_double.rs`, `logs_init_locked.rs`, `logs_init_reload.rs`). Most of what they assert — the shape of the layer stack, the env-filter precedence, the reload handle's behavior — does not actually require global installation. Splitting `init` into a pure `build_layers(opts)` helper that constructs the layer stack and returns a `WorkerGuard` plus a `reload::Handle`, plus a thin `init` that calls `set_global_default` over the result, lets the bulk of that coverage move into the shared integration binary. After this change:

- Engineers reading `agent/src/logs/mod.rs` see a 3-line `init` whose only side effect is the global install — the rest is a pure function they can unit-test or attach with `tracing::subscriber::with_default` in any test context.
- Test compile time and link time drop because four binaries collapse into the shared one.
- Engineers adding new logging configuration variants no longer have to add another top-level `tests/logs_init_<name>.rs` binary — they extend the matrix in `tests/logs/mod.rs`.
- Public API (`LoggingGuard`, `LogsErr`, `Options`, `LogLevel`), `RUST_LOG` precedence, and `agent/src/main.rs` integration are unchanged.

Operator/user-visible behavior is unchanged. The win is for the codebase: smaller `init`, fewer binaries, easier to extend.

## Progress

- [ ] Milestone 1 — Refactor `agent/src/logs/mod.rs` to expose `build_layers` and a thin `init`.
- [ ] Milestone 2 — Migrate `build_layers` matrix and reload mechanics into `agent/tests/logs/mod.rs`.
- [ ] Milestone 3 — Delete superseded top-level test binaries; keep the minimum needed for global-install coverage.
- [ ] Milestone 4 — Run preflight; confirm `Preflight clean` before publishing.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: `build_layers` is `pub`, not `pub(crate)`.
  Rationale: Cargo's integration-test scope (`agent/tests/`) is a separate crate from the library, so `pub(crate)` would not be reachable from `tests/logs/mod.rs`. The `logs` module is already `pub mod logs;` in `agent/src/lib.rs`; exposing one more pub function is consistent with how the rest of the module surface (`init`, `Options`, `LogLevel`, `LoggingGuard`, `LogsErr`) is already public for the same reason.
  Date/Author: 2026-04-29, ben@miruml.com.

- Decision: `build_layers` returns `(BoxedLayer, WorkerGuard, reload::Handle<EnvFilter, Registry>, bool)`, where `BoxedLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>` (i.e. `tracing_subscriber::Layer::boxed()`-style erased composite) and the trailing `bool` is `env_filter_locked`.
  Rationale: Returning a single composite layer keeps the `init` body trivially `Registry::default().with(layers)`. Boxing erases the divergent stdout/file-appender layer types so the function has a single signature regardless of `options.stdout`. The `bool` plumbs through to `LoggingGuard.env_filter_locked` without exposing internal state. If the implementer prefers, they may instead return `(reload_layer, BoxedLayer /* fmt */, WorkerGuard, ReloadHandle, bool)` and let `init` compose — both compile cleanly; the boxed-composite form is shorter at the call site.
  Date/Author: 2026-04-29, ben@miruml.com.

- Decision: Keep two top-level binaries — `logs_init_smoke.rs` (covers a successful global install + `LogsErr::SetGlobalDefault` on second install, both inside one `#[serial]`-tagged test that runs to completion in the same binary, which is fine because the binary's process-global subscriber state is fresh) and `logs_init_locked.rs` (RUST_LOG-locked branch through real `init`).
  Rationale: The smoke + double-install path can be combined: install once successfully, then call `init` again and assert the error. Both happen in the same process; the second `init` legitimately observes the already-installed subscriber. This keeps `init`'s "actually installs globally" coverage and its "returns the right error variant on collision" coverage in one binary instead of two. The `RUST_LOG`-locked branch must stay separate because the shared integration binary inherits `RUST_LOG=off` from `scripts/test.sh`, so any `init` inside it would also lock and we could not exercise the not-locked branch there. (In tests/logs/mod.rs we use `with_default` on a hand-built subscriber, never `init`, so RUST_LOG is irrelevant.)
  Date/Author: 2026-04-29, ben@miruml.com.

- Decision: Drop `logs_init_reload.rs`, `logs_init_stdout.rs`, `logs_init_file_only.rs` entirely.
  Rationale: Their assertions ("guard drops cleanly", "reload_level happy path") are about layer-construction and reload-handle mechanics, not global installation. They re-test what a `build_layers` matrix in `tests/logs/mod.rs` covers without spawning new processes. The `reload_level` happy path is already exercised in `tests/logs/mod.rs::test_reload_level_changes_filter` against a hand-built subscriber attached via `set_default`; a `build_layers`-based variant adds value but doesn't require its own binary.
  Date/Author: 2026-04-29, ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

You are working in the Miru agent repo. The crate root is `agent/agent/` (the outer `agent/` is the repo, the inner `agent/` is the Cargo crate named `miru-agent`). The library entry point is `agent/agent/src/lib.rs`, which declares `pub mod logs;`.

Key files for this task:

- `agent/agent/src/logs/mod.rs` — the file you are refactoring. Contains `LogLevel`, `Options`, `LogsErr`, `LoggingGuard`, and `init`. The current `init` does five things: (1) builds a non-blocking file appender via `tracing_appender::rolling::hourly` → produces a `WorkerGuard`; (2) resolves `(EnvFilter, env_filter_locked: bool)` from `RUST_LOG` or `options.log_level`; (3) wraps the env filter in `reload::Layer::new` → produces a `reload::Handle<EnvFilter, Registry>`; (4) branches on `options.stdout` to compose either a stdout `fmt::layer` or a file `fmt::layer` (with `with_writer(non_blocking).with_ansi(false)`) on top of `Registry::default().with(reload_layer)`; (5) calls `tracing::subscriber::set_global_default(subscriber)`. It returns `LoggingGuard { _worker: WorkerGuard, reload_handle: ReloadHandle, env_filter_locked: bool }`.
- `agent/agent/src/main.rs` — calls `logs::init(logs::Options::default())` early in `run_agent`, then `log_guard.reload_level(settings.log_level.clone())` after settings load. Also calls `logs::init(options)?` once in `run_provision`. Must continue to work unchanged.
- `agent/agent/tests/mod.rs` — the integration-test binary entry point. It declares `pub mod logs;` (and 22 other module mounts). Cargo treats this single `tests/mod.rs` plus its sibling `tests/<dir>/` directories as one integration-test binary; that's the "shared integration binary" referenced throughout this plan.
- `agent/agent/tests/logs/mod.rs` — the test module mounted by `tests/mod.rs`. Already contains tests for `LogLevel` (de)serialization, ordering, defaults, the `LoggingGuard::reload_level` happy path against a hand-built subscriber attached via `tracing::subscriber::set_default` (`test_reload_level_changes_filter`), and `LogsErr` trait-surface tests (`test_logs_err_reload_failed_display`, `test_logs_err_uses_default_error_trait`). This file is where new `build_layers` matrix tests will land.
- `agent/agent/tests/logs_init_double.rs` — `#[serial]` test that calls `logs::init` twice and asserts the second returns `LogsErr::SetGlobalDefault(_)`. Will be folded into the new combined smoke binary.
- `agent/agent/tests/logs_init_stdout.rs` — calls `logs::init(stdout=true, level=Debug)`, drops the guard. To delete.
- `agent/agent/tests/logs_init_file_only.rs` — calls `logs::init(stdout=false, level=Warn)`, drops the guard. To delete.
- `agent/agent/tests/logs_init_locked.rs` — relies on `scripts/test.sh` setting `RUST_LOG=off`; calls `logs::init`; asserts `env_filter_locked()` returns true and `reload_level` is a no-op. Stays as its own binary.
- `agent/agent/tests/logs_init_reload.rs` — `unsafe { std::env::remove_var("RUST_LOG") }`, then `logs::init`, then exercises `reload_level` happy path. To delete (replaced by `build_layers`-based coverage in `tests/logs/mod.rs`).
- `agent/agent/src/provision/errors.rs` — defines `From<logs::LogsErr> for ProvisionErr`. Read-only for this plan; we are not touching it.
- `agent/scripts/test.sh` — sets `RUST_LOG=off` then runs `cargo test --package miru-agent --features test`. Why it matters: tests inside the shared integration binary inherit `RUST_LOG=off`, so a real `init` call in that binary would always take the env-locked branch.
- `agent/scripts/preflight.sh` — runs lint and tests in parallel; prints `Preflight clean` on success, `Preflight FAILED` otherwise. This plan's "validation" step is `./scripts/preflight.sh` from the repo root.

Definitions (terms used in this plan):

- **Integration-test binary.** A test crate that Cargo compiles from a single top-level file in `tests/`. `tests/foo.rs` and `tests/bar.rs` are two binaries; `tests/foo/mod.rs` mounted from `tests/mod.rs` is part of the same binary as `tests/mod.rs`. Each binary is a fresh process when `cargo test` runs.
- **Global subscriber.** The single `Dispatch` installed by `tracing::subscriber::set_global_default`. One per process; second install fails with `SetGlobalDefaultError`.
- **`tracing::subscriber::with_default(subscriber, || { ... })`.** Installs a subscriber for the duration of the closure on the current thread only — does not touch global state. Lets us exercise emission semantics in a unit-style test without conflicting with any other test.
- **Reload handle.** `tracing_subscriber::reload::Handle<EnvFilter, Registry>`. Returned alongside a `reload::Layer` and lets you swap the inner filter at runtime via `handle.reload(new_filter)`.
- **Worker guard.** `tracing_appender::non_blocking::WorkerGuard`. Returned alongside the non-blocking writer; flushes pending writes when dropped.
- **Env-filter locked.** Internal flag set when `EnvFilter::try_from_default_env()` succeeds (i.e., `RUST_LOG` was set). When locked, `LoggingGuard::reload_level` returns `Ok(())` without touching the handle, preserving env-wins-over-config precedence.

## Plan of Work

The work is four milestones. Each ends with a single commit so the PR is reviewable as discrete units and the history is bisectable.

### Milestone 1 — Refactor `agent/src/logs/mod.rs`

Edit `agent/agent/src/logs/mod.rs`:

1. Add a public type alias for the boxed composite layer near the existing `type ReloadHandle = ...` alias. Suggested:

       pub type BoxedLogLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync + 'static>;

   This keeps the `build_layers` signature readable.

2. Add a public function `build_layers` that contains all of `init`'s logic except `set_global_default`:

       pub fn build_layers(
           options: Options,
       ) -> (BoxedLogLayer, WorkerGuard, ReloadHandle, bool) {
           let file_appender = tracing_appender::rolling::hourly(options.log_dir, "miru.log");
           let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);

           let (env_filter, env_filter_locked) = match EnvFilter::try_from_default_env() {
               Ok(f) => (f, true),
               Err(_) => (EnvFilter::new(options.log_level.to_string()), false),
           };

           let (reload_layer, reload_handle) = reload::Layer::new(env_filter);

           let composite: BoxedLogLayer = if options.stdout {
               let fmt_layer = fmt::layer()
                   .with_file(true)
                   .with_line_number(true)
                   .with_thread_ids(true)
                   .with_thread_names(true);
               reload_layer.and_then(fmt_layer).boxed()
           } else {
               let fmt_layer = fmt::layer()
                   .with_writer(non_blocking)
                   .with_file(true)
                   .with_ansi(false)
                   .with_line_number(true)
                   .with_thread_ids(true)
                   .with_thread_names(true);
               reload_layer.and_then(fmt_layer).boxed()
           };

           (composite, worker_guard, reload_handle, env_filter_locked)
       }

   Note: `Layer::and_then` and `Layer::boxed` both come from `tracing_subscriber::Layer`. If `and_then` is not available in the locked version, the implementer may instead return four values without the composite and have `init` compose: `Registry::default().with(reload_layer).with(fmt_layer)`. In that case the signature becomes `pub fn build_layers(options: Options) -> (impl Layer<Registry> + Send + Sync, WorkerGuard, ReloadHandle, bool)` using `-> impl` if and only if both branches of the `if options.stdout` produce the same erased type — practically you'll need to box, hence the tuple variant above. Pick whichever compiles cleanly with the workspace-locked `tracing-subscriber` version; the public signature is an internal detail callers in `tests/logs/mod.rs` adapt to.

3. Rewrite `init` as a thin wrapper:

       pub fn init(options: Options) -> Result<LoggingGuard, LogsErr> {
           let (layers, worker_guard, reload_handle, env_filter_locked) = build_layers(options);
           let subscriber = Registry::default().with(layers);
           tracing::subscriber::set_global_default(subscriber)?;
           Ok(LoggingGuard {
               _worker: worker_guard,
               reload_handle,
               env_filter_locked,
           })
       }

4. Run `cargo build --package miru-agent --features test` from `agent/` to confirm the crate still compiles. Run the existing test binaries (`./scripts/test.sh`) to confirm no regression before tests are reorganized.

5. Commit M1.

### Milestone 2 — Migrate coverage into `agent/tests/logs/mod.rs`

Edit `agent/agent/tests/logs/mod.rs`:

1. Add the `build_layers` matrix tests. These call `build_layers` directly and validate the returned tuple's invariants without ever calling `set_global_default`, so they can live in the shared integration binary. Add a new section header comment `// ========================= build_layers ========================= //` after the existing `// ========================= variants =========================== //` block.

2. The matrix should cover:

   - `test_build_layers_stdout_debug` — `Options { stdout: true, log_level: LogLevel::Debug, log_dir: <tempdir> }`. Assert that the reload handle accepts a reload (`handle.reload(EnvFilter::new("warn")).is_ok()`), and that the worker guard is alive (it's `!Sized`, but we can attach the layer via `tracing::subscriber::with_default(Registry::default().with(layer), || tracing::warn!("hello"))` and assert it didn't panic). The point is to prove the tuple is well-formed; emission assertions are covered by the dedicated `with_default` test below.
   - `test_build_layers_file_only_warn` — `Options { stdout: false, log_level: LogLevel::Warn, log_dir: <tempdir> }`. Same shape.
   - `test_build_layers_respects_rust_log_when_set` — set `RUST_LOG=info` for the duration of the test (`temp_env` style — but `temp_env` is not a workspace dep, so use `unsafe { std::env::set_var(...) }` and remove it after; or scope with a guard struct). Call `build_layers`, assert the trailing bool is `true`. This covers the env-locked branch without touching globals. Note the unsafety caveat: this test must be `#[serial]` because env vars are process-wide. Mark it with `#[serial_test::serial(rust_log)]` so it serializes against any other test that touches `RUST_LOG`.
   - `test_build_layers_uses_options_when_rust_log_unset` — clear `RUST_LOG`, call `build_layers`, assert the trailing bool is `false`. Also `#[serial(rust_log)]`. Caveat: `scripts/test.sh` exports `RUST_LOG=off`, so the test must affirmatively clear it before calling `build_layers` and restore it after. Use a small RAII guard or explicit `unsafe { std::env::set_var("RUST_LOG", "off") }` in a finalizer.
   - `test_build_layers_reload_handle_changes_filter` — call `build_layers`, attach the returned composite via `tracing::subscriber::with_default(Registry::default().with(layer), || { ... })`, and inside the closure use the returned reload handle to flip the filter mid-stream. Assert that a `tracing::debug!` event before reload-to-debug is filtered out and after is not. This is the same pattern as the existing `test_reload_level_changes_filter` but exercised through `build_layers`'s public API rather than a hand-built `reload::Layer`. Note: emission goes to the file appender for `stdout: false`, which is hard to assert against; instead, configure `stdout: true` and pipe stdout through a `CapturingWriter` by adding an extra fmt layer attached only in the test (or simply assert that `handle.reload(...).is_ok()` for both pre- and post-reload calls and rely on the existing `test_reload_level_changes_filter` for emission semantics — implementer's choice based on what compiles cleanly).

3. Update the explanatory comment at line 212 of `tests/logs/mod.rs` (currently: `// Note: tests that call \`logs::init\` ... live in dedicated integration-test binaries`). Replace with a comment explaining the new split: most layer-construction coverage lives here via `build_layers`; only tests that actually need `set_global_default` to fire (or that need a non-`RUST_LOG=off` environment) live in `tests/logs_init_*.rs`.

4. Update the comment at line 270 (`// \`test_reload_level_no_op_when_env_filter_locked\` lives in ...`). Keep noting that `tests/logs_init_locked.rs` still exists; remove or revise references to the deleted binaries.

5. Run `./scripts/test.sh` from the repo root. Expect all new tests to pass alongside the existing ones.

6. Commit M2.

### Milestone 3 — Remove superseded top-level test binaries

1. Combine smoke + double-install into a single new binary `agent/agent/tests/logs_init_smoke.rs`:

       // Dedicated integration-test binary so the global-subscriber install in this
       // test cannot collide with subscribers installed by other integration tests.
       // Also covers the SetGlobalDefault error path: a second init in the same
       // process must fail with LogsErr::SetGlobalDefault.

       use miru_agent::filesys::{Dir, PathExt};
       use miru_agent::logs::{self, LogsErr, Options};

       use serial_test::serial;

       #[tokio::test]
       #[serial]
       async fn test_init_installs_globally_and_rejects_double_install() {
           let dir = Dir::create_temp_dir("miru_test_logs_smoke").await.unwrap();

           let options = Options {
               stdout: false,
               log_dir: dir.path().clone(),
               ..Default::default()
           };
           let guard = logs::init(options).expect("first init should succeed");

           let options_second = Options {
               stdout: false,
               log_dir: dir.path().clone(),
               ..Default::default()
           };
           match logs::init(options_second) {
               Err(LogsErr::SetGlobalDefault(_)) => {}
               Err(other) => panic!("expected LogsErr::SetGlobalDefault on double init, got: {other:?}"),
               Ok(_) => panic!("expected double init to fail, but it succeeded"),
           }

           drop(guard);
       }

2. Delete the now-redundant binaries with `git rm`:

       git rm agent/tests/logs_init_double.rs
       git rm agent/tests/logs_init_stdout.rs
       git rm agent/tests/logs_init_file_only.rs
       git rm agent/tests/logs_init_reload.rs

   Keep `agent/tests/logs_init_locked.rs` as-is — it still needs its own binary because it asserts the `RUST_LOG`-locked branch through the real `logs::init`, and the shared integration binary already inherits `RUST_LOG=off` (so the not-locked branch wouldn't be reachable there even if we tried).

3. Confirm result is `tests/logs_init_smoke.rs` + `tests/logs_init_locked.rs` = two top-level binaries (was five).

4. Run `./scripts/test.sh` from the repo root. Expect all tests to pass.

5. Commit M3.

### Milestone 4 — Preflight and finalize

1. From `agent/`, run `./scripts/preflight.sh`. Wait for completion. Expected last line: `Preflight clean`.

2. If `Preflight FAILED`, read the inline `=== Lint ===` and `=== Tests ===` blocks, fix the underlying issue, and re-run. Common failures and fixes:
   - Coverage gate trip on `agent/src/logs/.covgate`: the matrix tests should preserve coverage; if they don't, add targeted tests until coverage rises above the gate. Do not lower the gate.
   - Clippy unused-import warning on `Layer` or `boxed`: tighten imports in `agent/src/logs/mod.rs`.
   - Import-linter findings on the new test file: ensure imports are grouped `// standard / // internal / // external`, blank line between groups (matches the convention in AGENTS.md and existing tests).

3. Commit M4 (any preflight-driven fixes; if none, skip the empty commit).

## Concrete Steps

All commands assume working directory `agent/` (the repo root) unless stated otherwise. Branch `feat/idempotent-upgrade-reset` is already checked out — do not switch.

Milestone 1:

    # 1. Edit agent/src/logs/mod.rs to add build_layers and rewrite init.
    #    (Use your editor; see Plan of Work § Milestone 1 for code.)

    cargo build --package miru-agent --features test
    # Expected: Compiling miru-agent ... Finished `dev` profile.

    ./scripts/test.sh
    # Expected: test result: ok. <N> passed; 0 failed (before any test reorg).

    git add agent/src/logs/mod.rs
    git commit -m "refactor(logs): factor init into pure build_layers + thin install wrapper"

Milestone 2:

    # 1. Edit agent/tests/logs/mod.rs to add the build_layers matrix tests.
    #    Update the two explanatory comments noted in Plan of Work.

    ./scripts/test.sh
    # Expected: test result: ok. all new build_layers tests pass.

    git add agent/tests/logs/mod.rs
    git commit -m "test(logs): cover build_layers matrix in shared integration binary"

Milestone 3:

    # 1. Create agent/tests/logs_init_smoke.rs with the combined smoke + double-install test.

    git rm agent/tests/logs_init_double.rs
    git rm agent/tests/logs_init_stdout.rs
    git rm agent/tests/logs_init_file_only.rs
    git rm agent/tests/logs_init_reload.rs
    # Expected: rm 'agent/tests/logs_init_*.rs' x4

    ls agent/tests/logs_init_*.rs
    # Expected:
    #   agent/tests/logs_init_locked.rs
    #   agent/tests/logs_init_smoke.rs

    ./scripts/test.sh
    # Expected: test result: ok. 0 failed.

    git add agent/tests/logs_init_smoke.rs
    git commit -m "test(logs): collapse 5 logs_init binaries to 2 (smoke + locked)"

Milestone 4:

    ./scripts/preflight.sh
    # Expected last line: "Preflight clean"

    # If anything failed, address it and (if changes were needed) commit:
    # git add -p ; git commit -m "fix(logs): address preflight findings"

End of milestones. Plan deliverable is complete; promotion to `plans/active/` is the implementer's responsibility.

## Validation and Acceptance

The change is complete when **all** of the following are observably true.

1. From `agent/`, `cargo build --package miru-agent --features test` succeeds with no warnings.
2. From `agent/`, `./scripts/test.sh` reports `test result: ok. 0 failed` for every binary it runs.
3. The directory listing `ls agent/tests/logs_init_*.rs` shows exactly two files: `logs_init_locked.rs` and `logs_init_smoke.rs`.
4. `agent/tests/logs/mod.rs` contains, at minimum, these new test functions: `test_build_layers_stdout_debug`, `test_build_layers_file_only_warn`, `test_build_layers_respects_rust_log_when_set`, `test_build_layers_uses_options_when_rust_log_unset`, `test_build_layers_reload_handle_changes_filter`. Each has a corresponding `cargo test --package miru-agent --features test logs::<name>` line (replace `logs::` with the actual integration-binary path) that runs and passes.
5. `agent/src/logs/mod.rs::init` is at most ~10 lines of body (the worker/handle wiring lives entirely inside `build_layers`).
6. `agent/src/main.rs` is unmodified — `git diff main -- agent/src/main.rs` from the repo root shows no changes touching `logs::init` or `log_guard`.
7. Public API surface is unchanged: `LoggingGuard::reload_level`, `LoggingGuard::env_filter_locked`, `LogsErr` variants, `From<LogsErr> for ProvisionErr`. `cargo public-api` would show one addition (`build_layers` and the `BoxedLogLayer` alias) and no removals or modifications.
8. **Preflight gate.** `./scripts/preflight.sh` from `agent/` ends with the literal line `Preflight clean`. **This must report `clean` before the changes are published.** A `Preflight FAILED` run is not acceptance — fix and rerun.

Behavioral acceptance (the "user can do" view): an operator running `miru-agent` sees identical log output before and after the refactor; `RUST_LOG=debug miru-agent` still logs at debug level; settings-driven log level still applies on settings-load. Neither pattern is regressed because `init` is still the same composition, just split.

## Idempotence and Recovery

- Milestone 1's source edit is idempotent: running the editor again produces the same `mod.rs`. If `cargo build` fails after the edit, re-edit to fix the type signature; the locked workspace version of `tracing-subscriber` may not expose `Layer::and_then` or `Layer::boxed` exactly as written — fall back to the alternate "return tuple of pieces" signature documented in Plan of Work § Milestone 1, step 2, and have `init` compose with `.with(...)`.
- Milestone 2's test additions are additive and do not interact with global state (they use `with_default` and `build_layers` only). Re-running them is safe.
- Milestone 3's `git rm` operations are recoverable via `git checkout main -- agent/tests/logs_init_<name>.rs` if a deletion is determined to have been wrong (e.g., a unique assertion was missed). Verify the new `tests/logs/mod.rs` matrix before deleting; once committed, revert is `git revert <commit-sha>`.
- Milestone 4 is a read-only gate. If `preflight.sh` reports `Preflight FAILED`, the underlying lint/test/covgate output is captured in the script's `=== Lint ===` / `=== Tests ===` blocks; address the specific finding and rerun. Do not modify `.covgate` thresholds to mask real coverage drops.
- The whole refactor is reversible: `git revert` the four milestone commits in reverse order restores the prior state exactly. The five deleted binaries are recoverable from history.
