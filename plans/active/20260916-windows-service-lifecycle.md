# Windows service lifecycle: run the agent under the Service Control Manager

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench2/repos/agent) | read/write | Rust workspace for the Miru device agent (crate `miru-agent` in `agent/`). All edits, validation, and commits happen here. |

Branch `feat/windows-service-lifecycle` (created and checked out; equals `main` at `4b3ac664`,
clean tree), PR base `main`; **PR 5** of the roadmap `plans/active/20260910-windows-support.md`.

## Purpose / Big Picture

On Windows the agent must run as a Windows service: started by the Service Control Manager (SCM),
reporting `StartPending → Running → StopPending → Stopped`, and shutting down cleanly on
`SERVICE_CONTROL_STOP` / `SERVICE_CONTROL_SHUTDOWN`. Today the Windows build only knows ctrl-c.

After this PR: `miru-agent.exe` started by the SCM runs as service `miru-agent` and stops within
~15 s of `sc.exe stop`; `--console` keeps today's foreground ctrl-c behavior; without `--console`
outside the SCM it prints an error naming `--console` and exits 1; on Windows
`settings.is_persistent = false` is overridden to persistent with a warning. Linux behavior stays
byte-identical; nothing inside `app::run` changes.

Non-goals: MSI `ServiceInstall` (PR 8 / draft #236 — the service name `miru-agent` here must
match its future `ServiceInstall Name`), a Windows Event Log sink, `PRESHUTDOWN` /
pause-continue, a service-account privilege check, `catch_unwind` around the agent body, socket
activation or idle-exit on Windows, and clippy for the Windows target in CI.

## Progress

- [x] M0 Activate plan (`docs(plans):` commit; roadmap PR 5 marker)
- [x] M1 `windows-service` dependency + lockfile; portable `service` module + tests
- [x] M2 `service/windows.rs` SCM plumbing + `cfg(windows)` tests
- [ ] M3 `--console` flag, `platform::supports_idle_exit`, `resolve_persistence` + tests
- [ ] M4 `main.rs` restructure (sync `main`, `run_runtime_mode`, `service_body`); draft PR opened
- [ ] M5 `ARCHITECTURE.md` updates
- [ ] M6 `./scripts/preflight.sh` CLEAN; CI green incl. `windows-check`; PR leaves draft

## Surprises & Discoveries

- 2026-09-16 (M1): the lockfile step added exactly `widestring 1.2.1` and `windows-service 0.8.1`
  (both `bitflags 2.13.1` and `windows-sys 0.61.2` were already locked), but the re-resolve also
  flipped `tempfile 3.27.0`'s `getrandom` edge from 0.3.4 to 0.4.3: tempfile declares
  `getrandom = ">=0.3.0, <0.5"`, so any resolve unifies it onto the highest already-locked
  version. Both getrandom versions stay in the lock for other dependents; the second
  `cargo check` is a no-op, so the change is kept.
- 2026-09-16 (M1): cargo-machete did not flag `windows-service` on Linux (it scans source text and
  sees the `windows_service::Error` field in `service/errors.rs`), so no
  `[package.metadata.cargo-machete]` entry was needed.
- 2026-09-16 (M1): the `ServiceErr` Display test lives in `agent/tests/service/errors.rs`
  (mirroring `agent/src/service/errors.rs`) rather than inside `stop_signal.rs`.
- 2026-09-16 (M2): `scripts/lint.sh` runs `cargo fmt` in write mode and rustfmt follows the
  `#[cfg(windows)] pub mod windows;` declaration, so both Windows-only files are formatted (and
  `fmt --check`-clean) on Linux even though rustc/clippy never compile them here.
