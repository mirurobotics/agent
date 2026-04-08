# Flip WriteOptions Default to Atomic

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Change the default variant of the `Atomic` enum in `filesys` and update the one test that asserts on the old default. |

This plan lives in `agent/plans/` because every file touched is inside the `miru-agent` Rust crate in this repository. No other repos are read or written.

**Repo layout note.** The `agent` repository is a Cargo workspace at its root. The `miru-agent` crate lives in the subdirectory `agent/` inside the repo (yes, the crate directory shares the name of the repo). All paths in this plan are **repo-root-relative**. So `agent/src/filesys/mod.rs` in this plan means the file at `<repo-root>/agent/src/filesys/mod.rs` — the crate source — not a path relative to the crate. The `scripts/` directory lives at the repo root (`<repo-root>/scripts/`), not inside the crate. Commands in Concrete Steps are run from the repo root.

## Purpose / Big Picture

The `miru-agent` Rust crate has a `WriteOptions` struct used by `filesys::File::write_bytes` and its friends. `WriteOptions` derives `Default`, and today `WriteOptions::default()` returns a non-atomic, no-overwrite write because the `Atomic` enum has `#[default]` on the `No` variant. That is a footgun: a developer who reaches for `WriteOptions::default()` silently gets a non-atomic write, even though atomic writes are what every production call site in `agent/src/` actually asks for (all production write sites use `WriteOptions::OVERWRITE_ATOMIC` or explicitly construct `WriteOptions { atomic: Atomic::Yes, .. }`).

After this change, `WriteOptions::default()` will return `{ overwrite: Overwrite::Deny, atomic: Atomic::Yes }`. New code that reaches for the default gets the safer behavior. Production behavior does not change at all — no production call site uses `WriteOptions::default()`. This is a targeted fix to the defaults; nothing else about the API shape changes.

A reader can verify the change by:

1. Running `./scripts/test.sh` and seeing the full test suite pass.
2. Reading the test `tests/filesys/path.rs::write_options::default` and seeing that `WriteOptions::default().atomic` is asserted to equal `Atomic::Yes`.

## Progress

- [x] M1: Flip the `Atomic` default and update the single test assertion that reads it. (2026-04-08)
  - [x] Move `#[default]` from `Atomic::No` to `Atomic::Yes` in `agent/src/filesys/mod.rs`.
  - [x] Update the `default()` assertion in `agent/tests/filesys/path.rs` to expect `Atomic::Yes`.
  - [x] Run `./scripts/test.sh` from `agent/` and confirm all tests pass.
  - [x] Run `./scripts/lint.sh` from `agent/` and confirm clean exit.
  - [x] Commit the change with a single conventional commit message (see Concrete Steps).

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

- Observation: `WriteOptions::ATOMIC` is now an exact alias of `WriteOptions::default()` — both produce `{ overwrite: Overwrite::Deny, atomic: Atomic::Yes }`.
  Evidence: After the `#[default]` flip in `agent/src/filesys/mod.rs`, the `ATOMIC` constant (lines 60–63) and the derived `Default` impl yield identical values. The constant is kept intentionally (see Decision Log) and is not removed — this observation is recorded for future readers who may want to consolidate.

- Observation: Flipping the default is behaviorally a no-op in production; every production write site under `agent/src/` already passes `WriteOptions::OVERWRITE_ATOMIC` or explicitly sets `atomic: Atomic::Yes`.
  Evidence: A `grep` over `agent/src/**/*.rs` at implementation time confirmed zero callers of `WriteOptions::default()` in production code. All ~50 callers live under `agent/tests/**`, where the write mechanism (tempfile+rename vs direct write) is irrelevant to the fixture behavior they exercise.

## Decision Log

