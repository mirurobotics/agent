# Remove the agent's non-persistent (idle-exit) mode

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench2/repos/agent) | read-write | Rust workspace for the Miru device agent (crate `miru-agent` in `agent/`). All edits, validation, and commits happen here. |

Branch `refactor/remove-non-persistent-mode` (already checked out, created from `main` at `c0658842`), PR base `main`. The plan lives here because every change is in the agent repo. Nothing outside the agent sets `is_persistent`.

## Purpose / Big Picture

Today `settings.json` has a boolean `is_persistent` (default `true`). When it is `false` the agent exits after 60 s without local API requests (`idle_timeout`) or after 15 min (`max_runtime`). That exit also stops the MQTT worker, the poller, token refresh, and the data-upload scan, upload and delete workers, so cloud deployments and uploads stall until a local client connects to the Unix socket and systemd restarts the agent. No customer has used the mode. After this change the agent always runs until it gets a shutdown signal (SIGTERM/SIGINT/ctrl-c, or a Windows service stop), on every platform.

Observable result: an existing `settings.json` containing `"is_persistent": false` still loads, logs one startup warning, and the agent runs persistently. Unix socket activation through systemd (`build/debian/miru.socket`, `LISTEN_FDS`) is unchanged.

## Progress

- [x] M0 Activate plan (move to `plans/active/`; `docs(plans):` commit)
- [x] M1 Remove idle-exit runtime (options, `run.rs`, `platform::supports_idle_exit`, `main.rs`) and its tests
- [ ] M2 Remove the `activity` module, the request-touch middleware, and the `activity_tracker` fields
- [ ] M3 Drop `Settings.is_persistent`; add the startup warning and its tests
- [ ] M4 Docs: `ARCHITECTURE.md`, `plans/active/20260910-windows-support.md`
- [ ] M5 Preflight CLEAN (draft PR open, CI green incl. `windows-check`, `lint`, `test`); then plan moved to `plans/completed/` with Outcomes filled, and CI re-confirmed green on that commit

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- 2026-10-01: Keep `LifecycleOptions` with only `max_shutdown_delay`. Rationale: `AppOptions.lifecycle` and `ShutdownManager` (shutdown watchdog) stay unchanged; minimal churn.
- 2026-10-01: `run()` drops its returned `Arc<AppState>` binding. Rationale: `ShutdownManager::with_app_state` already holds a clone until shutdown, and every worker and server holds its own clones, so nothing depends on the local binding.
- 2026-10-01: Remove the `activity` module entirely. Rationale: `Tracker::touch()` is called only by the router middleware and `last_touched()` only by the idle-exit loop.
- 2026-10-01: Old settings files load unchanged: `Settings` has no `deny_unknown_fields`, so the `is_persistent` key is ignored on read and dropped when the file is next rewritten.
- 2026-10-01: The startup warning is a separate `disk::warn_if_persistence_disabled` that reads the raw JSON, called once in `main.rs::run_agent` after `await_activation` and before `reconcile_agent_version`. Rationale: settings are deserialized two or three times per start, and upgrade reconcile drops the key before `read_settings` runs.

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

All paths are relative to the repo root `/home/ben/miru/workbench2/repos/agent`; the crate lives in `agent/`. "Idle exit" or "non-persistent mode" means the agent process exits on its own when idle; "persistent" means it runs until signalled. "Socket activation" means systemd owns the Unix socket (`build/debian/miru.socket`, mode 0660, group `miru`), starts `miru.service` when a client connects, and hands the socket to the agent via the `LISTEN_FDS` environment variable (`acquire_unix_socket_listener` in `agent/src/server/unix.rs`). Socket activation stays: the persistent install uses it for socket permissions and to keep the socket across restarts. Do not touch `build/debian/miru.socket`, `build/debian/miru.service` (`Requires=miru.socket`), or `LISTEN_FDS` handling.

