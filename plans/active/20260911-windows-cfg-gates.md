# Windows cfg-gates: compile the agent for x86_64-pc-windows-msvc

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench4/repos/agent) | read/write | Rust workspace for the Miru device agent. All edits, validation, and commits happen here. |

Branch: `feat/windows-cfg-gates` (base `main` at 56b1c86, which includes PR 2
crypt/openssl demotion #231, PR 4 platform paths #232, and the portable path
fixtures #233). This is **PR 3** of the Windows-support roadmap
(`plans/active/20260910-windows-support.md`).

## Purpose / Big Picture

Make `cargo check --target x86_64-pc-windows-msvc --package miru-agent` pass,
and enforce it in CI so Unix-isms cannot regress. Four production areas have
unconditional Unix APIs (verified inventory in the roadmap plan and the
2026-09-11 audit):

1. `main.rs` — `tokio::signal::unix` SIGTERM/SIGINT.
2. `privilege/` — `nix` geteuid/getegid + passwd lookup (the `nix` dep is
   already unix-only, so the module cannot compile on Windows).
3. `filesys/files.rs` — `std::os::unix::fs::OpenOptionsExt` `.mode()` calls.
4. `server/unix.rs` — `tokio::net::UnixListener`, systemd `LISTEN_FDS`
   fd-3 adoption (`FromRawFd`). `routes.rs` (`Options` + `routes()`) stays
   portable.

Windows counterparts in this PR are deliberately minimal (full service
lifecycle is PR 5, TCP transport is PR 11): ctrl-c shutdown, a warn-only
privilege stub, mode-bits ignored (NTFS ACLs are the installer's job, PR 8),
and no local API server (Phase 1 runs `enable_socket_server: false`).

## Progress

- [x] Activate plan (`docs(plans):` commit on the branch)
- [x] `main.rs`: cfg-split `await_shutdown_signal` (unix: SIGTERM/SIGINT/ctrl-c; windows: ctrl-c)
- [x] `privilege/`: unix impl + tests under `cfg(unix)`; windows warn-only `verify_effective_user` stub; `Syscall` error variant unix-gated (enum stays inhabited)
- [x] `filesys/files.rs`: gate `OpenOptionsExt` import + `.mode()` application (windows ignores `WriteOptions.mode`); delete unused `create_symlink`
- [x] `server/`: split portable `routes.rs` (`Options` + `routes()`) from unix-only `unix.rs` (socket + LISTEN_FDS)
- [x] `app/run.rs`: gate `serve` import, `init_socket_server`, `with_socket_server_handle`; windows branch warns and skips when `enable_socket_server` is set
- [x] `agent/tests/mod.rs`: `#[cfg(unix)]` on `privilege` test module
- [x] Windows compile validated by the CI windows-check job (local cross-check infeasible; see Surprises)
- [x] CI: windows check job/step in the Lint workflow
- [x] `./scripts/test.sh` + `./scripts/lint.sh` green on Linux (zero behavior change)
- [x] Push; CI green; PR opened

## Surprises & Discoveries

- 2026-09-11: aws-lc-sys cannot cross-compile x86_64-pc-windows-msvc from
  Linux — its build script drives the host `cc` with pthread-based sources.
  The roadmap's "cross-check on Linux runners" assumption is wrong; the CI
  check runs natively on `windows-latest` (roadmap updated in this PR).
- 2026-09-11: first CI attempt died with `startup_failure`: the org's
  Actions allowlist rejected the third-party `ilammy/setup-nasm` action.
  NASM is installed via `choco` in a plain run step instead.
- 2026-09-11: the windows-check job passed on its first real run — the
  cfg-gating surface matched the audit exactly, no hidden Unix-isms.

## Decision Log

- 2026-09-11 (authoring): `enable_socket_server` on Windows is a warn-and-skip
  at the `init_socket_server` call site, not a typed error: Phase 1 ships with
  the server disabled and PR 11 replaces the branch with the TCP transport;
  failing the whole agent for a setting that has no Windows meaning yet would
  brick a fleet on a config toggle. (`serve()` itself is `cfg(unix)` — there is
  no Windows stub to call.)
- 2026-09-11 (authoring): `PrivilegeErr` keeps `UserNotFound`/`WrongUser`
  unconditional (nix-free payloads) and gates only `Syscall{errno: Errno}` —
  the enum stays inhabited on Windows, avoiding uninhabited-type edge cases in
  callers, and PR 5's service-account check can reuse the existing variants.
- 2026-09-11 (authoring): `WriteOptions.mode` is silently ignored on Windows
  (documented on the field) rather than erroring: mode bits are advisory
  hardening on Unix; NTFS ACLs from the installer (PR 8) are the Windows
  equivalent. Erroring would make every `Some(mode)` call site cfg-aware.
- 2026-09-11 (authoring): CI runs the Windows check as `cargo check` (lib +
  bin) only — `--all-targets` would compile the test crate, whose Unix-API
  gating is PR 6's scope.
- 2026-09-11: deleted unused `files::create_symlink` and `CreateSymlinkErr`
  instead of keeping a unix-only wrapper. No production callers; the helper
  existed only for its own tests. Retention ELOOP fixtures keep using
  `std::os::unix::fs::symlink` directly.
- 2026-09-11: split `server/serve.rs` into `routes.rs` (portable router +
  `Options`) and `unix.rs` (UnixListener + systemd fd adoption). The file was
  two responsibilities; per-item `#[cfg(unix)]` imports were the smell. TCP
  transport (PR 11) adds a sibling module rather than more cfg in the router.
- 2026-09-11 (gate reduction, per Ben's preference to avoid cfg where an
  abstraction can absorb the difference): removed the inline `cfg`s from the
  two `files.rs` write paths by extracting `mode_open_options`/`apply_mode`
  helpers (gate isolated to their two arms; call sites are platform-agnostic),
  and moved the `layout::root()` suffix (`var/lib/miru` vs `Miru`) into
  `platform::data_root_suffix()` so `layout.rs` is now cfg-free.
  Three gates were assessed as irreducible and left in place — no library
  bridges the underlying *concept*, only the API:
  - shutdown signals (`main.rs`): SIGTERM has no Windows equivalent (service
    control lands in PR 5); `tokio::signal::ctrl_c` is already the shared arm.
  - `privilege/`: euid/gid-vs-passwd verification has no Windows analog (SID /
    service-account model, PR 5). `whoami`-class crates return a name, not a
    verification.
  - `server/unix.rs` UDS + systemd fd: `interprocess` could bridge
    UDS↔named-pipe, but the roadmap rejected named pipes for localhost TCP
    (PR 11), so there is no Windows implementation yet by design.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Key call paths: `main.rs:37` calls `privilege::verify_effective_user("miru")`
unconditionally (stays portable via the stub). `app/run.rs:139` starts the
socket server only when `options.enable_socket_server`. `routes.rs`'s
`Options { socket_file }` and `routes()` are referenced by tests and stay
unconditional. `WriteOptions.mode` is set by `crypt/rsa.rs` (0o600/0o640
key files) and honored in `filesys/files.rs` atomic + direct writers.
Conventions: import ordering, funclen ≤ 50, `./scripts/test.sh`,
`./scripts/lint.sh`, coverage gates Linux-only.

## Plan of Work / Concrete Steps

Edits as itemized in Progress. Validation loop:

    cargo check --target x86_64-pc-windows-msvc -p miru-agent   # windows
    ./scripts/test.sh                                           # linux suite
    ./scripts/lint.sh

CI: extend `.github/workflows/` lint workflow with the target install +
check (exact placement decided against the workflow file's layout; if the
msvc cross-check proves toolchain-infeasible on Linux runners — aws-lc-sys
needs a cross C compiler — fall back to a `windows-latest` runner job doing
the same `cargo check`, and record it here).

Commits: `docs(plans):` activation; `feat(windows): cfg-gate unix-only
APIs behind cfg(unix) with windows counterparts`; `ci: check the
x86_64-pc-windows-msvc target`.

## Validation and Acceptance

1. `cargo check --target x86_64-pc-windows-msvc -p miru-agent` exits 0.
2. Linux behavior byte-identical: full suite + lint green; no test
   added/removed; `cargo tree` unchanged on Linux.
3. CI enforces the Windows check on every PR.

## Idempotence and Recovery

Pure cfg-gating on a feature branch; revert = delete branch. No wire, data,
or packaging changes. Windows runtime behavior remains unreachable in
production until PR 5 (service) and PR 7/8 (build/installer) ship.
