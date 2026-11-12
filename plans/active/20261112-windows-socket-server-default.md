# Make the Unix socket server explicitly unix-only on Windows

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Socket-server default, Windows startup warning, unix-only `socket_file`, tests, docs |

This plan lives in `agent/plans/` because all changes are in the agent repo. Paths below are relative to the agent repo root (the directory holding `Cargo.toml` and `ARCHITECTURE.md`); the crate is in `agent/`.

## Purpose / Big Picture

On Windows the settings `enable_socket_server` and the server option `socket_file` silently do nothing, because the Unix-socket server only exists on Unix. Today provisioning writes `enable_socket_server: true` into every Windows device's `settings.json`, which suggests a socket is served when none is.

After this change: a newly provisioned Windows device gets `"enable_socket_server": false` in `C:\ProgramData\Miru\settings.json` (Linux devices still get `true`); a Windows agent whose settings say `"enable_socket_server": true` logs one startup warning that the Unix socket isn't supported on Windows and the local device API is served over TCP only, then keeps running; and Windows builds no longer carry a socket path in `server::Options`.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `server::DEFAULT_ENABLE_SOCKET_SERVER` | `DEFAULT_ENABLE_UNIX_SERVER`, `DEFAULT_ENABLE_SOCKET` | Pairs with sibling `DEFAULT_ENABLE_TCP_SERVER` and names the setting it defaults (`enable_socket_server`); "unix server" appears nowhere else as a name. No existing item uses this name. |

### Precedent

No new files. The constant mirrors `DEFAULT_ENABLE_TCP_SERVER` in `agent/src/server/mod.rs`. The warning mirrors the `#[cfg(windows)]` TCP-off warning in `init_local_api_servers` (`agent/src/app/run.rs`). The `#[cfg(unix)]` import gate mirrors `#[cfg(unix)] use crate::server::unix::serve;` in `agent/src/app/run.rs`. The new settings test mirrors `tcp_server_enabled_by_default_only_on_windows` in `agent/tests/disk/settings.rs`; the changed options test mirrors `tcp_server_enabled_only_on_windows` in `agent/tests/app/options.rs`. Both siblings assert the literal `cfg!(windows)` rather than the constant, so the new assertions use `cfg!(unix)`.

### Left out

- A test for the new Windows warning: its sibling, the TCP-off warning, has no test, app-run tests have no log-capture harness, and behavior (the agent keeps running) is unchanged.
- Gating `enable_socket_server` itself with `cfg(unix)`: the settings file has one schema on every OS, and the Windows warning reads the field.
- Gating `server::errors::BindUnixSocketErr`: it is an error type, not an option, and the goal does not name it.
- Rewriting `settings.json` on existing Windows devices, and named-pipe or `AF_UNIX` transports on Windows: out of scope (the latter is a non-goal of `plans/active/20260910-windows-support.md`).

### Open questions