- 2026-09-16 (M2): to catch type/borrow errors before `windows-check`, `service/windows.rs` and
  `tests/service/windows.rs` were also compiled and run on Linux against a throwaway shim crate
  (session scratchpad, not committed) that mirrors the windows-service 0.8.1 public signatures
  copied from the registry source (`ServiceStatus` derives, `#[non_exhaustive]` `Error` /
  `ServiceControl`, `register` bounds, `define_windows_service!` verbatim). All 17 tests plus a
  `dispatch` check (`Winapi` raw_os_error 1063 → `NotLaunchedByScm`) pass under
  `clippy -D warnings`. The real Windows CI job remains the arbiter.

## Decision Log

- 2026-09-16: SCM integration lives in a new lib module `agent/src/service/` (`mod.rs`,
  `errors.rs` portable; `windows.rs` under `#[cfg(windows)]`), not in `main.rs` (a bin, outside
  covgate) or `platform/` (path/capability dispatch only); `windows_service` would shadow the crate.
- 2026-09-16: `StopSignal` wraps `tokio::sync::watch::channel(false)`: `watch` over `Notify`
  (lost-wakeup hazard with two waiters) and over `broadcast` (would duplicate `run`'s own
  channel); `send_replace(true)` is sync, idempotent, and needs no runtime.
- 2026-09-16: `app::run`'s private `ShutdownManager` is not bypassed — `run` still receives a
  plain future; service mode passes `stop.wait()` where Linux passes `await_shutdown_signal()`.
- 2026-09-16: `Running` is reported *before* the agent body runs (`await_activation` can wait
  indefinitely; SCM kills a service stuck in `StartPending`). `StopPending` (30 s `wait_hint`) is
  reported by the control handler the moment STOP/SHUTDOWN arrives, so SCM sees a pending stop
  during `ShutdownManager`'s ≤ 15 s drain; `run_lifecycle` reports it again (harmless repeat)
  after `body()` returns so an error exit with no control also passes through `StopPending`. The
  handler reaches the status handle via a shared `Arc<OnceLock<ServiceStatusHandle>>` filled
  right after `register` returns (`ServiceStatusHandle: Copy + Send + Sync`). A drain-timeout
  `exit(1)` is logged by SCM as an unexpected termination — accepted.
- 2026-09-16: `ServiceErr` follows the `privilege/errors.rs` precedent (unconditional
  `NotLaunchedByScm`, cfg-gated `Scm { source }`; blanket `impl crate::errors::Error`). Win32
  error 1063 → `NotLaunchedByScm`, message points at `--console`, exit 1 — no silent fallback.
- 2026-09-16: forced persistence = `platform::supports_idle_exit()` (unix `true`, windows
  `false`) + pure `LifecycleOptions::resolve_persistence(requested, supports_idle_exit)`:
  `main.rs` stays cfg-free at that site and the fn is unit-testable (app covgate 90.38 needs it).
- 2026-09-16: `--console` is a bare flag parsed like `--version` (`trim_start_matches('-')`),
  accepted and ignored on Unix. No `is_runtime()` helper on `Args`; `main.rs` decides from
  existing fields.
- 2026-09-16: `main()` becomes sync: `service_dispatcher::start` blocks the calling thread and
  the SCM-spawned `service_main` thread needs its own tokio runtime; `runtime()` builds what
  `#[tokio::main]` expands to. The cfg'd block in `run_runtime_mode`, `service_body`, and the gated
  `service::{self, StopSignal}` import are the only new `#[cfg]`s in `main.rs` — irreducible.
- 2026-09-16: service mode logs with `logs::Options { stdout: false, .. }` (rolling `miru.log`
  under `%ProgramData%\Miru\logs`, the `run_provision` precedent); console mode keeps
  `Options::default()`. A service-mode `logs::init` failure → `Failed` → `ServiceSpecific(1)`.
- 2026-09-16: `agent/src/service/.covgate` is `90.00`, not `0`: covgate runs on Linux only, where
  `windows.rs` contributes zero regions, so the gate measures exactly the portable relay and error
  type, which should be gated. Windows-only code is verified by `windows-check`, not covgate.
