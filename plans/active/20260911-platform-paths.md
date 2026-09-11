# Platform paths: per-OS defaults for the data root and log directory

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench4/repos/agent) | read/write | Rust workspace for the Miru device agent. All edits, validation, and commits happen here. |

Branch: `feat/platform-paths` (base `main` at 9b1f936). This is PR 4 ("platform paths") of the Windows-support roadmap in `plans/active/20260910-windows-support.md`, pulled forward of PR 3: it is independent of the cfg-gating work and of PR 2 (`refactor/crypt-aws-lc-rs`, open as #231).

## Purpose / Big Picture

The agent's two on-disk roots are hardcoded Unix literals: the state layout
roots at `/var/lib/miru` (`agent/src/disk/layout.rs:96` — `Default` is
`Dir::new("/")` and `root()` appends `var/lib/miru`) and logging defaults to
`/var/log/miru` (`agent/src/logs/mod.rs:55`). After this PR, defaults are
per-OS: Unix behavior is byte-identical, and Windows builds resolve
`%ProgramData%\Miru` (data) and `%ProgramData%\Miru\logs` (logs) via the
`ProgramData` environment variable with a `C:\ProgramData` fallback — never a
hardcoded `C:` drive assumption. `Layout` stays parameterized by
`filesystem_root` so tests keep injecting temp roots.

Out of scope: the Unix-socket path default (`server/serve.rs:40` — the local
server is disabled on Windows in Phase 1; transport is PR 11), all
`cfg`-gating of Unix-only APIs (PR 3), test-suite portability (PR 6), and any
packaging. Note the Windows arms cannot be compile-checked in CI until PR 3
adds the msvc `cargo check` job (which itself waits on PR 2 demoting
openssl); the design below keeps the Windows path logic unconditionally
compiled and unit-tested on Linux to de-risk that gap.

## Progress

- [x] Activate plan (`docs(plans):` commit on the branch) (72d0378)
- [x] New `platform` module: per-OS path defaults, Windows logic unconditionally compiled + Linux-tested (bc0a530)
- [x] `disk::Layout`: `Default` root via `platform::data_root_base()`; `root()` appends `var/lib/miru` (unix) / `Miru` (windows) (bc0a530)
- [x] `logs::Options::default`: `log_dir` via `platform::log_dir()` (bc0a530)
- [x] Tests: `agent/tests/platform/mod.rs` (env override, fallback, structure); existing disk/logs tests unchanged (bc0a530)
- [x] `./scripts/test.sh` — 1639 passed, 0 failed; `./scripts/lint.sh` clean (fix-mode reformatting folded into bc0a530)
- [x] Push; CI green (lint/test/tools pass); PR opened as #232

## Surprises & Discoveries

- 2026-09-11: none of substance — the change landed exactly as specified.
  Full suite + lint ran clean locally on a cold cache (~14 min build), so no
  CI-delegation waiver was needed this time.

## Decision Log

- 2026-09-11 (authoring): New top-level `platform` module rather than a
  helper inside `disk/`: both `disk` and `logs` (and later `server`, PR 11)
  need the defaults, and the roadmap names a "platform-paths module". Costs
  are the standard new-module chores (lib.rs, tests mirror, `.covgate`).
- 2026-09-11 (authoring): The Windows path functions take
  `Option<OsString>` (the `ProgramData` env value) as a parameter and are
  compiled on every target. Rationale: pure functions are unit-testable on
  Linux today — the only Windows-specific code is the one-line `cfg` dispatch
  reading the real env var, minimizing the surface that PR 3's future msvc
  check must catch.
- 2026-09-11 (authoring): On Windows, `Layout::root()` appends `Miru` (the
  env value already ends in `ProgramData`), while Unix appends
  `var/lib/miru` to `/`. Test roots keep working on both: a temp
  `filesystem_root` yields `<tmp>/var/lib/miru` on Unix and `<tmp>\Miru` on
  Windows. Existing Linux tests asserting `/var/lib/miru` are untouched;
  cfg-gating those assertions for Windows runners is PR 6's scope.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

`filesys::Dir` wraps `PathBuf`; `Dir::subdir`/`file` join with
`Path::join` and strip a single leading `MAIN_SEPARATOR` — already portable.
`Layout` (`agent/src/disk/layout.rs`) derives every state path from
`root()`; `logs::Options` (`agent/src/logs/mod.rs`) carries `log_dir` into
`tracing-appender`. Neither default is overridable from settings or env in
production today (`main.rs` always uses `Default`), so changing the two
`Default` impls changes production behavior on Windows only.

Repo conventions that apply: import ordering (standard/internal/external
groups with comments), function length ≤ 50 body lines, new module chores
(`agent/src/<mod>/mod.rs` + `pub mod` in `agent/src/lib.rs` + mirror in
`agent/tests/mod.rs` + `.covgate`), `./scripts/test.sh` (requires
`--features test`), `./scripts/lint.sh`.

## Plan of Work / Concrete Steps

1. `agent/src/platform/mod.rs` (new):
   - `pub fn data_root_base() -> PathBuf` — cfg dispatch: unix →
     `unix_data_root_base()`; windows →
     `windows_data_root_base(std::env::var_os("ProgramData"))`.
   - `pub fn log_dir() -> PathBuf` — cfg dispatch: unix → `unix_log_dir()`;
     windows → `windows_log_dir(std::env::var_os("ProgramData"))`.
   - `pub fn unix_data_root_base()` = `/`; `pub fn unix_log_dir()` =
     `/var/log/miru`.
   - `pub fn windows_data_root_base(program_data: Option<OsString>)` = the
     env value, else `C:\ProgramData`; `pub fn windows_log_dir(...)` = base
     joined `Miru\logs`. All four compiled unconditionally.
   - `.covgate` at 95.
2. `agent/src/lib.rs` + `agent/tests/mod.rs`: register `platform`
   (alphabetical).
3. `agent/src/disk/layout.rs`: `Default` becomes
   `Self::new(filesys::Dir::new(platform::data_root_base()))`; `root()`
   gains the cfg split described above.
4. `agent/src/logs/mod.rs`: `log_dir: platform::log_dir()`.
5. `agent/tests/platform/mod.rs`: env-override honored, fallback constant
   used, log dir nests under the base, unix constants exact.
6. Validate: targeted `cargo test --features test` for `platform`, `disk`,
   `logs`; full `./scripts/test.sh` + `./scripts/lint.sh` as cache permits
   (CI is the authoritative gate — record any local waiver in the Decision
   Log as PR 1 did).
7. Commits: `docs(plans):` for this file; `feat(platform): per-OS default
   data root and log dir` for the code. Push, open PR.

## Validation and Acceptance

1. Linux behavior byte-identical: `Layout::default().root()` displays
   `/var/lib/miru`; `logs::Options::default().log_dir` is `/var/log/miru`;
   all pre-existing tests pass unmodified.
2. Windows logic proven on Linux: `windows_data_root_base(None)` is
   `C:\ProgramData`; `windows_data_root_base(Some(custom))` is `custom`;
   `windows_log_dir` nests `Miru`/`logs` under the base.
3. Lint clean (`./scripts/lint.sh`), coverage gate for `platform` met, CI
   green on the pushed head.

## Idempotence and Recovery

All edits are additive or default-value swaps on a feature branch; revert =
delete branch. No data, wire, or packaging changes. If PR 3's later msvc
check surfaces a Windows compile issue in the cfg dispatch, the fix is
contained to `platform/mod.rs`.