Lifecycle options, `agent/src/app/options.rs`: `LifecycleOptions { is_persistent, max_runtime (15 min), idle_timeout (60 s), idle_timeout_poll_interval (5 s), max_shutdown_delay (15 s) }` (L14-33) and `resolve_persistence(requested, supports_idle_exit)` (L35-45), which warns that non-persistent runtime is unsupported on Windows. `use tracing::warn` (L12) exists only for it. `max_shutdown_delay` is used by `ShutdownManager` in `agent/src/app/run.rs` (field L522, `new` L539, watchdog L638-649 that calls `std::process::exit(1)`) and stays.

Runtime, `agent/src/app/run.rs`: `run()` (L32-85) builds a `ShutdownManager`, awaits `init()` to completion, then, when `!options.lifecycle.is_persistent`, `tokio::select!`s the shutdown signal against `await_idle_timeout` (L87-107, reads `app_state.activity_tracker`) and `await_max_runtime` (L109-112); otherwise it awaits only the shutdown signal. `init()` (L115-135) returns `Result<Arc<AppState>, ServerErr>`. `init_local_api_servers` passes `app_state.activity_tracker.clone()` to `server::State::new` (L435). `init_tcp_server` warns about idle exit when non-persistent (L474-479). Imports L5 `std::time::{Duration, SystemTime}` and L8 `crate::activity` are used only by the idle-exit code. The inline `mod tests` (L788+) uses `LifecycleOptions::default()` and is unaffected.

Platform, `agent/src/platform/mod.rs`: `supports_idle_exit()` (doc + fn L71-82; Unix `true`, Windows `false`) and the module doc (L1-6) mentioning it and "capability dispatch". Its only caller is `agent/src/main.rs`.

Entry point, `agent/src/main.rs`: imports `options::{AppOptions, LifecycleOptions}` (L10-13) and `miru_agent::platform` (L21, used only for `supports_idle_exit`). `build_app_options` (L327-359) resolves persistence (L335-343) and sets `lifecycle`. `run_agent` (L245-289) runs `logs::init` → `await_activation` → `reconcile_agent_version` (reads settings via `get_bootstrap_backend_host`, then `upgrade::reconcile_impl` in `agent/src/app/upgrade.rs` L133-149 reads them again and, on a version change, `disk::setup::reset` rewrites `settings.json`) → `read_settings` → `build_app_options` → `run`. `main.rs` is not coverage-gated.

Activity tracker, `agent/src/activity/mod.rs` (+ `agent/src/activity/.covgate` 86.95): `Tracker { last_activity: Arc<AtomicU64> }` with `new`, `last_touched`, `touch`. Uses: `agent/src/lib.rs` L1 `pub mod activity;`; `agent/src/app/state.rs` L6 import, L38 `AppState.activity_tracker`, L85-86 construction, L108 field init; `agent/src/server/state.rs` L5 import, L21 field, L32 the fifth of seven `State::new` parameters, L41; `agent/src/server/routes.rs` `router()` (L24-26, `middleware(table(state.clone()), state)`) and `middleware()` (L28-52: doc comment "Authorization redaction, activity tracking, and request tracing..." and a `from_fn` closure L34-40 that calls `touch()`; `state` is used only by that closure). Other "activity" hits in the crate (`activity_status`, `DplActivity`, `new_activity`) are unrelated deployment state.

Settings, `agent/src/disk/settings.rs`: `Settings` derives `Serialize`; `is_persistent` at L16, default `true` at L32; custom `Deserialize` through private `DeserializeSettings` (L47 `is_persistent: Option<bool>`, L80-82 `deserialize_warn!`). No `deny_unknown_fields` in `agent/src`. `agent/src/disk/mod.rs` re-exports `pub use self::settings::{Backend, MQTTBroker, Settings, TCPServer};`. `files::read_json::<T>(&filesys::File)` (`agent/src/filesys/files.rs` L103) returns `Err` for a missing file or bad JSON without logging.

Tests (integration tests compile into one target `mod`, `agent/tests/mod.rs`, mirroring `src/`):