- Decision: Flip the default of `Atomic` rather than removing the `Atomic::No` variant or removing `WriteOptions::OVERWRITE`.
  Rationale: Three tests in `agent/tests/filesys/file.rs` (around lines 472, 578, 683) still exercise the non-atomic branch in `File::write_bytes` explicitly via `WriteOptions { atomic: Atomic::No, .. }`. Removing the variant or the constant would break those tests and delete the non-atomic code path that they cover. The footgun we want to fix is strictly about the implicit default, not the existence of the non-atomic option.
  Date/Author: 2026-04-08

- Decision: Update only `agent/tests/filesys/path.rs` for assertions — leave all other test files that use `WriteOptions::default()` untouched.
  Rationale: An orchestrator audit established that the ~50 callsites of `WriteOptions::default()` across `agent/tests/**` are fixture writes that don't care whether the write is atomic. The only test that asserts the shape of `WriteOptions::default()` itself is `path.rs::write_options::default`. Changing anything else would be scope creep.
  Date/Author: 2026-04-08

## Outcomes & Retrospective

**Outcome.** M1 shipped in a single commit (`2e11438 refactor(filesys): default WriteOptions to atomic writes`). Two files changed, four lines of diff total: one `#[default]` attribute moved in `agent/src/filesys/mod.rs`, one assertion updated in `agent/tests/filesys/path.rs`. The full test suite (`./scripts/test.sh`) and lint suite (`./scripts/lint.sh`) passed. `WriteOptions::default()` now returns `{ overwrite: Overwrite::Deny, atomic: Atomic::Yes }` as intended.

**Retrospective.** The plan's blast-radius audit held: production code was untouched, and every `WriteOptions::default()` call site in tests continued to pass without modification. The only noteworthy side effect (captured in Surprises & Discoveries) is that `WriteOptions::ATOMIC` is now an exact alias of the derived default. A follow-up consolidation of that constant is out of scope for this plan and would require its own plan if pursued.

## Context and Orientation

This plan operates on the `miru-agent` Rust crate, whose source lives at `agent/src/` (inside this repository). The `filesys` module provides the crate's filesystem I/O primitives. The two files that matter for this change are:

- `agent/src/filesys/mod.rs` — defines the `Atomic`, `Overwrite`, and `Sync` enums and the `WriteOptions` / `AppendOptions` structs. It also defines three associated constants on `WriteOptions`: `OVERWRITE_ATOMIC`, `OVERWRITE`, and `ATOMIC`. Current relevant excerpt:

        /// Whether a write should be performed atomically (write to a temporary file,
        /// then rename into place).
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub enum Atomic {
            #[default]
            No,
            Yes,
        }

        /// Options for file write operations.
        #[derive(Clone, Copy, Debug, Default)]
        pub struct WriteOptions {
            pub overwrite: Overwrite,
            pub atomic: Atomic,
        }

        impl WriteOptions {
            /// Overwrite existing files using atomic writes.
            pub const OVERWRITE_ATOMIC: Self = Self {
                overwrite: Overwrite::Allow,
                atomic: Atomic::Yes,
            };

            /// Overwrite existing files, non-atomic.
            pub const OVERWRITE: Self = Self {
                overwrite: Overwrite::Allow,
                atomic: Atomic::No,
            };
            ...
        }

  Because `WriteOptions` derives `Default` and its two fields also derive `Default`, `WriteOptions::default()` is entirely driven by the `#[default]` annotations on the underlying enums. Moving `#[default]` from `Atomic::No` to `Atomic::Yes` is therefore the only source change required to flip the default.

- `agent/tests/filesys/path.rs` — contains the `write_options` test module (lines 27–48) with three tests: `default`, `overwrite_atomic`, and `overwrite`. Only the `default` test (lines 30–35) is affected by the flip:

        #[test]
        fn default() {
            let opts = WriteOptions::default();
            assert_eq!(opts.overwrite, Overwrite::Deny);
            assert_eq!(opts.atomic, Atomic::No);
        }

  The `overwrite_atomic` and `overwrite` tests assert on the `OVERWRITE_ATOMIC` and `OVERWRITE` constants, which are not changing, so they stay as-is.

Two non-goal files to keep in mind:

