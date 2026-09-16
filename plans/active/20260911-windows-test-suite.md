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

Before this branch, #234's `windows-check` job compiled lib + bin for
`x86_64-pc-windows-msvc` but did not compile or run the **test crate**. This
work makes `cargo test --package miru-agent --locked` compile and pass on Windows, and adds
a CI step that runs it — turning up **runtime** Unix assumptions (path
separators in assertions, permission-mode checks, symlink behavior,
delete-while-open) that a compile check cannot catch, and gating the
Unix-only-API test code (`PermissionsExt`) that would otherwise fail to compile
on Windows.

The scope also includes production fixes discovered by the Windows tests:
home-directory lookup uses `std::env::home_dir()`, and file copies no longer
carry a sync option, so `copy_to` is plain `tokio::fs::copy` on every platform.

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
- [x] CI: add a `cargo test --package miru-agent --locked` step to the windows job (RUST_LOG=off; no coverage)
- [x] Gate genuinely Unix-only test code (from audit)
- [x] Replace portable scenarios' Unix-only fixtures with cross-platform fixtures
- [x] Derive behavioral path expectations from fixtures while preserving literal wire pins
- [x] Initial pre-review Linux validation: `./scripts/test.sh` + `./scripts/lint.sh` green
- [x] Post-review targeted queue, filesystem, path, and deleter tests green on Linux
- [x] Push; inspect the first Windows CI test-job failure and fix its compile errors/warnings locally
- [x] Re-run Windows CI and iterate until green (2026-09-11, commit `57361317124296750ad8175be10669686eb5f6f0`)
- [x] PR opened
- [x] Fresh CI validation of the 2026-09-13 review fixes; all checks green (run 34922948716, green)

## Surprises & Discoveries

- 2026-09-11: `File::new` normalizes separators for the host, so behavioral
  queue and error-display expectations must come from their constructed
  fixtures. Raw persisted JSON is different: it intentionally pins `/data/...`
  and its expected `File` must be deserialized from that same literal wire
  value so the comparison tests preservation rather than host normalization.
  2026-09-15: queue wire tests build the raw `"file"` JSON from a plain
  host-rooted string (`abs_path("data/a.log")`) rather than from `File`'s
  serializer, so they still pin that `File` serializes as a bare string while
  staying host-portable. Fixture paths across the suite come from
  `test_utils::filesys::{abs_path, abs_file, abs_dir}`.
  2026-09-15: After `Dir::new` gained the same normalization as `File::new`
  (#240), the `abs_file`/`abs_dir` wrappers were removed; only `abs_path` (raw
  `PathBuf` fixtures) and `missing_file` remain.
- 2026-09-11: an existing directory represented as a `File` supplies a portable
  delete failure after a successful stat. It exercises retry counts, backoff,
  attempt caps, and persistence without a Unix symlink loop.
- 2026-09-11: the first Windows CI run reached lib-test compilation and found
  five shutdown-manager tests calling `with_socket_server_handle`, whose
  implementation was unnecessarily Unix-gated even though it only stores a
  Tokio join handle. Compile that helper for Unix production and all unit-test
  builds so its platform-neutral shutdown/error tests continue to run on
  Windows; the socket server initialization itself remains Unix-only.
- 2026-09-11: the second Windows CI run (34655051306) compiled and ran all 346
  lib tests; 345 passed. `stat_failure_counts_an_attempt` was the sole failure
  because Windows maps metadata of a child beneath a file to `NotFound`, so the
  deleter correctly classified that fixture as already gone instead of retryable.

## Decision Log

- 2026-09-11 (authoring): keep the CI job id `windows-check` and add a test
  step to it rather than renaming or adding a second windows job — preserves
  any branch-protection required-check keyed on that name, and one
  `windows-latest` runner (a 2x-cost GitHub-hosted runner) doing check-then-test
  is cheaper than two. Rename deferred until required-checks are confirmed.
- 2026-09-11: tests that specifically assert Unix mode bits, permission-denied
  behavior induced by Unix modes, Unix absolute-path semantics, or other Unix
  APIs remain gated with `#[cfg(unix)]`. Tests of cross-platform error behavior remain
  enabled: the `set_permissions` missing-target cases borrow permissions from
  an existing directory, and deleter retry/backoff/persistence cases use an
  existing directory represented as a `File`.
- 2026-09-11: keep literal `/data/...` JSON in queue wire tests. Build the
  whole-job expected `File` by deserializing that literal, while deriving all
  behavioral queue names through their job factories.
  2026-09-15: queue wire tests build the raw `"file"` JSON from a plain
  host-rooted string (`abs_path("data/a.log")`) rather than from `File`'s
  serializer, so they still pin that `File` serializes as a bare string while
  staying host-portable. Fixture paths across the suite come from
  `test_utils::filesys::{abs_path, abs_file, abs_dir}`.
  2026-09-15: After `Dir::new` gained the same normalization as `File::new`
  (#240), the `abs_file`/`abs_dir` wrappers were removed; only `abs_path` (raw
  `PathBuf` fixtures) and `missing_file` remain.
- 2026-09-11: replace the stat-classification canary's child-beneath-a-file
  fixture with a path containing an embedded NUL. Rust rejects that path as
  invalid input before filesystem lookup on Unix and Windows, deterministically
  exercising the existing non-`NotFound` metadata-error branch without a
  production seam or behavior change.
- 2026-09-14 (review iteration 2): the Windows-only readonly clear/open/restore
  path existed only to `sync_data` a copied file. No production caller needs a
  synced copy, so `CopyOptions` and the sync path were removed; `copy_to` takes
  an `Overwrite` and delegates to `tokio::fs::copy`. This removes all
  `#[cfg(windows)]` code from `files.rs`.
