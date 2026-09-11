# Windows test-suite portability + CI test job

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench4/repos/agent) | read/write | Rust workspace for the Miru device agent. All edits, validation, and commits happen here. |

Branch: `test/windows-test-suite` (base `main` at a0e7afb, which includes PR 3
windows cfg-gates #234). This is **PR 6** of the Windows-support roadmap
(`plans/active/20260910-windows-support.md`).

## Purpose / Big Picture

#234's `windows-check` job compiles lib + bin for
`x86_64-pc-windows-msvc` but never compiles or runs the **test crate**. This
PR makes `cargo test --features test` compile and pass on Windows, and adds a
CI step that runs it — turning up **runtime** Unix assumptions (path
separators in assertions, permission-mode checks, symlink behavior,
delete-while-open) that a compile check cannot catch, and gating the
Unix-only-API test code (`PermissionsExt`, `std::os::unix::fs::symlink`) that
would otherwise fail to compile on Windows.

Coverage gates stay Linux-only (they need `cargo-llvm-cov` and the
`.covgate` thresholds are tuned on Linux); the Windows job runs tests
without coverage.

## Constraint: no local Windows

There is no Windows runtime or cross-compilable test crate available locally
(aws-lc-sys drives the host C compiler and cannot cross-build the test crate
from Linux — see #234). Validation of the Windows arm is therefore
**entirely CI-driven** on the `windows-latest` runner. To minimize round-trips
the gating is front-loaded from an exhaustive audit of the test tree before
the first CI run; residual runtime failures are then fixed from CI logs.

## Progress

- [x] Activate plan (`docs(plans):` commit on the branch)
- [x] CI: add a `cargo test --features test` step to the windows job (RUST_LOG=off; no coverage)
- [x] Gate/fix won't-compile-on-windows test code (from audit)
- [x] Gate/fix compiles-but-fails-at-runtime test code (from audit)
- [x] Linux: `./scripts/test.sh` + `./scripts/lint.sh` green (zero behavior change on Linux)
- [ ] Push; iterate on the Windows CI test job until green
- [ ] PR opened; all checks green

## Surprises & Discoveries

(Add entries as work proceeds — especially runtime failures the audit missed.)

## Decision Log

- 2026-09-11 (authoring): keep the CI job id `windows-check` and add a test
  step to it rather than renaming or adding a second windows job — preserves
  any branch-protection required-check keyed on that name, and one
  `windows-latest` runner (a 2x-cost GitHub-hosted runner) doing check-then-test
  is cheaper than two. Rename deferred until required-checks are confirmed.
- 2026-09-11 (authoring): Unix-API tests (permission modes, symlink loops)
  are gated with `#[cfg(unix)]` rather than reimplemented for Windows — they
  assert Unix-specific behavior (mode bits, ELOOP) that has no Windows
  meaning. Windows equivalents (ACLs) are the installer's concern (PR 8), not
  the agent's test suite. This matches #234's decision to ignore mode bits on
  Windows.

## Outcomes & Retrospective

(Summarize at completion.)

## Audit inventory

From the exhaustive test-tree audit (2026-09-11). Fixed in this PR:

Won't-compile-on-Windows (unconditional `use std::os::unix::fs::PermissionsExt;`
imports + ungated mode/symlink bodies):
- `tests/{disk/device, provisioning/check, crypt/rsa, filesys/dirs,
  filesys/files, filesys/path, deploy/filesys, gcs/mod}.rs` — import gated
  `#[cfg(unix)]`; each ungated `from_mode`/`.mode()` test or helper gated.
- `tests/deploy/filesys.rs` — `read_only`/`writeable` perm fixtures + their 7
  permission-denied tests gated (Windows ignores the readonly attribute for
  child creation, so the denial can't reproduce).
- `tests/data_uploads/retention/deleter.rs` + in-crate
  `src/data_uploads/retention/deleter.rs` test module — `symlink_loop`
  fixture + its caller tests gated; the integration `wedged_job` helper gated
  (unused on Windows once its only caller is gated).

Already gated (no action): `tests/mod.rs` `privilege` module, `deploy/apply.rs`
perm tests, the existing `#[cfg(unix)]` mode-test bodies in `filesys/{dirs,
files}.rs`, `logs/mod.rs` + `disk/layout.rs` per-OS split (#233).

Compiles + passes on Windows (verified reasoning, left unchanged): `app/run.rs`
`/tmp/miru.sock` fixtures (stored, never bound on Windows — bind is cfg-skipped),
`deploy/errors.rs` `/etc/app/config.json` (a `String` field, not a `Path`).

Runtime-uncertain (the CANARY — left ungated deliberately):
`src/.../deleter.rs::stat_failure_counts_an_attempt` induces a retryable stat
failure via ENOTDIR (`stat` of a child-of-a-file) using portable APIs. On Unix
that is `FileMetadataErr` → `SweepOutcome::Failed`. On Windows it depends on
whether `metadata(file\child)` returns `NotADirectory` (→ Failed, passes) or
`PathNotFound` (→ NotFound → `AlreadyGone`, fails). Unverifiable without a
Windows runtime, so it rides the first Windows CI run: if it fails, gate it
(and the symlink tests stay gated). If it passes, a follow-up could convert the
gated symlink tests to this cross-platform ENOTDIR wedge and drop those gates.

## Plan of Work / Concrete Steps

1. CI: in `.github/workflows/ci.yml`, add to the `windows-check` job after
   the check step:

       - name: Run Windows Tests
         env:
           RUST_LOG: "off"
         run: cargo test --package miru-agent --features test --locked

2. Gate won't-compile items behind `#[cfg(unix)]` (imports, whole test fns,
   or test modules as appropriate). Where a test exercises a cross-platform
   behavior via a Unix-only mechanism, keep the Unix path gated and add a
   Windows-appropriate variant only if the behavior is meaningful there.
3. Fix runtime-divergent assertions (path-string comparisons already handled
   in #233; re-verify none remain in the audited set).
4. Linux validation loop (`./scripts/test.sh`, `./scripts/lint.sh`).
5. Push; read the Windows CI test job logs; fix residuals; repeat until green.

## Validation and Acceptance

1. `cargo test --features test` compiles and passes on `windows-latest` (CI).
2. Linux behavior byte-identical: full suite + lint green; no Linux test
   removed or weakened (gated tests still run on Linux).
3. CI enforces the Windows test run on every PR.

## Idempotence and Recovery

Test-only + CI edits on a feature branch; revert = delete branch. No
production code, wire, or packaging changes.