- 2026-09-16: the console path discards `RunOutcome` and exits 0 as today; only the service path
  maps it to an exit code.
- 2026-09-16 (M2): `dispatch` matches `Err(windows_service::Error::Winapi(ref e))` with a
  `raw_os_error() == Some(1063)` guard (named const `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT`)
  and a catch-all `Err(source)` arm for the `#[non_exhaustive]` enum. `run_lifecycle` returns
  `stop_pending.and(stopped)` so the `StopPending` error wins when both trailing reports fail.
- 2026-09-16 (M2): the test `RecordingSink` (`RefCell<Vec<ServiceStatus>>` + `fail_on:
  Option<ServiceState>`) records only successful reports and stays local to
  `tests/service/windows.rs` (cfg(windows)-only, so not in `test_utils`). One extra
  `run_lifecycle` case beyond the plan list: a sink failing on `Running` skips the body and
  leaves only `StartPending` recorded.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Toolchain 1.97.0, MSRV 1.93.0, tokio 1.53.1. `AGENTS.md` rules that bite here: import groups
`// standard crates` / `// internal crates` / `// external crates`; funclen ≤ 50; `errors.rs` per
module; integration tests under `agent/tests/<module>/` declared in `agent/tests/mod.rs`; ≥4
`assert_eq!` on one variable trips the `field-by-field-assert` lint — compare whole structs.

`agent/src/main.rs` is the bin (`handle_provision_result` line 96, `handle_reprovision_result`
148); `run_agent` (165) calls `await_shutdown_signal()` twice (into `await_activation(&layout,
tokio::time::sleep, …)` and `run(...)`), each failure is `error!` + `return` (log-init failure:
`eprintln!` + `return`); lines 25-26 gate `#[cfg(unix)] use tokio::signal::unix::signal;`;
`await_shutdown_signal` is cfg-split at 271-295.
`agent/src/app/run.rs:30-33`: `pub async fn run(options: AppOptions, shutdown_signal: impl
Future<Output = ()> + Send + 'static)`. Other modules touched: `agent/src/cli/mod.rs` (bare flags
match on `trim_start_matches('-')`); `agent/src/platform/mod.rs` (dispatchers hold the only
`#[cfg]`s, as `#[cfg(unix)] { ... }` / `#[cfg(windows)] { ... }` blocks); `agent/src/errors/mod.rs`
(trait `Error`, all default methods; `trace!()` → `Box<Trace>`).

The `windows-service` 0.8.1 crate is `#![cfg(windows)]` (empty on Linux), so every `use
windows_service::…` must sit inside `cfg(windows)` items. Beyond what M2 spells out:
`service_dispatcher::start` blocks until the service stops and fails with
`Error::Winapi(io::Error)` when not launched by SCM; the `register` handler runs on the
dispatcher thread and must return quickly; `ServiceStatus` has no `Default`; `ServiceControl` and
`Error` are `#[non_exhaustive]` (`_ =>` arms).

Scripts (Linux only): `scripts/test.sh` (`RUST_LOG=off cargo test --package miru-agent`),
`scripts/lint.sh` (custom linter, fmt, machete, audit, clippy `-D warnings`), `scripts/covgate.sh`
(per `agent/src/**/.covgate`), `scripts/preflight.sh` (lint + covgate + tools lint + tools
covgate). CI (`.github/workflows/ci.yml`): `lint`, `test` (covgate), `tools` on ubuntu;
`windows-check` on windows-latest runs only `RUST_LOG=off cargo test --package miru-agent
--locked` (no clippy, no covgate; rustc warnings do not fail it — read the log). No local Windows
host exists and cross-compiling from Linux fails (aws-lc-sys), so `windows-check` is the arbiter
for Windows-only code.

## Plan of Work / Concrete Steps

Working directory for every command: `/home/ben/miru/workbench2/repos/agent`. One commit per
milestone, in the M0 form, ending with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

### M0 — Activate plan

