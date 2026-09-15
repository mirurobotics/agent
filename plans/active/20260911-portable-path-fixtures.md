# Portable path fixtures: remove Unix-format filesystem literals from test code

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench4/repos/agent) | read/write | Rust workspace for the Miru device agent. All edits, validation, and commits happen here. |

Branch: `test/portable-path-fixtures` (base `main` at f1bdac9). A pulled-forward
slice of Windows-roadmap PR 6 ("test-suite portability",
`plans/active/20260910-windows-support.md`): the path-literal share only. The
Windows CI runner, `cfg(unix)` gating of Unix-API tests, and Unix-feature test
gating stay in their roadmap homes (see Decision Log).

## Purpose / Big Picture

A repo-wide audit (2026-09-11) found every filesystem path literal in
Unix format. Production code is already handled (platform module, #232) or
scheduled (PR 3 cfg-gates; PR 11 socket transport). This PR makes the **test
code's** path fixtures platform-agnostic so the suite is ready for a Windows
runner, fixing the fixtures that would actually fail there:

- `File::is_absolute()` fixtures like `/etc/foo.json` are NOT absolute on
  Windows (no drive prefix) — `validate_filepath` accept-tests and the
  `is_absolute` test would fail outright.
- `agent/tests/disk/layout.rs` pins the Unix layout (`/var/lib/miru/...`) in
  17 assertions; after #232 the Windows layout is `<ProgramData>\Miru\...`.
- `agent/tests/logs/mod.rs` pins `/var/log/miru` as the default.

The fix patterns: derive "some absolute path" fixtures from
`std::env::temp_dir()` (absolute on every platform); derive layout/logs
expectations from a per-OS `expected_root()` helper that pins the Unix
literals under `cfg(unix)` and the `%ProgramData%`-based layout under
`cfg(windows)`.

## Audit inventory (2026-09-11, full repo)

Filesystem Unix-format literals and their disposition:

**Fixed in this PR (test code, behavior-sensitive or real-FS):**
- `agent/src/deploy/filesys.rs` test mod — `/tmp/config.json` ×7,
  `/tmp/miru.backup.config.json` ×4, `/etc/myapp/config.json`,
  `/etc/myapp/../passwd` (validate_filepath tests fail on Windows)
- `agent/tests/deploy/filesys.rs:1107` — `/etc/myapp/../passwd`
- `agent/tests/filesys/file.rs` — `/tmp` display fixture; `/etc/foo.json`
  `is_absolute` fixture (fails on Windows)
- `agent/tests/filesys/dir.rs` — `/tmp` display fixture
- `agent/tests/disk/layout.rs` — 17 `/var/lib/miru...` assertions
- `agent/tests/logs/mod.rs:64` — `/var/log/miru` default assertion
- `agent/tests/app/state.rs:322` — `/tmp/miru` log dir (writes real logs)
- `agent/tests/provisioning/check.rs:61` — `/var/lib/miru/...` error fixture

**Left as-is — intentional Unix contract (not portability bugs):**
- `agent/src/platform/mod.rs` — `unix_*` constants ARE the per-OS abstraction
- `agent/src/server/serve.rs:40` — UDS default; Unix-only feature (PR 3
  gates, PR 11 adds TCP)
- `agent/tests/app/run.rs` `/tmp/miru.sock` ×4 + server tests — exercise the
  UDS transport; follow the module's `cfg(unix)` gating in PR 3
- `agent/src/privilege/*` + `agent/tests/privilege/mod.rs` — Unix user model;
  module is Windows-stubbed in PR 3
- `agent/src/privilege/errors.rs:9` — `/etc/passwd` in an error message

**Left as-is — inert data (never touches `Path` semantics):**
- `agent/tests/deploy/errors.rs:77` `/etc/app/config.json` — error-Display
  fixture; any string works on any OS

**Deferred to PR 6 proper (Unix-API tests needing `cfg(unix)` gates —
unverifiable until a Windows test build exists):**
- symlink loops: `agent/src/data_uploads/retention/deleter.rs:487-488`,
  `agent/tests/data_uploads/retention/deleter.rs:44-45`
- `PermissionsExt`/`from_mode`: `agent/src/deploy/filesys.rs` (4 sites),
  `agent/tests/{disk/device,provisioning/check,crypt/rsa,gcs/mod,filesys/path,filesys/files}.rs`

**Deferred with an open product question:** scanner/rule test globs built as
`format!("{}/*.mcap", dir)` — production globs are backend-supplied data, and
file-rule support on Windows is an open M4 customer question (workbench
`plans/backlog/20260911-windows-paths-m4.md`); glob-crate separator semantics
on Windows get decided there.

**False positives (not filesystem paths):** URL paths in `http/*`, MQTT
topics, S3/GCS object keys — `/` is the correct separator; untouched.

## Progress

- [x] Activate plan (`docs(plans):` commit on the branch)
- [x] `agent/src/deploy/filesys.rs`: temp_dir-based fixtures + portable validate_filepath paths
- [x] `agent/tests/deploy/filesys.rs`: portable traversal fixture
- [x] `agent/tests/filesys/{file,dir}.rs`: portable display/is_absolute fixtures
- [x] `agent/tests/disk/layout.rs`: per-OS `expected_root()` helper; all assertions derived; unix root pinned byte-for-byte in `root_dir`
- [x] `agent/tests/logs/mod.rs`: per-OS default expectation (unix literal pinned)
- [x] `agent/tests/app/state.rs`, `agent/tests/provisioning/check.rs`: temp_dir-based fixtures
- [x] `./scripts/test.sh` (all green) and `./scripts/lint.sh` clean
- [x] Push; CI green (lint/test/tools pass); PR opened as #233

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- 2026-09-11 (authoring): Scope = path literals only. `cfg(unix)` gating of
  symlink/permission tests is excluded: it is not path-format work, it
  roughly doubles the diff across 9 more files, and its correctness is
  unverifiable until PR 2+3 give a compiling Windows test build. The
  inventory above preserves the list so PR 6 does not re-audit.
- 2026-09-11 (authoring): "Some absolute path" fixtures use
  `std::env::temp_dir()` rather than a cfg-based literal helper — absolute on
  every platform, no cfg surface in tests, and consistent with the repo's
  existing `test_utils::filesys::dirs::temp` fixture idiom.
- 2026-09-11 (review): superseded — per Ben's preference, all fixtures use
  the repo's `test_utils::filesys::dirs::temp()` RAII helper uniformly (one greppable
  idiom), including path-only fixtures, at the cost of a real mkdir/rmdir
  per fixture. `tests/app/state.rs` binds the guard for the log dir; the
  global tracing worker writing to an unlinked dir after test end is
  harmless on the Unix runners that execute this test today.