- Existing Windows devices keep `"enable_socket_server": true` (upgrade preserves `settings.json`), so they log the new warning on every start until an operator sets it to `false`. Is that acceptable? Default: yes; the warning names the setting and the fix is one edit.
- Should `plans/active/20260910-windows-support.md` change? It never states the `enable_socket_server` default (line 13's `enable_socket_server: false` describes the Phase 1 manual setting). Default: leave its prose alone and append one dated entry to its `## Decision log` recording this change.

## Progress

- [x] Source written
- [ ] M1: source changes (constant, defaults, warning, unix-only `socket_file`, its test-fixture use).
- [ ] M2: tests assert the platform default.
- [ ] M3: docs (`ARCHITECTURE.md`, Windows support plan).
- [ ] CI green on the pushed head (`$preflight` reports `CLEAN`).

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- 2026-11-12: The Windows warning names `enable_socket_server`, so an operator who sees it knows which setting to turn off.
- 2026-11-12: The warning no longer says the API is served over TCP, because it runs before the TCP-off check and would contradict it when TCP is off too (design review).

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The agent serves a local HTTP "device API" to on-device programs over two transports, both started by `init_local_api_servers` in `agent/src/app/run.rs`:

- A Unix domain socket (`/run/miru/miru.sock`), served by `agent/src/server/unix.rs`, which is compiled only on Unix (`#[cfg(unix)] pub mod unix;` in `agent/src/server/mod.rs`). `init_socket_server` in `agent/src/app/run.rs` is also `#[cfg(unix)]`, and its call in `init_local_api_servers` is wrapped in `#[cfg(unix)] if options.enable_socket_server { ... }`.
- Loopback TCP (`agent/src/server/tcp.rs`), on all platforms, on by default only on Windows via `pub const DEFAULT_ENABLE_TCP_SERVER: bool = cfg!(windows);` in `agent/src/server/mod.rs`.

`cfg!(unix)` / `cfg!(windows)` are compile-time booleans; `#[cfg(unix)]` on an item, field, statement or import removes it from non-Unix builds.

Settings flow: `agent/src/disk/settings.rs` defines `Settings` (serialized as `settings.json`) with `enable_socket_server: bool` and `enable_tcp_server: bool`; `impl Default for Settings` sets `enable_socket_server: true` and `enable_tcp_server: DEFAULT_ENABLE_TCP_SERVER` (imported from `crate::server`). Provisioning (`agent/src/provisioning/`) writes `Settings::default()`-based settings to disk; upgrade (`agent/src/app/upgrade.rs`) keeps the existing file. At runtime `build_app_options` in `agent/src/main.rs` copies the settings into `AppOptions` (`agent/src/app/options.rs`), whose `impl Default for AppOptions` also sets `enable_socket_server: true` and `enable_tcp_server: server::DEFAULT_ENABLE_TCP_SERVER`. `build_app_options` builds `server::Options { tcp_port: ..., ..Default::default() }` and never names `socket_file`.

`server::Options` (`agent/src/server/mod.rs`) has `socket_file: filesys::File` (doc "Unix socket path (unix only).") and `tcp_port: u16`; its `Default` sets `socket_file: filesys::File::new("/run/miru/miru.sock")`. The file's only import, `use crate::filesys;`, exists for that field. The only reader of `socket_file` is `init_socket_server` (unix only). The only other constructor naming it is the `options` test helper in `agent/tests/app/run.rs` (line ~60), which sets `socket_file: filesys::File::new(PathBuf::from("/tmp").join("miru.sock"))`; `PathBuf` (imported at line 2) is used nowhere else in that file.

In `init_local_api_servers` today, Windows has one warning: `#[cfg(windows)] if !options.enable_tcp_server { tracing::warn!("tcp server is disabled; the local device API has no transport on windows"); return Ok(()); }`. An earlier warn-and-skip for `enable_socket_server` on Windows (2026-09-11 decision in `plans/active/20260911-windows-cfg-gates.md`: warn, never error, so a config toggle can't brick a fleet) was dropped when the TCP transport landed, so the setting is now silently ignored. `init_optional_services` calls `init_local_api_servers` only when either flag is set.

Tests: `agent/tests/app/options.rs` has `socket_server_enabled` (`assert!(AppOptions::default().enable_socket_server)`) next to `tcp_server_enabled_only_on_windows`; `agent/tests/disk/settings.rs` has `tcp_server_enabled_by_default_only_on_windows` and no socket-default test. CI (`.github/workflows/ci.yml`) runs the suite on Linux (`test` job, with coverage gates) and on Windows (`windows-check` job, `cargo test --package miru-agent`), and clippy/fmt/import lint on Linux (`lint` job).

## Plan of Work

M1 — source. In `agent/src/server/mod.rs`, add after `DEFAULT_ENABLE_TCP_SERVER`:

    /// Whether the Unix socket server is on when settings don't say. Windows
    /// has no Unix socket, so it is off there.
    pub const DEFAULT_ENABLE_SOCKET_SERVER: bool = cfg!(unix);

In the same file, put `#[cfg(unix)]` on `use crate::filesys;`, on the `socket_file` field (doc becomes `/// Unix socket path.`), and on the `socket_file: ...` line in `impl Default for Options`. In `agent/src/disk/settings.rs`, import `DEFAULT_ENABLE_SOCKET_SERVER` alongside the other `crate::server` constants, set `enable_socket_server: DEFAULT_ENABLE_SOCKET_SERVER` in `Settings::default`, and add a field doc mirroring the `enable_tcp_server` one:

    /// Unix socket listener for the local device API. On by default except on
    /// Windows, which has no Unix socket (see `DEFAULT_ENABLE_SOCKET_SERVER`).

In `agent/src/app/options.rs`, set `enable_socket_server: server::DEFAULT_ENABLE_SOCKET_SERVER` in `AppOptions::default`. In `agent/src/app/run.rs` `init_local_api_servers`, insert before the existing `#[cfg(windows)]` TCP-off block, so it fires even when TCP is off:

    #[cfg(windows)]
    if options.enable_socket_server {
        tracing::warn!(
            "enable_socket_server is ignored on windows, which has no unix socket"
        );
    }

In `agent/tests/app/run.rs` `options`, put `#[cfg(unix)]` on the `socket_file` line of the `Options { ... }` literal and write it as `filesys::File::new("/tmp/miru.sock")` (`File::new` normalizes `/`-rooted literals), then delete the now-unused `use std::path::PathBuf;` and its `// standard crates` header (it is the group's only import). This is a use of the gated field, so it lands with M1 to keep every commit compiling on Windows.

M2 — tests. In `agent/tests/app/options.rs`, rename `socket_server_enabled` to `socket_server_enabled_only_on_unix` and assert `assert_eq!(AppOptions::default().enable_socket_server, cfg!(unix));`. In `agent/tests/disk/settings.rs`, add directly before `tcp_server_enabled_by_default_only_on_windows` (field order):

    #[test]
    fn socket_server_enabled_by_default_only_on_unix() {
        assert_eq!(Settings::default().enable_socket_server, cfg!(unix));
    }

Between M1 and M2 the old `socket_server_enabled` would fail on Windows only; CI runs on the pushed head, which includes both.

M3 — docs. In `ARCHITECTURE.md` line 46 (the `server` paragraph), change "a Unix socket (`/run/miru/miru.sock`, `server/unix.rs`, unix only)" to "a Unix socket (`/run/miru/miru.sock`, `server/unix.rs`, unix only; on by default via `settings.enable_socket_server`)". Replace the line 110 invariant with: "**Windows has no Unix socket.** On Windows the local device API is the TCP listener, which is on by default (`enable_tcp_server`), and `enable_socket_server` is off by default. Startup warns when `enable_socket_server` is set, and warns and nothing listens when `enable_tcp_server` is turned off." In `plans/active/20260910-windows-support.md`, append to `## Decision log` (after the last 2026-10-01 entry, before `## Risks`), in that file's format: "- 2026-11-12: `enable_socket_server` defaults to off on Windows (`DEFAULT_ENABLE_SOCKET_SERVER = cfg!(unix)`), startup warns when it is set there, and `server::Options::socket_file` is unix-only. Rationale: the setting had no effect on Windows but provisioning wrote `true`, so the file misstated what the agent serves; a warning alone would fire on every Windows start. Existing `settings.json` files are not rewritten."

## Concrete Steps

All commands run from the agent repo root on branch `fix/windows-socket-settings`. No local build, lint or test commands.

1. Make the M1 edits, then:

        git add agent/src/server/mod.rs agent/src/disk/settings.rs agent/src/app/options.rs agent/src/app/run.rs agent/tests/app/run.rs
        git commit -m "fix(server): default the unix socket server off on windows and warn when set"

2. Make the M2 edits, then:

        git add agent/tests/app/options.rs agent/tests/disk/settings.rs
        git commit -m "test(settings): assert the platform default for enable_socket_server"

3. Make the M3 edits, then:

        git add ARCHITECTURE.md plans/active/20260910-windows-support.md
        git commit -m "docs: document the windows socket server default and warning"

4. Check nothing else names the field: `grep -rn "socket_file" agent/src agent/tests` lists only `server/mod.rs`, `server/unix.rs`, `server/errors.rs`, `app/run.rs` (inside `init_socket_server`) and the gated line in `agent/tests/app/run.rs`.

5. Validate with `$preflight` (push and watch CI).

## Validation and Acceptance

CI workflow `ci.yml` on the pushed head: job `windows-check` passes, which proves Windows compiles without `socket_file` and that `socket_server_enabled_only_on_unix` and `socket_server_enabled_by_default_only_on_unix` see `false`; jobs `test` (Linux, coverage gates) and `lint` (clippy `-D warnings`, fmt, import linter) pass, proving Linux behavior and defaults (`true`) are unchanged. The `agent/tests/app/run.rs` tests pass on both OSes with the gated fixture. Validation is complete when `$preflight` reports `CLEAN`.

Observable behavior on a Windows device: provisioning writes `"enable_socket_server": false` to `C:\ProgramData\Miru\settings.json`. Setting it to `true` and restarting the service logs `enable_socket_server is ignored on windows, which has no unix socket` at warn level, and the agent keeps serving the device API on `127.0.0.1:6478`. On Linux, provisioning still writes `"enable_socket_server": true` and `/run/miru/miru.sock` is served as before.

## Idempotence and Recovery

All steps are file edits and commits; re-applying an edit that is already present changes nothing. To undo a milestone, `git revert <sha>` from the agent repo root. No data migration: existing `settings.json` files are read as before.
