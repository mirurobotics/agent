# Compile the Unix-socket plumbing out of Windows builds

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Add `#[cfg(unix)]` to the socket path option, the socket bind error, the shutdown manager's socket server handle, and the tests that use them. |

This plan lives in `agent/plans/` because every edit is in the agent repo. Work on branch `refactor/windows-unix-socket-gates` (base: `main` at e9f2c46).

## Purpose / Big Picture

The agent serves its local device API over a Unix socket on Linux and over loopback TCP on Windows. The socket server itself, `agent/src/server/unix.rs`, is already compiled only on Unix (`#[cfg(unix)]`), but the plumbing around it still compiles on Windows: a socket path option, a socket bind error and a shutdown slot for the socket server's task, none of which Windows can ever set or raise. After this change a Windows build carries none of them, so a reader of the Windows build sees only the transport it has. Nothing changes at runtime on any platform: Linux keeps the same options, errors and shutdown order, and Windows never used these items. The proof is that the `windows-check` and `test` jobs in `.github/workflows/ci.yml` stay green.

## Decisions

### Precedent

No file is added. Each gate mirrors the existing `#[cfg(unix)]` gates on `pub mod unix;` in `agent/src/server/mod.rs` and on the `serve` import and `init_socket_server` in `agent/src/app/run.rs`. Gating one enum variant mirrors `PrivilegeErr::Syscall` in `agent/src/privilege/errors.rs` and `ScmErr::Scm` in `agent/src/windows/errors.rs`.

`ServerErr` implements `crate::errors::Error` through the `impl_error!` macro in `agent/src/errors/mod.rs`, which today takes bare variant names and so cannot drop a gated variant from its match arms. The macro gains an optional attribute list per variant, `$(#[$meta:meta])* $variant:ident`, which it copies onto each generated match arm. No macro in the crate takes per-item attributes yet; this is the usual `macro_rules!` form for it, and every existing caller compiles unchanged.

### Left out

- `enable_socket_server` on `AppOptions` (`agent/src/app/options.rs`) and on the settings file: the settings file keeps one schema on every OS, and the Windows build reads the flag to warn that it has no effect.
- `server::routes`: it holds only `router()`, which both transports serve, and no socket option.
- `ShutdownManager::shutdown_server` and `set_server_handle`: the TCP server handle still uses both.
- New tests: this change only removes items from Windows builds and changes no behavior, so there is no new behavior to test. The existing tests listed in Validation cover the gated code on Linux and must keep compiling on Windows.

## Progress

- [ ] Plan committed.
- [ ] Source gated and committed.
- [ ] Branch pushed; `windows-check` and `test` pass in CI.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

(Add entries as work proceeds.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

`agent/src/server/mod.rs` defines `server::Options`, whose `socket_file` field (default `/run/miru/miru.sock`) is the Unix socket path. Its only reader is `init_socket_server` in `agent/src/app/run.rs`, already `#[cfg(unix)]`. The `use crate::filesys;` import in `server/mod.rs` exists only for that field.

`agent/src/server/errors.rs` defines `BindUnixSocketErr` and the `ServerErr::BindUnixSocketErr` variant, raised only from `agent/src/server/unix.rs`, plus the variant's entry in the `crate::impl_error!(ServerErr { ... })` list.

`agent/src/app/run.rs` defines `ShutdownManager`, which holds the join handles of every task the agent stops at shutdown. Its `socket_server_handle` field is set only by `with_socket_server_handle`, called only from `init_socket_server`. That method carries `#[cfg_attr(not(unix), allow(dead_code))]` today to silence the Windows dead-code warning. `shutdown_impl` step 4 awaits the socket handle, then the TCP handle.

Tests that touch these items: the inline `#[cfg(test)] mod tests` at the bottom of `agent/src/app/run.rs` calls `with_socket_server_handle` and reads `socket_server_handle`, and the `options` helper in `agent/tests/app/run.rs` sets `socket_file`. No test names `BindUnixSocketErr`. The `windows-check` CI job runs `cargo test --package miru-agent` on Windows, so every test must compile there.

## Plan of Work

In `agent/src/server/mod.rs`, put `#[cfg(unix)]` on `use crate::filesys;`, on the `socket_file` field and on its line in `Options::default`.

In `agent/src/server/errors.rs`, put `#[cfg(unix)]` on the `BindUnixSocketErr` struct, its `crate::errors::Error` impl, the `ServerErr::BindUnixSocketErr` variant and its entry in the `impl_error!` list.

In `agent/src/errors/mod.rs`, let `impl_error!` take `$(#[$meta:meta])*` before each variant and emit those attributes before each `Self::$variant(e) => ...` arm.

In `agent/src/app/run.rs`, put `#[cfg(unix)]` on the `socket_server_handle` field, its line in `ShutdownManager::new`, and the socket statement in `shutdown_impl` step 4 (folding the `take()` into the `shutdown_server` call so one statement carries the gate). Replace the `cfg_attr(not(unix), allow(dead_code))` on `with_socket_server_handle` with `#[cfg(unix)]`. In its inline tests, gate the two socket-only tests (`shutdown_impl_returns_socket_server_inner_error`, `shutdown_impl_maps_socket_server_join_error`) whole, and gate only the socket lines in the three tests that register many handles (`shutdown_impl_runs_app_state_shutdown_after_worker_panic`, `shutdown_impl_returns_first_error`, `shutdown_impl_ok_when_all_steps_succeed`), so their other steps still run on Windows.

In `agent/tests/app/run.rs`, gate the `socket_file` line of the `options` helper with `#[cfg(unix)]`, write it `filesys::File::new("/tmp/miru.sock")`, and drop the then-unused `use std::path::PathBuf;` and its `// standard crates` header.

## Concrete Steps

From the agent repo root, commit this plan:

    git add plans/active/20261112-windows-unix-socket-gates.md
    git commit -m "docs(plans): add windows unix-socket gates plan"

Make the edits in Plan of Work, then from the agent repo root:

    git add agent/src plans/active/20261112-windows-unix-socket-gates.md agent/tests
    git commit -m "refactor(server): compile unix-socket plumbing out of windows builds"
    git push -u origin refactor/windows-unix-socket-gates

## Validation and Acceptance

Validation runs in CI on the pushed head, never locally. The `windows-check` job in `.github/workflows/ci.yml` compiles the library, binary and every test target on Windows and runs them; it passes only if no remaining code names a gated item and no import is left unused. The `test` job runs the Linux suite with coverage gates; it passes with the same tests as before, which shows Linux behavior is unchanged. The `lint` job must also stay green.

Existing tests that must keep compiling and passing on Windows with their socket lines gated: `shutdown_impl_runs_app_state_shutdown_after_worker_panic`, `shutdown_impl_returns_first_error` and `shutdown_impl_ok_when_all_steps_succeed` in `agent/src/app/run.rs`, and every test in `agent/tests/app/run.rs` through the `options` helper. On Linux, `shutdown_impl_returns_socket_server_inner_error` and `shutdown_impl_maps_socket_server_join_error` keep covering the socket handle's shutdown. Validation is complete when `$preflight` reports `CLEAN`.

## Idempotence and Recovery

Every edit adds or moves an attribute, so re-applying a step is a no-op once it is in place. If `windows-check` fails on a name this plan missed, gate that use the same way and push again; reverting the source commit restores the previous state.