1. This file exists at `plans/active/20260916-windows-service-lifecycle.md`.
2. Append the marker `(in progress — \`plans/active/20260916-windows-service-lifecycle.md\`)` to
   the PR 5 paragraph of `plans/active/20260910-windows-support.md` (starts `**PR 5 — Windows
   service lifecycle.**`, line 102). No other roadmap edits (draft PR #236 rewrites that file).
3. Commit:

        git add plans/active/20260916-windows-service-lifecycle.md \
            plans/active/20260910-windows-support.md
        git commit -m "docs(plans): add windows service lifecycle plan" \
            -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"

### M1 — Dependency and portable `service` module

1. `/Cargo.toml` `[workspace.dependencies]`: append `windows-service = "0.8.1"` after `uuid`
   (currently the last entry; the list is not alphabetical). `agent/Cargo.toml`, after the
   `[target.'cfg(unix)'.dependencies]` block:

        [target.'cfg(windows)'.dependencies]
        windows-service = { workspace = true }

2. Update the lockfile for all targets (Windows CI uses `--locked`) with
   `cargo check --package miru-agent`. Expected: `Adding widestring v1.x` and `Adding
   windows-service v0.8.1`, then `Finished`; `git diff --stat Cargo.lock` shows the two new
   packages; a second run prints no `Adding` lines.
3. `agent/src/service/mod.rs` — module doc "OS service-manager integration"; `pub mod errors;
   #[cfg(windows)] pub mod windows;`; portable types:

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum RunOutcome { Completed, Failed }
        #[derive(Debug, Clone)]
        pub struct StopSignal { tx: watch::Sender<bool> }   // plus impl Default (= new())
        impl StopSignal {
            pub fn new() -> Self                       // watch::channel(false)
            pub fn trigger(&self)                      // tx.send_replace(true)
            pub fn is_triggered(&self) -> bool         // *tx.borrow()
            pub fn wait(&self) -> impl Future<Output = ()> + Send + 'static {
                let mut rx = self.tx.subscribe();      // Receiver<bool> is Send
                async move { let _ = rx.wait_for(|v| *v).await; }
            }
        }

   `wait` owns its `Receiver` because `app::run` needs a `'static` future; `wait_for` checks the
   current value first, so a `wait()` created after `trigger()` resolves at once. Doc comments
   state the contract: `trigger` is callable from a non-tokio thread, idempotent, and wakes every
   past and future `wait()`. Create `agent/src/service/windows.rs` now as a doc-comment-only stub
   (`//! Windows SCM integration (filled in by M2).`) so the `pub mod` line compiles on Windows.
4. `agent/src/service/errors.rs`:

        #[derive(Debug, thiserror::Error)]
        pub enum ServiceErr {
            #[error("miru-agent was not started by the Windows Service Control \
                     Manager; run it with --console to run in the foreground")]
            NotLaunchedByScm { trace: Box<Trace> },
            #[cfg(windows)]
            #[error("service control manager call failed: {source}")]
            Scm { source: windows_service::Error, trace: Box<Trace> },
        }
        impl crate::errors::Error for ServiceErr {}

5. `agent/src/service/.covgate` containing `90.00`.
6. Tests: `agent/tests/service/mod.rs` with `pub mod stop_signal;` (M2 adds `#[cfg(windows)] pub
   mod windows;`); `pub mod service;` between `server` and `services` in both `agent/src/lib.rs`
   and `agent/tests/mod.rs`. `agent/tests/service/stop_signal.rs` (`#[tokio::test]`, `timeout`):
   - `trigger` resolves two independent `wait()` futures (1 s timeout); a `wait()` created after
     `trigger()` resolves at once; untriggered `wait()` stays pending (`timeout(100ms)` → `Err`);
   - `trigger()` twice is harmless and `wait()` still resolves; `trigger()` from
     `std::thread::spawn` (no runtime) wakes an async waiter;
   - `is_triggered()` is false then true; `Default` equals `new()` behavior;
   - `ServiceErr::NotLaunchedByScm { trace: miru_agent::trace!() }` Display contains `--console`
     (here or in a small `tests/service/errors.rs`).