- `agent/tests/app/run.rs`: constants `HANG_GUARD` 60 s, `NEVER` 3600 s, `SHUTDOWN_WATCHDOG` 300 s; helpers `options(layout, lifecycle, tcp_port)`, `exits_after_max_runtime()` L80-89, `exits_when_idle()` L91-100, `persistent()` L102-110, `run_to_exit` L112-121 (still used by `invalid_app_state_initialization`), `RunningAgent`/`spawn_run` L123-135, `RunningAgent::stop` L137-146 (`self.shutdown.send(()).unwrap()` panics if `run` already returned, hiding its `Err`), `write_stale_discovery_file` L148, `wait_for_file` L154. Tests: `max_runtime_reached` L190-197, `tcp_port_in_use_does_not_abort_startup` L199-213, `discovery_write_failure_does_not_abort_startup` L215-234, `tcp_requires_bearer_and_cleans_up_discovery_file` L236-259 (uses `persistent()`), `stale_discovery_file_removed_when_tcp_disabled` L261-272, `is_persistent` L274-296, `idle_timeout_reached` L298-305, `shutdown_signal_received` L307-318 (uses `persistent()`). All but `invalid_app_state_initialization` are `#[serial]`; all run on Windows in CI. The discovery file path is `layout.device_api()`.
- `agent/tests/app/options.rs`: `mod lifecycle_options_default` (L7-46; only `max_shutdown_delay_is_15_seconds` survives) and `mod lifecycle_options_resolve_persistence` (L48-70, delete).
- `agent/tests/platform/mod.rs` `mod dispatch`: `supports_idle_exit_on_unix` / `does_not_support_idle_exit_on_windows` (L102-112).
- `agent/tests/activity/mod.rs` and `agent/tests/mod.rs` L1 `pub mod activity;`.
- `agent/tests/app/state.rs`: `// check last activity` asserts at L167-169 (in `success_missing_device_file_but_valid_token`) and L195-197 (in `success_missing_token_file`); each test also has a `let begin_test = Utc::now().timestamp();` (L151, L185) used only by those asserts, and `state` becomes unused.
- `server::State` fixtures: `agent/tests/server/handlers.rs` (L62 `use miru_agent::activity;` inside a module, L97, L121), `agent/tests/server/sse.rs` (L11, L45, L60), `agent/tests/server/tcp.rs` (L14, L53).
- `agent/tests/disk/settings.rs`: `is_persistent` at L14, L43, L54. Log-capture pattern: `mod redaction` in `agent/tests/server/tcp.rs` (L327-367) uses `crate::logs::CapturingWriter` (from `agent/tests/logs/mod.rs`), `tracing_subscriber::fmt().with_ansi(false).with_writer(...)`, and `tracing::subscriber::set_default` inside a default current-thread `#[tokio::test]`.