- 2026-09-11 (authoring): `agent/tests/disk/layout.rs` keeps pinning the Unix
  literals byte-for-byte under `cfg(unix)` (the layout is a compatibility
  contract, not an implementation detail); the `cfg(windows)` expectation is
  derived from `platform::windows_data_root_base` + `Miru`, pinning the
  *structure* while tolerating the env-dependent base.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

`filesys::File::is_absolute()` delegates to `Path::is_absolute()`, which on
Windows requires a drive/UNC prefix — `/etc/foo.json` is relative there.
`deploy::filesys::validate_filepath` (absolute + no `..`) is exercised by
unit tests in `agent/src/deploy/filesys.rs` and integration tests in
`agent/tests/deploy/filesys.rs`. `disk::Layout::root()` is per-OS since
#232. Repo conventions: import ordering, ordinary `cargo test` or
`./scripts/test.sh`, `./scripts/lint.sh`, funclen ≤ 50.

## Plan of Work / Concrete Steps

1. `agent/src/deploy/filesys.rs` tests: add
   `fn tmp_path(name: &str) -> String` (temp_dir join, display string);
   replace the 11 `/tmp/...` fixture strings; rebuild the two `/etc/myapp`
   validate_filepath fixtures from `temp_dir().join("myapp")...`.
2. `agent/tests/deploy/filesys.rs`: traversal fixture from temp_dir joins.
3. `agent/tests/filesys/file.rs` + `dir.rs`: display fixtures from
   temp_dir; `is_absolute` fixture from temp_dir.
4. `agent/tests/disk/layout.rs`: `expected_root()` (cfg-split as in Decision
   Log) + `expected_root_under(base)` for the custom-root test; rewrite the
   17 assertions as joins off it. Default-root test compares
   `filesystem_root` to `platform::data_root_base()`.
5. `agent/tests/logs/mod.rs`: expected default from cfg-split helper.
6. `agent/tests/app/state.rs`: `temp_dir().join("miru")`;
   `agent/tests/provisioning/check.rs`: temp_dir-based error fixture.
7. Validate (`./scripts/test.sh`, `./scripts/lint.sh`), commit
   (`test(portability): ...`), push, PR.

## Validation and Acceptance

1. Full suite green on Linux with zero behavior change — same tests, same
   coverage, no `#[test]` added or removed.
2. Grep gate: no `"/tmp`, `"/etc`, `"/var` literals remain in the files this
   PR touches (the intentional categories above are the only repo survivors).
3. Lint clean; CI green on the pushed head.

## Idempotence and Recovery

Test-only edits on a feature branch; revert = delete branch. No production
code, wire, or packaging changes.