7. `./scripts/test.sh` and `./scripts/lint.sh` pass. Commit `feat(windows): add windows-service
   dependency and stop-signal relay`.

### M2 — SCM plumbing (`service/windows.rs`)

1. `agent/src/service/windows.rs` (`windows_service::{define_windows_service, service::{…},
   service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
   service_dispatcher}` imports go in the external group):

        pub const SERVICE_NAME: &str = "miru-agent";
        const PENDING_WAIT_HINT: Duration = Duration::from_secs(30);
        pub type ServiceBody = fn(StopSignal) -> RunOutcome;
        static BODY: OnceLock<ServiceBody> = OnceLock::new();
        pub fn dispatch(body: ServiceBody) -> Result<(), ServiceErr>
            // BODY.set(body) (ignore AlreadySet); service_dispatcher::start(SERVICE_NAME,
            // ffi_service_main); Err(Winapi(e)) with e.raw_os_error() == Some(1063)
            // → NotLaunchedByScm, any other Err → Scm { source }
        define_windows_service!(ffi_service_main, service_main);
        fn service_main(_args: Vec<OsString>)
            // let Some(body) = BODY.get() else { return };
            // let stop = StopSignal::new(); let handler_stop = stop.clone();
            // let slot: Arc<OnceLock<ServiceStatusHandle>> = Arc::default();
            // let handler_slot = slot.clone();
            // let handle = match service_control_handler::register(SERVICE_NAME,
            //     move |c| handle_control(c, &handler_stop, handler_slot.get()))
            //     { Ok(h) => h, Err(_) => return };   // no handle → nothing can be reported
            // let _ = slot.set(handle);   // SCM sends no controls before Running is reported
            // let _ = run_lifecycle(&handle, || body(stop));
        pub fn handle_control<S: StatusSink>(control: ServiceControl, stop: &StopSignal,
            sink: Option<&S>) -> ServiceControlHandlerResult
            // Stop | Shutdown → if let Some(s) = sink { let _ = s.report(status(StopPending,
            //   NO_ERROR)); } then stop.trigger(); NoError. Interrogate → NoError.
            //   _ → NotImplemented. Runs on the SCM thread: sync, no awaits, no locks held.
        pub fn status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus
            // OWN_PROCESS; controls_accepted = STOP | SHUTDOWN iff Running, else empty();
            // checkpoint 0; wait_hint = PENDING_WAIT_HINT for StartPending/StopPending, else
            // Duration::ZERO; process_id None
        pub fn exit_code(outcome: RunOutcome) -> ServiceExitCode
            // Completed → NO_ERROR; Failed → ServiceSpecific(1)
        pub trait StatusSink { fn report(&self, status: ServiceStatus) -> Result<(), ServiceErr>; }
        impl StatusSink for ServiceStatusHandle   // set_service_status → Scm
        pub fn run_lifecycle<S: StatusSink>(
            sink: &S, body: impl FnOnce() -> RunOutcome) -> Result<(), ServiceErr>
            // report(StartPending, NO_ERROR)?; report(Running, NO_ERROR)?; let outcome = body();
            // always attempt report(StopPending) AND report(Stopped, exit_code(outcome));
            // return the first error encountered