Tooling: `./scripts/lint.sh` runs the import-group linter (`// standard crates` / `// internal crates` / `// external crates`), `cargo fmt`, machete, clippy (`-D warnings`), a ≤50-line function-body limit for production code, and a ban on runs of ≥4 field-by-field `assert_eq!` in one test. Coverage gates (`./scripts/covgate.sh`, region coverage per directory with a `.covgate` file, run by CI's `test` job): app 90.38, server 87, disk 96.79, platform 95.00, windows 90.00, shutdown 100.00. CI `.github/workflows/ci.yml` jobs: `lint`, `windows-check` (full test suite on Windows), `windows_package_scope`, `windows-package`, `test`, `tools`. `libs/` is generated; never commit changes there.

Conventions: signed Conventional Commits (`git commit -S`), every message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; local tests use `RUST_LOG=off`; format with `cargo fmt -p miru-agent`, never `cargo fmt --all`; check `git status --short` before each commit and stage paths explicitly (never `libs/`); no git worktrees; code comments are concise and present tense, without history.

## Plan of Work

**M1 — remove the idle-exit runtime.** In `agent/src/app/options.rs` reduce `LifecycleOptions` to `pub max_shutdown_delay: Duration` (default 15 s), delete `resolve_persistence` and `use tracing::warn` (and the then-empty `// external crates` group). In `agent/src/app/run.rs`: replace the `match init(...)` binding and the whole `if !options.lifecycle.is_persistent { ... } else { ... }` block with

    if let Err(e) = init(&options, shutdown_tx.clone(), &mut shutdown_manager).await {
        error!("Failed to start server: {}", e);
        shutdown_manager.shutdown().await?;
        return Err(e);
    }

    shutdown_signal.await;
    info!("Shutdown signal received, shutting down...");

and change `init` to return `Result<(), ServerErr>` (`init_app_state(...).await?` stays bound as `app_state` for the calls below; end with `Ok(())`). Delete `await_idle_timeout`, `await_max_runtime`, the `is_persistent` warning block in `init_tcp_server`, and the now-unused imports `std::time::{Duration, SystemTime}` and `crate::activity` (the `app_state.activity_tracker.clone()` argument at L435 still compiles until M2). In `agent/src/platform/mod.rs` delete `supports_idle_exit` and rewrite the module doc to "Per-OS filesystem path defaults." plus the existing sentence about OS-specific functions and dispatchers, without the `supports_idle_exit` sentence. In `agent/src/main.rs` drop the `resolve_persistence` call and the `lifecycle:` field from `build_app_options` (`..Default::default()` supplies it), change the import to `options::AppOptions`, and remove `use miru_agent::platform;`.

Tests in M1. `agent/tests/app/run.rs`: delete `NEVER` (and its comment), `exits_after_max_runtime`, `exits_when_idle`, `max_runtime_reached`, `is_persistent`, `idle_timeout_reached`. Rename `persistent()` to `lifecycle()` returning `LifecycleOptions { max_shutdown_delay: SHUTDOWN_WATCHDOG }` (no `..Default::default()`: with one field clippy's `needless_update` fires) with doc `/// Lifecycle options whose shutdown watchdog outlasts HANG_GUARD.`; update its two callers. In `shutdown_signal_received`, after its sleep, add `assert!(!agent.handle.is_finished(), "run exited before the shutdown signal");` (replaces the deleted `is_persistent` test's guard against a self-exit). In `RunningAgent::stop` replace `self.shutdown.send(()).unwrap();` with `let _ = self.shutdown.send(());` so a run that already returned surfaces its own `Err`. Convert `tcp_port_in_use_does_not_abort_startup`, `discovery_write_failure_does_not_abort_startup`, and `stale_discovery_file_removed_when_tcp_disabled` from `run_to_exit(options(&layout, exits_after_max_runtime(), X)).await.unwrap();` to

    let agent = spawn_run(options(&layout, lifecycle(), X));
    agent.stop().await.unwrap();

keeping their setup and post-run assertions (no TCP server registers a discovery file, so `!layout.device_api().exists()` still proves startup removed the stale one); keep `_taken` alive across `stop()` in the port test. Replace the comment `// the bind fails, the agent keeps running, and max_runtime ends the run` (L206) with `// the bind fails and the agent keeps running`, and the two-line `// the tcp server fails to start, the agent keeps running, and` / `// max_runtime ends the run` (L229-230) with `// the tcp server fails to start and the agent keeps running`. Add a second doc line to `RunningAgent::stop`: `/// run() polls the signal only after init() finishes and the oneshot buffers it, so Ok means startup completed.` `agent/tests/app/options.rs`: delete the four removed-field tests and `mod lifecycle_options_resolve_persistence`. `agent/tests/platform/mod.rs`: delete the two `supports_idle_exit` tests.

**M2 — remove the activity tracker.** Delete `agent/src/activity/` (both `mod.rs` and `.covgate`), `pub mod activity;` in `agent/src/lib.rs`, `agent/tests/activity/`, and `pub mod activity;` in `agent/tests/mod.rs`. `agent/src/app/state.rs`: remove the import, the `activity_tracker` field, its construction (L85-86) and field init. `agent/src/server/state.rs`: remove the import, field, `State::new` parameter, and field init (`State::new` takes six arguments). `agent/src/app/run.rs`: drop the `app_state.activity_tracker.clone()` argument. `agent/src/server/routes.rs`: remove the touch `from_fn` layer; `middleware(router: Router) -> Router` loses its `state` parameter; `router()` becomes `middleware(table(state))`; doc comment becomes "Authorization redaction and request tracing applied to every transport. Redaction runs outermost so trace spans never log a token." Update fixtures in `agent/tests/server/handlers.rs`, `sse.rs`, `tcp.rs` (drop the import, the `activity_tracker` local, and the argument). In `agent/tests/app/state.rs` delete both `// check last activity` blocks and the two `begin_test` lines, and bind the now-unused state as `let (_state, _) = env.init().await.unwrap();` (a named binding, not `_`, so the state and its storage actors live until the test's later reads; `Utc` is still used elsewhere in the file).

**M3 — drop the setting and warn on it.** In `agent/src/disk/settings.rs` remove `is_persistent` from `Settings`, its `Default`, `DeserializeSettings`, and the `deserialize_warn!` call. Add `warn_if_persistence_disabled` (signature under Interfaces): read the file with `files::read_json::<serde_json::Value>`; on `Err` return `false`; if `value.get("is_persistent").and_then(serde_json::Value::as_bool) == Some(false)` log `warn!("settings.is_persistent is no longer supported; the agent always runs persistently")` and return `true`; otherwise return `false`. It never fails and logs nothing else (`read_settings` reports unreadable files). Imports: `use crate::filesys::{self, files};` (internal) and `use tracing::{error, warn};` (external). Re-export it from `agent/src/disk/mod.rs` (`pub use self::settings::{warn_if_persistence_disabled, Backend, MQTTBroker, Settings, TCPServer};`). In `agent/src/main.rs::run_agent`, right after the `await_activation` match and before `reconcile_agent_version`, add

    // warn before upgrade reconcile rewrites settings.json without the key
    disk::warn_if_persistence_disabled(&layout.settings()).await;

In `agent/tests/disk/settings.rs` remove `is_persistent` from the two `Settings` literals and the `json!` input, and add the tests listed under Validation (M3).

**M4 — docs.** `ARCHITECTURE.md`: L32 `platform` entry becomes "per-OS defaults (data root, log dir)", dropping capability dispatch; L46 replace the last-but-one sentence ("Only the Unix socket is socket-activated, so in non-persistent mode ...") with "On Linux, systemd's `miru.socket` unit owns the Unix socket and hands it to the agent through socket activation (`LISTEN_FDS`); the agent binds the TCP listener itself."; delete the L72 `activity` entry; replace the L103 bullet with two bullets: "**The agent always runs persistently.** It exits only on a shutdown signal or a fatal startup error. A `settings.json` with `is_persistent: false` loads and logs a startup warning." and "**Windows has no Unix socket.** On Windows the local device API is the TCP listener, which is on by default (`enable_tcp_server`). Startup warns and nothing listens when that flag is turned off." `plans/active/20260910-windows-support.md`: in the inventory row (L26) end the last cell at "no activation equivalent" (drop "— persistent only"); rewrite Decision 3 (L55-56) as "**All platforms run persistently.** The agent has no idle-exit mode (see the 2026-10-01 decision). Socket activation stays Linux-only and covers only the Unix socket."; Non-goals (L363) becomes "Named-pipe transport or socket activation on Windows"; leave dated decision-log entries unchanged (the 2026-09-30 entry at L221-223 mentioning the activity tracker is historical); append to `## Decision log` (after the last 2026-10-01 entry, before `## Risks`):

    - 2026-10-01: Removed the non-persistent (idle-exit) mode and
      `settings.is_persistent`; the agent always runs persistently, and the
      idle activity tracker is gone. Rationale: no customer used it, and idle
      exit stopped MQTT, polling, token refresh and upload workers, so cloud
      deployments and uploads stalled until a local client woke the agent. A
      settings file with `is_persistent: false` loads with a startup warning.

The roadmap has no PR entry for this change (it is not Windows work), so none is added. `plans/completed/*` are historical and not edited.

**M5 — validation and completion.** Run preflight until it reports `CLEAN`, then complete the plan (Concrete Steps M5).

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent`. Every commit uses `git commit -S -m "<subject>" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"` (written below as `commit "<subject>"`). Before each commit tick the milestone in Progress and run `git status --short` (nothing under `libs/` may be staged). `./scripts/lint.sh` runs `cargo fmt -p miru-agent` and rewrites formatting in place, so run it before staging.

M0:

    mv plans/backlog/20261001-remove-non-persistent-mode.md plans/active/
    git add plans/active/20261001-remove-non-persistent-mode.md
    commit "docs(plans): activate remove non-persistent mode plan"

M1:

    RUST_LOG=off cargo test --package miru-agent --test mod app::
    RUST_LOG=off cargo test --package miru-agent --test mod platform::
    ./scripts/lint.sh
    git add agent/ plans/active/20261001-remove-non-persistent-mode.md && commit "refactor(app): remove idle-exit lifecycle mode"

Expected: both report `test result: ok.`; `app::run` lists `invalid_app_state_initialization`, `tcp_port_in_use_does_not_abort_startup`, `discovery_write_failure_does_not_abort_startup`, `tcp_requires_bearer_and_cleans_up_discovery_file`, `stale_discovery_file_removed_when_tcp_disabled`, `shutdown_signal_received` and no `max_runtime_reached`, `is_persistent`, `idle_timeout_reached`; `app::options` lists only `max_shutdown_delay_is_15_seconds` plus `app_options_default::*`.

M2:

    RUST_LOG=off cargo test --package miru-agent --test mod server::
    RUST_LOG=off cargo test --package miru-agent --test mod app::
    ./scripts/lint.sh
    git add agent/ plans/active/20261001-remove-non-persistent-mode.md && commit "refactor(server): remove activity tracker"

Expected: `test result: ok.` for both, including `server::tcp::redaction::authorization_header_is_redacted_in_trace_spans` (redaction still runs outermost).

M3:

    RUST_LOG=off cargo test --package miru-agent --test mod disk::settings
    RUST_LOG=off cargo test --package miru-agent --test mod app::upgrade
    ./scripts/lint.sh
    grep -rnE "is_persistent|idle_timeout|max_runtime|supports_idle_exit|resolve_persistence|activity_tracker|activity::" agent/src agent/tests
    git add agent/ plans/active/20261001-remove-non-persistent-mode.md && commit "refactor(disk): drop is_persistent setting"

Expected: tests `ok`, including the new tests under Validation. The grep prints only `agent/src/disk/settings.rs` (the warning function), `agent/tests/disk/settings.rs` (its tests), and `agent/src/gcs/store.rs` `idle_timeout_err` (an unrelated GCS download timeout).

M4:

    grep -nE "is_persistent|idle|supports_idle_exit|activity" ARCHITECTURE.md
    ./scripts/lint.sh
    git add ARCHITECTURE.md plans/active/ && commit "docs: remove non-persistent mode from architecture and windows plan"

Expected: the grep prints only the new "always runs persistently" invariant line.

M5: run the `preflight` skill. It pushes the branch, opens a draft PR against `main` if none exists, watches CI (`lint`, `test` with coverage gates, `tools`, `windows-check`, Windows packaging), fixes failures from CI logs in follow-up commits (not amends), and must report `CLEAN`. By hand:

    RUST_LOG=off cargo test --package miru-agent
    git push -u origin refactor/remove-non-persistent-mode
    gh pr create --draft --base main --title "refactor(app): remove non-persistent mode" --body-file <body.md>
    gh pr checks --watch

The PR body summarizes the change and ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. If a coverage gate fails, add tests; never lower a `.covgate`. Once CI is green, tick M5 with the green SHA, fill Surprises and Outcomes, then:

    git mv plans/active/20261001-remove-non-persistent-mode.md plans/completed/
    git add plans/completed/20261001-remove-non-persistent-mode.md
    commit "docs(plans): complete remove non-persistent mode plan"
    git push
    gh pr checks --watch                  # must be green on this head
    gh pr ready                           # only once CI is green on the final head

## Validation and Acceptance

**Preflight must report CLEAN (CI green on the pushed branch head, including `windows-check`, `lint`, and `test` with coverage gates) before the PR leaves draft or the task is reported complete.**

Behavior, all through `RUST_LOG=off cargo test --package miru-agent` (full suite passes on Linux and in `windows-check`):

- Default settings run until signalled: `app::run::shutdown_signal_received` asserts the run task is still running before the test sends the shutdown signal, then that `stop()` returns `Ok`.
- Startup survives optional-transport failures: `tcp_port_in_use_does_not_abort_startup`, `discovery_write_failure_does_not_abort_startup`, and `stale_discovery_file_removed_when_tcp_disabled` (spawn, `stop()`, `Ok`, stale discovery file gone) pass deterministically with no sleeps.
- An old settings file keeps working. New tests in `agent/tests/disk/settings.rs`, in a `pub mod persistence_warning` with a helper that writes given contents to `settings.json` in a `crate::test_utils::filesys::dirs::temp` dir:
  - `deserialize_ignores_is_persistent`: `json!({"is_persistent": false, "enable_poller": false})` deserializes to `Settings { enable_poller: false, ..Settings::default() }`.
  - `serialize_omits_is_persistent`: `serde_json::to_value(Settings::default())` has no `is_persistent` key.
  - `warns_when_disabled` (`{"is_persistent": false}` → `true`), `silent_when_enabled` (`true` → `false`), `silent_when_absent` (`{}` → `false`), `silent_when_not_bool` (`"false"` string → `false`), `silent_when_file_missing` (→ `false`), `silent_when_invalid_json` (`not json` → `false`).
  - `logs_warning_once` (`#[tokio::test]`, current-thread): install a `tracing_subscriber::fmt().with_ansi(false).with_writer(CapturingWriter(buf.clone())).finish()` via `tracing::subscriber::set_default`, call the function on a `false` file, and assert the captured text contains `settings.is_persistent is no longer supported` exactly once (`matches(..).count() == 1`).
- No leftovers: the M3 and M4 greps print only the expected lines.

Manual check (optional, needs an activated Linux device): put `"is_persistent": false` in `/var/lib/miru/settings.json`, install this build, `sudo systemctl restart miru`; `journalctl -u miru` shows the warning once and the agent is still running after 15 minutes with no local API traffic.

## Idempotence and Recovery

All edits are source, test, and doc changes; re-running tests, `cargo fmt -p miru-agent`, and `./scripts/lint.sh` is safe. Each milestone compiles on its own (M1 leaves the activity tracker in place), so a failed milestone can be fixed forward with another commit. At runtime nothing migrates; the warning never fails startup. Rollback is reverting the PR; an older agent reading a settings file without `is_persistent` defaults it to `true`.

## Interfaces and Dependencies

No new crates (`serde_json` and `tracing` are already dependencies). Changed or new signatures:

    // agent/src/app/options.rs
    #[derive(Debug, Clone, Copy)]
    pub struct LifecycleOptions {
        pub max_shutdown_delay: Duration,   // default 15 s
    }

    // agent/src/disk/settings.rs (re-exported as disk::warn_if_persistence_disabled)
    /// Logs a warning when `file` sets `is_persistent` to `false`, which the
    /// agent ignores. Returns whether it warned; never fails.
    pub async fn warn_if_persistence_disabled(file: &filesys::File) -> bool;

    // agent/src/server/state.rs
    pub fn new(storage, http_client, syncer, token_mngr, event_hub, shutdown_tx) -> State;

Removed: `LifecycleOptions::resolve_persistence`, `platform::supports_idle_exit`, `miru_agent::activity` (`Tracker`), `AppState.activity_tracker`, `server::State.activity_tracker`, `Settings.is_persistent`.