- `agent/src/filesys/file.rs` — holds `File::write_bytes` (lines ~150–222). Lines 201–220 are the non-atomic write branch, taken when `opts.atomic == Atomic::No`. This branch stays exactly as it is.

- `agent/tests/filesys/file.rs` — contains three tests (near lines 472, 578, 683) that explicitly construct `WriteOptions { atomic: Atomic::No, .. }` to exercise the non-atomic branch. Do not touch these; they must continue to pass unchanged.

Key terms:

- **Atomic write**: write to a temporary sibling file, then `rename(2)` it into place. A crash during the write leaves either the old or the new file, never a truncated one.
- **`WriteOptions::default()`**: the value produced by Rust's `Default` trait for `WriteOptions`. It is what callers get when they write `WriteOptions::default()` or use struct update syntax like `WriteOptions { overwrite: Overwrite::Allow, ..Default::default() }`.
- **`#[default]` attribute**: on a `Default`-deriving enum, this attribute marks which variant the derived `Default::default()` returns.

Blast radius audit (already performed by the orchestrator, recorded here so this plan is self-contained):

- Production code under `agent/src/` contains zero occurrences of `WriteOptions::default()`. Every production write site uses `WriteOptions::OVERWRITE_ATOMIC` or explicitly constructs `WriteOptions { atomic: Atomic::Yes, .. }`. So flipping the default is behaviorally a no-op in production.
- Test code under `agent/tests/` contains roughly 50 calls to `WriteOptions::default()`. The majority are fixture writes in `agent/tests/filesys/{file,dir,cached_file}.rs`, `agent/tests/app/{state,run}.rs`, `agent/tests/authn/token_mngr.rs`, and `agent/tests/events/store.rs`. They write bytes to a file in a test directory and do not care whether the write is atomic; flipping the default affects only the mechanism of the write (tempfile + rename vs. direct write), not its observable result. These tests should pass unchanged.
- The only test that asserts on the concrete value of `WriteOptions::default()` is `agent/tests/filesys/path.rs::write_options::default`. This test must be updated.

Repo conventions (from `agent/AGENTS.md`) that apply to this work:

- **Test command**: `./scripts/test.sh`, run from the repo root (`agent/`). It invokes `RUST_LOG=off cargo test --features test -- --test-threads=1`. Both `--features test` and `--test-threads=1` are required. Without `--features test`, test-only helpers gated on `#[cfg(feature = "test")]` are missing and the build fails. Without `--test-threads=1`, tests that bind `/tmp/miru.sock` race each other and produce misleading socket-conflict errors. Do not invoke `cargo test` directly.
- **Lint command**: `./scripts/lint.sh`, run from the repo root. Runs a custom import linter, `cargo fmt`, `cargo machete`, `cargo audit`, and `cargo clippy`.
- **Coverage**: `./scripts/covgate.sh`. Not expected to change from this refactor (no new or removed code paths), but it is part of the preflight toolchain and is mentioned here so that `$preflight` runs it as usual.

## Plan of Work

This is a single-milestone change. All edits are within the `miru-agent` crate.

**M1. Flip the `Atomic` default and update the one dependent test assertion.**

Two file edits, then test, lint, and commit.

Edit 1 — `agent/src/filesys/mod.rs`, in the `Atomic` enum declaration (around lines 23–28):

- Remove `#[default]` from the `No` variant.
- Add `#[default]` to the `Yes` variant.

After the edit, the enum should read exactly:

    /// Whether a write should be performed atomically (write to a temporary file,
    /// then rename into place).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Atomic {
        No,
        #[default]
        Yes,
    }

Do not change the derives, the doc comment, the variant order, or any of the surrounding `Overwrite` / `Sync` / `WriteOptions` definitions. Do not change the `OVERWRITE_ATOMIC`, `OVERWRITE`, or `ATOMIC` constants.

Edit 2 — `agent/tests/filesys/path.rs`, in the `write_options::default` test (around lines 30–35):