2. `agent/tests/service/windows.rs` (`#[cfg(windows)] pub mod windows;` in
   `agent/tests/service/mod.rs`); only `dispatch`, `service_main`, and the `StatusSink` impl touch
   Win32, so the rest is testable without a registered service. One `pub mod` per fn under test
   (`handle_control`, `status`, `exit_code`, `run_lifecycle`):
   - `handle_control` (recording sink passed as `Some(&sink)`): `Stop` → sink recorded exactly
     `[status(StopPending, NO_ERROR)]`, triggered, `NoError`; `Shutdown` → same; `Interrogate` →
     `NoError`, sink empty, not triggered; `Pause` → `NotImplemented`, sink empty, not triggered;
     `Stop` with `None::<&RecordingSink>` → still triggered + `NoError`; a sink whose `report`
     fails → still triggered + `NoError` (report errors never block the stop).
     `ServiceControlHandlerResult` derives only `Debug`, so assert with
     `assert!(matches!(result, ServiceControlHandlerResult::NoError))`, not `assert_eq!`.
   - `status`: compare whole `ServiceStatus` structs — `Running` → controls `STOP | SHUTDOWN`,
     wait_hint `ZERO`; `StartPending`/`StopPending` → empty controls, wait_hint 30 s; `Stopped`
     with `ServiceSpecific(1)`; all with `OWN_PROCESS`, checkpoint 0, `process_id: None`.
   - `exit_code`: `Completed` → `NO_ERROR`; `Failed` → `ServiceSpecific(1)`.
   - `run_lifecycle` with a recording sink (`RefCell<Vec<ServiceStatus>>`): state sequence
     `[StartPending, Running, StopPending, Stopped]`; `Stopped` carries the exit code for each
     outcome; a sink failing on `StartPending` returns `Err` and the body never runs
     (`Cell<bool>` flag); a sink failing only on `StopPending` still reports `Stopped` and
     returns `Err`. Failing sinks return `ServiceErr::NotLaunchedByScm { trace }` as the
     stand-in (constructing `windows_service::Error` is unnecessary).
3. Linux `./scripts/test.sh` and `./scripts/lint.sh` pass unchanged (the module is `cfg(windows)`).
   Commit `feat(windows): add SCM control handler and service status lifecycle`.

### M3 — `--console`, `supports_idle_exit`, `resolve_persistence`

1. `agent/src/cli/mod.rs`: `Args` gains

        /// Run the agent in the foreground with ctrl-c shutdown instead of as a Windows
        /// service. Accepted and ignored on Unix, where the agent always runs in the foreground.
        pub console: bool,

   and the parser arm `"console" => args.console = true`. Tests in `agent/tests/cli/mod.rs`
   `args_parse`: `console` true for `--console`, `-console`, `console`; false by default;
   `--version --console` sets both.
2. `agent/src/platform/mod.rs` (also update the module doc, which covers only paths today):

        /// Whether the runtime may exit when idle (`settings.is_persistent = false`). Only
        /// Unix supports it — socket activation restarts the agent on demand. Windows runs
        /// as a service and is persistent-only.
        pub fn supports_idle_exit() -> bool {
            #[cfg(unix)] { true }
            #[cfg(windows)] { false }
        }

   Tests in `agent/tests/platform/mod.rs` `dispatch`: `#[cfg(unix)]` asserts true,
   `#[cfg(windows)]` asserts false.
3. `agent/src/app/options.rs`:

        impl LifecycleOptions {
            /// Resolves the effective persistence for this platform; warns when a
            /// non-persistent request is overridden.
            pub fn resolve_persistence(requested: bool, supports_idle_exit: bool) -> bool {
                if !requested && !supports_idle_exit {
                    tracing::warn!("settings.is_persistent = false is not supported on \
                                    this platform; running persistently");
                }
                requested || !supports_idle_exit
            }
        }

   Tests in `agent/tests/app/options.rs`, new `pub mod lifecycle_options_resolve_persistence`:
   all four bool combinations.
4. `./scripts/test.sh`, `./scripts/lint.sh` pass. Commit `feat(windows): add --console flag and
   force persistence on windows`.

### M4 — `main.rs` restructure