- 2026-09-14: `dirs::home()` delegates to `std::env::home_dir()` (stable,
  un-deprecated since 1.87; MSRV 1.93) instead of hand-selecting
  `HOME`/`USERPROFILE`.
- 2026-09-14: the Windows job runs only `cargo test` (the check step was
  redundant and used a different target dir); rust-cache gets
  `key: test-suite` + `cache-on-failure` because the cache saved by the
  check-only job on `main` never contained `target/debug/` and rust-cache does
  not re-save on an exact hit.

## Outcomes & Retrospective

Windows test execution, portable fixtures and assertions, and the production
home-lookup and synced-copy fixes are implemented. CI was green on 2026-09-11
at `57361317124296750ad8175be10669686eb5f6f0`
([run 34658026631](https://github.com/mirurobotics/agent/actions/runs/34658026631)).
Review fixes validated by run 34922948716.

Follow-up (needs a Windows dev box): un-gate the deploy permission-denied tests
with a per-platform inducer so their platform-neutral rollback assertions also
run on Windows.

## Audit inventory

From the exhaustive test-tree audit (2026-09-11). Fixed in this PR:

Genuinely Unix-only code remains gated (Unix API imports and tests that assert
mode bits, Unix permission denial, Unix absolute-path semantics, or Unix-only
integrations):
- `tests/{disk/device, provisioning/check, crypt/rsa, filesys/dirs,
  filesys/files, filesys/path, deploy/filesys}.rs` and `src/gcs/store.rs`
  (inline `source_unreadable` module) — import gated `#[cfg(unix)]`;
  `from_mode`/`.mode()` tests and helpers gated where their semantics are
  Unix-specific.
- `tests/deploy/filesys.rs` — `read_only`/`writeable` perm fixtures + their 7
  permission-denied tests gated (Windows ignores the readonly attribute for
  child creation, so the denial can't reproduce).

Portable scenarios remain enabled on Windows:
- `tests/filesys/{dirs,files}.rs` missing-target `set_permissions` tests use
  permissions cloned from an existing directory rather than Unix mode bits.
- `tests/data_uploads/retention/deleter.rs` and the in-crate deleter test module
  use an existing directory represented as a `File` for retry count, attempt
  cap, backoff, and persistence coverage. Removing a directory through the file
  unlink path fails portably after stat succeeds.
- Generic queue behavioral paths come from each supplied job factory; retention
  TTL behavior does likewise. Literal JSON remains unchanged in both queue wire
  pins, and expected `File` values are deserialized from the literal.
- Filesystem error-display tests derive expected strings from their `PathBuf`,
  `File`, and `Dir` fixtures. The home-directory test compares directly with
  `std::env::home_dir()`, without changing the environment.
- Copy coverage (all platforms) includes a readonly source and verifies the
  copied contents and the preserved readonly attribute.

Already gated (no action): `tests/mod.rs` `privilege` module, `deploy/apply.rs`
perm tests, the existing `#[cfg(unix)]` mode-test bodies in `filesys/{dirs,
files}.rs`, `logs/mod.rs` + `disk/layout.rs` per-OS split (#233).

Compiles + passes on Windows (verified reasoning, left unchanged): `app/run.rs`
`/tmp/miru.sock` fixtures (stored, never bound on Windows — bind is cfg-skipped),
`deploy/errors.rs` `/etc/app/config.json` (a `String` field, not a `Path`).

Resolved after the second Windows run: the narrow stat-classification canary
confirmed that Windows maps `metadata(file\child)` to `NotFound`, unlike Unix's
ENOTDIR. `src/.../deleter.rs::stat_failure_counts_an_attempt` now uses an
embedded-NUL path, which produces a portable invalid-input metadata error and
therefore continues to pin the counted-retry branch. The broader retry,
backoff, attempt-cap, and persistence coverage continues to use the portable
existing-directory-as-`File` fixture.

## Plan of Work / Concrete Steps

1. CI: in `.github/workflows/ci.yml`, add to the `windows-check` job after
   the check step:

       - name: Run Windows Tests
         env:
           RUST_LOG: "off"
         run: cargo test --package miru-agent --locked

2. Gate genuinely Unix-specific assertions and APIs behind `#[cfg(unix)]`.
   Keep platform-neutral behavior enabled with portable fixtures: permissions
   copied from existing metadata and an existing directory used as an
   undeletable `File`.
3. Make runtime path assertions platform-native by deriving queue names and
   displayed error paths from fixtures. Preserve literal JSON in wire pins and
   deserialize the expected `File` from that wire literal.
4. Validate changed tests on Linux, then rely on the native Windows CI job for
   Windows runtime behavior.
5. Push; read the Windows CI test job logs; fix residuals; repeat until green.

## Validation and Acceptance

1. `cargo test --package miru-agent --locked` compiles and passes on `windows-latest` (CI).
2. Linux behavior remains covered: Unix-only tests still run on Linux, while
   portable replacements exercise the same cross-platform error paths.
3. Behavioral path assertions accept native separators without weakening queue
   ordering/error semantics; literal persisted JSON remains pinned exactly.
4. CI enforces the Windows test run on every PR.
5. Home-directory lookup matches `std::env::home_dir()` on every platform.
6. A copy succeeds for a readonly source on every platform and preserves the
   destination's readonly attribute.

## Idempotence and Recovery

Revert the relevant commits to undo the production, test, plan, and CI changes;
deleting the feature branch does not undo merged changes. Wire format and
packaging are unchanged. A failed copy can leave a partially written
destination; callers that need atomicity use the atomic write path.