- Change the `opts.atomic` assertion to expect `Atomic::Yes` instead of `Atomic::No`.
- Leave the `opts.overwrite` assertion (`Overwrite::Deny`) unchanged.

After the edit, the test should read:

    #[test]
    fn default() {
        let opts = WriteOptions::default();
        assert_eq!(opts.overwrite, Overwrite::Deny);
        assert_eq!(opts.atomic, Atomic::Yes);
    }

Do not change the `overwrite_atomic` or `overwrite` tests below it; both `OVERWRITE_ATOMIC` and `OVERWRITE` constants are unchanged.

Files explicitly not touched (from Context and Orientation):

- `agent/src/filesys/file.rs` — the non-atomic branch in `File::write_bytes` (lines 201–220) stays.
- `agent/tests/filesys/file.rs` — the three tests around lines 472, 578, 683 that explicitly use `Atomic::No` stay. They exercise the still-present non-atomic branch.

No new files are created.

## Concrete Steps

All commands run from the **`agent` repository root**, i.e. `cd /path/to/agent/` (on the author's machine: `/home/ben/miru/workbench1/agent`). This is the repo root, not the nested `agent/` crate directory — `scripts/test.sh` and `scripts/lint.sh` live at the repo root and use `git rev-parse --show-toplevel` internally to find the workspace. Verify you are on the working branch before starting:

    git rev-parse --abbrev-ref HEAD

Expected transcript:

    refactor/atomic-writes

If you are not on `refactor/atomic-writes`, switch to it before making edits. The branch is identical to `main` at the start of this plan.

**Step 1 — Edit `agent/src/filesys/mod.rs`.**

Move the `#[default]` attribute from `Atomic::No` to `Atomic::Yes`, exactly as described in Plan of Work Edit 1.

Confirm only one file is modified so far:

    git status --short

Expected transcript:

     M agent/src/filesys/mod.rs

**Step 2 — Edit `agent/tests/filesys/path.rs`.**

In the `write_options::default` test, change `Atomic::No` to `Atomic::Yes` in the `opts.atomic` assertion. Do not touch any other test in the file.

Confirm exactly two files are now modified:

    git status --short

Expected transcript:

     M agent/src/filesys/mod.rs
     M agent/tests/filesys/path.rs

Optionally inspect the diff to confirm it is tiny:

    git diff

Expected: only the two hunks described above; nothing else.

**Step 3 — Run the test suite.**

    ./scripts/test.sh

This runs `RUST_LOG=off cargo test --features test -- --test-threads=1`. Expected result: the full crate test suite passes. In particular:

- `tests::filesys::path::write_options::default` passes (and would have failed without the Edit 2 update — proving the default was flipped).
- `tests::filesys::path::write_options::overwrite_atomic` passes (unchanged).
- `tests::filesys::path::write_options::overwrite` passes (unchanged).
- The three tests in `tests::filesys::file` that explicitly construct `WriteOptions { atomic: Atomic::No, .. }` around lines 472, 578, 683 pass unchanged.
- All other tests across `tests::filesys`, `tests::app`, `tests::authn`, `tests::events`, etc. pass unchanged. They use `WriteOptions::default()` for fixture writes; flipping the mechanism from non-atomic to atomic does not affect their observable behavior.

Expected final line from `cargo test`:

    test result: ok. <N> passed; 0 failed; <M> ignored; ...

If any test fails, read the failure carefully. A failure in `tests::filesys::path::write_options::default` means Edit 2 was not applied; re-check the file. A failure elsewhere is unexpected — do not "fix" it by editing more code; pause and diagnose.

**Step 4 — Run the linter.**

    ./scripts/lint.sh

Expected: clean exit (the custom import linter, `cargo fmt`, `cargo machete`, `cargo audit`, and `cargo clippy` all pass). No import changes were made, so the import linter should be uninvolved; `cargo fmt` and `cargo clippy` should have nothing to complain about since the edits are trivial.

**Step 5 — Commit the milestone.**

One commit per milestone (this plan has one milestone, so exactly one commit). From the `agent/` repo root:

    git add agent/src/filesys/mod.rs agent/tests/filesys/path.rs
    git commit -m "refactor(filesys): default WriteOptions to atomic writes"

Optionally extend the commit body with a short rationale (why the default was flipped; that production is unaffected). Do not push. Do not open the PR from within this plan — the orchestrator's `$pr` step owns that.

After the commit, confirm the working tree is clean:

    git status --short

Expected transcript: empty output.

## Validation and Acceptance

**Behavioral acceptance.** After this change, the following Rust snippet (conceptually, inside a `#[test]`) holds:

    let opts = WriteOptions::default();
    assert_eq!(opts.overwrite, Overwrite::Deny);
    assert_eq!(opts.atomic, Atomic::Yes);

That is exactly what the updated `tests::filesys::path::write_options::default` test asserts.

**Test acceptance.** Run from the `agent/` repo root:

    ./scripts/test.sh

Expect the entire crate test suite to pass (`test result: ok`). Specifically:

- `tests::filesys::path::write_options::default` passes after the change. This test would have failed if only Edit 1 had been applied and Edit 2 were missing — it is the regression guard for the flip.
- The three `Atomic::No` tests in `tests::filesys::file` (around lines 472, 578, 683) still pass, proving the non-atomic branch in `File::write_bytes` is still wired up and still works.
- The ~50 tests that use `WriteOptions::default()` for fixture writes still pass, proving that switching the default mechanism from non-atomic to atomic is behaviorally transparent to them.

**Lint acceptance.** Run from the `agent/` repo root:

    ./scripts/lint.sh

Expect clean exit.

**Hard-stop preflight gate.** Before any PR is opened for this change, a fresh-context `$preflight` run (the preflight skill) must report status **clean**. This is a non-negotiable gate: do not open a PR if preflight reports findings. `$preflight` is expected to run `./scripts/test.sh`, `./scripts/lint.sh`, and `./scripts/covgate.sh` among other checks; any failures or warnings are a stop-the-line. If preflight surfaces findings, refine this plan (add discoveries to Surprises & Discoveries, decisions to Decision Log) and re-run until clean.

**Out-of-scope coverage note.** This change does not add or remove code paths, so it should not affect line coverage. `./scripts/covgate.sh` should report no coverage regression. If it does, investigate before opening the PR — that would be a surprise worth recording.

## Idempotence and Recovery

Every step in this plan is safe to repeat.

- **Edit 1 and Edit 2** are idempotent textual substitutions. If the edit has already been applied, re-applying the same change is a no-op (`git diff` will show no new modifications). If you are unsure whether an edit stuck, run `git diff agent/src/filesys/mod.rs agent/tests/filesys/path.rs` to inspect the current state.

- **`./scripts/test.sh`** and **`./scripts/lint.sh`** are both read-only with respect to the working tree and can be re-run freely.

- **Commit rollback**: if the commit was made but tests or lint later fail unexpectedly, revert the commit with `git revert HEAD` (preferred, preserves history) or, if not yet pushed and you want to redo the commit from scratch, `git reset --soft HEAD~1` to uncommit while keeping the changes staged. Do not use `git reset --hard` — that would discard the working tree.

- **Full rollback to `main`**: the working branch `refactor/atomic-writes` started identical to `main`, and before implementation begins it will contain one commit adding this plan file. A full rollback to `origin/main` (`git reset --hard origin/main`) would drop both the implementation commit and the plan-file commit, which is usually too aggressive. Prefer `git revert` of just the implementation commit; or, if you want to start over but keep the plan file, `git reset --hard <plan-commit-sha>`. Only `git reset --hard origin/main` if you intend to discard the plan file as well. A safer alternative is to abandon the branch and create a new one from `main`.

- **Preflight failure recovery**: if `$preflight` finds issues, do not bypass the gate. Update this plan's Surprises & Discoveries and Decision Log sections, address the finding, and re-run `$preflight` until it reports clean. Only then open the PR.