1. Drop `#[tokio::main]`; `fn main()` is sync. Everything through the `provision_args.check`
   block (lines 43-53, already synchronous) is unchanged; the `provision`/`reprovision` branches
   become `handle_provision_result(runtime().block_on(run_provision(p)))` /
   `handle_reprovision_result(runtime().block_on(run_reprovision(r)))` + `return`, and the last
   statement is `run_runtime_mode(cli_args.console)`. New and changed fns:

        fn runtime() -> tokio::runtime::Runtime {
            tokio::runtime::Builder::new_multi_thread().enable_all().build()
                .expect("failed to build tokio runtime")
        }
        fn run_runtime_mode(console: bool) {
            #[cfg(windows)]
            {
                if !console {
                    if let Err(e) = service::windows::dispatch(service_body) {
                        eprintln!("miru-agent: {e}");
                        std::process::exit(1);
                    }
                    return;
                }
            }
            let _ = console; // unix: the flag is a no-op (keeps clippy -D warnings quiet)
            runtime().block_on(run_agent(logs::Options::default(), await_shutdown_signal));
        }
        #[cfg(windows)]
        fn service_body(stop: StopSignal) -> RunOutcome {
            let options = logs::Options { stdout: false, ..Default::default() };
            runtime().block_on(run_agent(options, move || stop.wait()))
        }
        async fn run_agent<F, Fut>(log_options: logs::Options, shutdown: F) -> RunOutcome
        where F: Fn() -> Fut, Fut: Future<Output = ()> + Send + 'static

   `run_agent`'s body is unchanged except: `logs::init(log_options)`; `shutdown()` at the two
   former `await_shutdown_signal()` sites; each early `return` becomes `return
   RunOutcome::Failed` (logs init, http client, reconcile, settings) except
   `Outcome::ShutdownRequested => return RunOutcome::Completed`; `run(...)` `Err` → `error!` +
   `Failed`, `Ok` → `Completed`. If that exceeds 50 body lines, extract `async fn
   reconcile_version(layout: &disk::Layout) -> bool` instead of adding `lint:allow(funclen)`.
2. `build_app_options`: `is_persistent: LifecycleOptions::resolve_persistence(
   settings.is_persistent, platform::supports_idle_exit())`.
3. Imports: `std::future::Future` (standard group); `use miru_agent::platform;` between
   `miru_agent::network` and `miru_agent::privilege`; and these two internal-group lines, in this
   order (`RunOutcome` is needed on every target; the gate mirrors lines 25-26):

        use miru_agent::service::RunOutcome;
        #[cfg(windows)]
        use miru_agent::service::{self, StopSignal};

   Replace the Windows `await_shutdown_signal` comment with "console mode: ctrl-c; service mode
   uses `service::StopSignal`".
4. Validate:

        ./scripts/test.sh
        ./scripts/lint.sh
        cargo run --package miru-agent -- --version      # prints version
        cargo run --package miru-agent -- --console      # exit 1, WrongUser message
        cargo run --package miru-agent                   # identical exit and message

   (Both fail the `miru` privilege check identically, before either flag matters. Do NOT run the
   dev binary as `miru`: this host has an activated device and a live `miru.service`, so it would
   start a second production agent.) Commit `feat(windows): run the agent as a windows service`.
5. CI runs only on `pull_request` (`.github/workflows/ci.yml`), so open the draft PR now to get
   `windows-check` compiling `service/windows.rs` and `service_body` for the first time. Write the
   PR body to `/tmp/windows-service-lifecycle-pr.md` in the repo's PR style, ending with the line
   `🤖 Generated with [Claude Code](https://claude.com/claude-code)`, then:

        git push -u origin feat/windows-service-lifecycle
        gh pr create --draft --base main \
            --title "feat(windows): run the agent as a Windows service" \
            --body-file /tmp/windows-service-lifecycle-pr.md
        gh pr checks --watch             # iterate on the windows-check job log

### M5 — Documentation

`ARCHITECTURE.md`:
- Bird's Eye View (line 12, `**Agent runtime mode**` bullet): on Windows the runtime mode runs as
  service `miru-agent` under the SCM unless `--console` is passed (foreground, ctrl-c shutdown);
  `--console` is accepted and ignored on Unix.
- Codemap, Core infrastructure: add `platform` ("per-OS defaults (data root, log dir) and
  capability dispatch (`supports_idle_exit`); OS-specific fns compile on every target for
  testability") and `service` ("OS service-manager integration: portable `StopSignal` relay and
  `RunOutcome`; `service::windows` holds the SCM entry point, control handler, and status
  lifecycle (`cfg(windows)`)"); mention `--console` in `cli`'s entry.
- Cross-Cutting Concerns, Graceful shutdown: SIGTERM/SIGINT/ctrl-c on Unix, ctrl-c in Windows
  console mode, and `SERVICE_CONTROL_STOP`/`SHUTDOWN` in service mode (via `service::StopSignal`)
  all resolve the shutdown future `app/run.rs` awaits; broadcast channel and ordering unchanged.
- Architectural Invariants: **Windows is persistent-only.** `settings.is_persistent = false` is
  ignored on Windows with a startup warning (`platform::supports_idle_exit`); socket activation
  and idle exit are Linux-only.

Commit `docs(architecture): document windows service mode and --console`.

### M6 — Preflight, CI

        ./scripts/preflight.sh           # must be CLEAN: exit 0, last line "Preflight clean"
        git push
        gh pr checks --watch

All four CI jobs (Validation 2) must be green on the pushed head; if `windows-check` fails, fix
from the job log and re-push. Only then does the PR (opened in M4) leave draft.

## Validation and Acceptance

Required before the PR leaves draft or the task is reported complete:

1. Preflight CLEAN: `./scripts/preflight.sh` (M6) exits 0, all four checks (agent lint, agent
   covgate, tools lint, tools covgate) green, last line `Preflight clean`; covgate reports ≥ 90.00
   for `agent/src/service`, and app ≥ 90.38, cli = 100, platform ≥ 95.00 still hold.
2. CI green on the pushed head: `lint`, `test`, `tools`, `windows-check`; the latter's `cargo test
   --locked` succeeds (the M1 lockfile holds `windows-service`/`widestring`) and lists the
   `service::windows::{handle_control, status, exit_code, run_lifecycle}` tests as `ok`.
3. Linux behavior byte-identical: the M4 step 4 commands behave as stated; `test.sh` ends with
   `test result: ok.` everywhere and lists the new tests `service::stop_signal`,
   `app::options::lifecycle_options_resolve_persistence`, `cli::args_parse` (console), and
   `platform::dispatch` (supports_idle_exit).
4. Lockfile stable: the M1 step 2 `cargo check` is a no-op on a second run.
5. Docs: every M5 bullet is present in `ARCHITECTURE.md`; `cli::Args::console` has its doc
   comment; the roadmap diff is the M0 marker and nothing else.

Optional manual Windows validation (only if a Windows VM is available):

    sc.exe create miru-agent binPath= "C:\Program Files\Miru\Agent\miru-agent.exe" start= demand
    sc.exe start miru-agent && sc.exe query miru-agent     # STATE: RUNNING
    dir %ProgramData%\Miru\logs                            # miru.log.<hour> present
    sc.exe stop miru-agent           # STOPPED within ~15 s; log has "Shutdown signal received"
    miru-agent.exe                   # prints the NotLaunchedByScm message, exit code 1
    miru-agent.exe --console         # logs to stdout; ctrl-c stops it
    sc.exe delete miru-agent

## Idempotence and Recovery

All edits are additive and re-runnable; re-applying a milestone over a partially applied tree
converges, and the `cargo check` lockfile step is a no-op on repeat. One commit per milestone, so
`git revert <sha>` unwinds one in isolation. Risky step: the service-mode relay. If it misbehaves,
console mode and Linux are unaffected (the console path never touches `StopSignal`), and deleting
the `#[cfg(windows)]` block in `run_runtime_mode` restores the previous ctrl-c-only Windows
behavior.
