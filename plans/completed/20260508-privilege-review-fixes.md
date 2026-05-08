# Apply privilege module review fixes

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | All edits land in `agent/src/privilege/`, `agent/tests/privilege/`, the workspace `Cargo.toml`, and `agent/Cargo.toml`. |

This plan lives in `agent/plans/` because every code change is in this repo. Work is performed on the existing branch `feat/self-privilege-drop` (currently 3 commits ahead of `origin/feat/self-privilege-drop`); the existing PR #62 will be updated by pushing to this branch — no new PR is opened.

## Purpose / Big Picture

PR #62 introduced a self-privilege-drop: when `miru-agent` starts as root it drops to the `miru` user, and when it starts as a non-root user it verifies it is already running as `miru`. A code review surfaced nine concrete issues that weaken correctness, hide failure modes, leak public surface, or pull in an avoidable workspace dependency. After this plan is implemented:

- The `verify_effective_user` path no longer silently swallows a missing `miru` passwd entry — `UserNotFound` propagates to the caller and is surfaced to the user.
- Effective-user verification checks both uid **and** gid, so a host where `miru`'s gid drifted from its uid does not pass verification by accident.
- The privileged drop uses `setresuid`/`setresgid` (not just `setuid`/`setgid`) and verifies all three of real/effective/saved ids match the target after the drop, eliminating a class of "saved uid still 0" footguns.
- A `debug_assert!` in `drop_to` documents and enforces the "must be root when called" precondition in debug builds.
- The privilege module's public surface is reduced to what callers outside `agent/src/` actually need (`run_as` and the `PrivilegeErr` enum); helpers (`is_root_user`, `lookup_user`, `User`) become `pub(crate)`.
- The workspace stops depending on the `libc` crate; `nix` already provides equivalent calls.
- A new no-op test exercises `verify_effective_user`'s happy path (current effective user equals the target), so every CI runner exercises the success branch without needing root.

A reviewer on PR #62 can run `./scripts/preflight.sh` and observe `Preflight clean`, run `cargo test -p miru-agent privilege` and see the new and existing tests pass, and read the diff to see the swallowed-error arm and `setuid`/`setgid` calls replaced.

## Progress

- [ ] Milestone 1 — Test additions (Fix 1, Fix 2)
- [ ] Milestone 2 — `verify_effective_user` correctness + `WrongUser` gid extension (Fixes 3, 4)
- [ ] Milestone 3 — `drop_to` hardening: `debug_assert!`, `setresuid`/`setresgid`, `PostDropMismatch` triple + `u32` fields (Fixes 5, 6, 7)
- [ ] Milestone 4 — Surface reduction + `libc` removal (Fixes 8, 9)

Use timestamps (UTC) when checking off steps. Split partially completed work into "done" vs "remaining" if a milestone is interrupted.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go.)

- Decision: …
  Rationale: …
  Date/Author: …

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent repo (`mirurobotics/agent`) is a Cargo workspace at `/home/ben/miru/workbench1/repos/agent/`. The binary crate is `agent/` (package name `miru-agent`). The relevant files for this plan are:

- `agent/src/privilege/mod.rs` — public entry point `pub fn run_as(name: &str)`. Internally calls `is_root_user`, `lookup_user`, `verify_effective_user`, and `drop_to`. Uses `nix::unistd::{getegid, geteuid, initgroups, setgid, setuid}` plus `nix::unistd::User` re-exported as `pub type User`.
- `agent/src/privilege/errors.rs` — defines `pub enum PrivilegeErr` with variants `UserNotFound`, `WrongUser`, `Syscall`, `PostDropMismatch`. `PostDropMismatch` currently uses `nix::unistd::Uid`/`Gid` field types; the others use `u32` / `String`.
- `agent/tests/privilege/mod.rs` — integration tests. Currently has four tests: `lookup_user_returns_root_for_root`, `lookup_user_returns_user_not_found_for_nonexistent`, `run_as_rejects_non_target_user_when_not_root`, and `privilege_err_display_messages_are_human_readable`. Uses `unsafe { libc::getuid() }` once; this is the only `libc::` call in the agent crate.
- `Cargo.toml` (workspace root) — lists `libc = "0.2"` and `nix = { version = "0.31.2", default-features = false, features = ["user"] }` in `[workspace.dependencies]`.
- `agent/Cargo.toml` — pulls both via `libc = { workspace = true }` and `nix = { workspace = true }`.
- `scripts/preflight.sh` — runs lint and tests in parallel for both the agent crate and `tools/lint`. Final line on success is `Preflight clean`.
- `scripts/test.sh` — wraps `RUST_LOG=off cargo test --features test`.
- `scripts/lint.sh` — runs the custom import linter, `cargo fmt --check`, machete/diet (unused dep check), `cargo audit`, and `cargo clippy --all-features -- -D warnings`.

Definitions:

- **euid / egid**: effective uid / gid — the credentials the kernel uses for permission checks against this process.
- **real / effective / saved uid**: POSIX defines three uids per process. `setuid` only touches some of them depending on whether the caller is root; `setresuid` sets all three explicitly. The same applies to gids and `setresgid`.
- **`getresuid` / `getresgid`**: `nix::unistd::getresuid()` returns a `ResUid` struct with fields `real`, `effective`, `saved` (all `Uid`). `getresgid()` returns the same shape with `Gid` fields. Both are gated behind `nix`'s `user` feature, which the workspace already enables.
- **`PrivilegeErr::PostDropMismatch`**: returned when post-drop verification of credentials fails. Today it carries only `expected_uid`, `expected_gid`, `actual_uid`, `actual_gid` and only checks egid/euid. After Fix 6 it must also assert real and saved match.
- **smoke tests**: shell-driven privileged tests run outside `cargo test`, documented in the completed plans `agent/plans/completed/20260507-self-privilege-drop.md`, `20260507-privilege-swap-to-nix.md`, and `20260507-privilege-drop-cfg-gating.md`. These cover the "actually run as root and drop" scenario; the integration tests in `agent/tests/privilege/mod.rs` deliberately do not duplicate that.

Branch state at plan creation: `feat/self-privilege-drop` at HEAD `0b5fe50`, working tree clean, base `main`, open PR #62.

## Plan of Work

The fixes group into four atomic-commit milestones. Each milestone ends with a single commit so reviewers and `git bisect` can isolate the change.

### Milestone 1 — Test additions

**Fix 1 — `run_as` already-target-user no-op test.** In `agent/tests/privilege/mod.rs`, add a new `#[test]` that resolves the current effective user via `nix::unistd::User::from_uid(nix::unistd::geteuid())`, then calls `privilege::run_as(&user.name)` and asserts `Ok(())`. This exercises the `verify_effective_user` happy path on every CI runner without needing root. The test must handle `User::from_uid` returning `Ok(None)` or an error gracefully (skip with an `eprintln!` and `return` rather than panicking) so it never flakes on minimal runners; document this with a brief comment.

**Fix 2 — `drop_to` privileged test (lighter approach).** Do **not** add a privileged subprocess test. Instead, add a brief comment near the top of `agent/tests/privilege/mod.rs` stating that `drop_to` is covered by smoke-test steps in the three completed plans listed in *Context and Orientation* (cite their full paths). One short paragraph; no code.

### Milestone 2 — `verify_effective_user` correctness + `WrongUser` gid extension

**Fix 3 — `verify_effective_user` propagates `UserNotFound`.** In `agent/src/privilege/mod.rs`, replace the current body of `verify_effective_user` with:

    let user = lookup_user(name)?;
    if user.uid != geteuid() || user.gid != getegid() {
        return Err(PrivilegeErr::WrongUser {
            expected: name.to_string(),
            actual_uid: geteuid().as_raw(),
            actual_gid: getegid().as_raw(),
            expected_uid: user.uid.as_raw(),
            expected_gid: user.gid.as_raw(),
            argv0: std::env::args().next().unwrap_or_else(|| "miru-agent".into()),
            trace: trace!(),
        });
    }
    Ok(())

The previous behavior swallowed `Err(PrivilegeErr::UserNotFound { .. })` into a boolean `false` and then returned `WrongUser`, hiding the root cause. The new shape uses `?` to propagate `UserNotFound` directly; the existing test `run_as_rejects_non_target_user_when_not_root` already accepts both `WrongUser` and `UserNotFound` outcomes so it must continue to pass.

**Fix 4 — `verify_effective_user` checks gid too.** Combined with Fix 3 above (the `||` clause). In `agent/src/privilege/errors.rs`, extend `PrivilegeErr::WrongUser` to add `actual_gid: u32, expected_uid: u32, expected_gid: u32` and update its `#[error("...")]` Display message to include both uid and gid pairs. Suggested message:

    miru-agent must be run as root or the '{expected}' user, but is running as \
    uid {actual_uid} gid {actual_gid} (expected uid {expected_uid} gid {expected_gid}).\n\
    Try: sudo {argv0} ...

Update construction sites in `agent/src/privilege/mod.rs` (the only one is in `verify_effective_user` after Fix 3). Update the existing Display test `privilege_err_display_messages_are_human_readable` in `agent/tests/privilege/mod.rs` to populate the new fields and assert the message contains the new gid information. Update the destructuring in `run_as_rejects_non_target_user_when_not_root` to keep the test passing — it already uses `..` for unmatched fields, so only the asserted fields need attention.

### Milestone 3 — `drop_to` hardening

**Fix 5 — `drop_to` debug_assert euid=0 precondition.** At the top of `fn drop_to` in `agent/src/privilege/mod.rs`, add:

    debug_assert!(
        geteuid() == nix::unistd::Uid::from_raw(0),
        "drop_to requires euid=0",
    );

Update the rustdoc on `drop_to` to state: "Caller invariant: `geteuid() == 0`. Enforced by `debug_assert!` in debug builds." Keep the rustdoc on `run_as` accurate — it already documents the root-vs-non-root branch.

**Fix 6 — `setresuid`/`setresgid` and verify all three uids/gids.** In `agent/src/privilege/mod.rs`:

- Replace `use nix::unistd::{getegid, geteuid, initgroups, setgid, setuid};` with `use nix::unistd::{getegid, geteuid, getresgid, getresuid, initgroups, setresgid, setresuid};`. (`setgid`/`setuid` are removed; `getegid`/`geteuid` stay because `verify_effective_user` still uses them.)
- In `drop_to`, the syscall sequence becomes (still after `initgroups`): `setresgid(target.gid, target.gid, target.gid)` then `setresuid(target.uid, target.uid, target.uid)`. Order: `initgroups` → `setresgid` → `setresuid`.
- Replace the two-line post-drop check with a `getresuid()`/`getresgid()` pair and assert real, effective, and saved each match `target.uid` / `target.gid`. On mismatch, return `PrivilegeErr::PostDropMismatch` populated with the full triple.

**Fix 7 — `PostDropMismatch` carries the full triple, fields are `u32`.** In `agent/src/privilege/errors.rs`, replace `PostDropMismatch` with:

    #[error(
        "post-drop verification failed: expected uid={expected_uid} gid={expected_gid}, got \
         ruid={actual_ruid} euid={actual_euid} suid={actual_suid} \
         rgid={actual_rgid} egid={actual_egid} sgid={actual_sgid}"
    )]
    PostDropMismatch {
        expected_uid: u32,
        expected_gid: u32,
        actual_ruid: u32,
        actual_euid: u32,
        actual_suid: u32,
        actual_rgid: u32,
        actual_egid: u32,
        actual_sgid: u32,
        trace: Box<Trace>,
    },

All fields are `u32` (the current `nix::unistd::Uid`/`Gid` types are removed from this struct). Construction sites in `agent/src/privilege/mod.rs` use `.as_raw()` on the `Uid`/`Gid` returned from `getresuid()`/`getresgid()` and `target.uid` / `target.gid`. Update the Display test in `agent/tests/privilege/mod.rs` to construct the new shape with `u32` literals and assert the message contains `ruid=`, `euid=`, `suid=`, `rgid=`, `egid=`, `sgid=` and the expected/got uid/gid values.

### Milestone 4 — Surface reduction + `libc` removal

**Fix 8 — Reduce `pub` surface.** In `agent/src/privilege/mod.rs`:

- Change `pub fn is_root_user` → `pub(crate) fn is_root_user`.
- Change `pub fn lookup_user` → `pub(crate) fn lookup_user`.
- Change `pub type User = nix::unistd::User;` → `pub(crate) type User = nix::unistd::User;`.
- `pub fn run_as` and `pub use self::errors::PrivilegeErr;` stay public — `run_as` is the only external entry point.

Move the two integration tests `lookup_user_returns_root_for_root` and `lookup_user_returns_user_not_found_for_nonexistent` from `agent/tests/privilege/mod.rs` into a new `#[cfg(test)] mod tests { ... }` block at the bottom of `agent/src/privilege/mod.rs` so they can keep calling the now-`pub(crate)` `lookup_user`. Inside that block, import via `use super::*;` and reference `PrivilegeErr` directly (not via `crate::privilege::PrivilegeErr`).

Keep `run_as_rejects_non_target_user_when_not_root`, `privilege_err_display_messages_are_human_readable`, and the new Fix 1 test in the integration test file — they only call `pub fn run_as` and the public `PrivilegeErr` variants.

**Fix 9 — Drop workspace `libc` dep.** In `agent/tests/privilege/mod.rs`, replace the single `unsafe { libc::getuid() }` call (currently in `run_as_rejects_non_target_user_when_not_root`) with `nix::unistd::getuid().as_raw()` — `getuid` is safe under `nix`. Remove `libc = "0.2"` from `Cargo.toml` `[workspace.dependencies]`. Remove `libc = { workspace = true }` from `agent/Cargo.toml` `[dependencies]`. Run `cargo check -p miru-agent` to confirm nothing else depends on `libc`.

If `cargo machete` (run by `scripts/lint.sh`) flags any other unrelated unused dep, that is out of scope — leave it. The only entry being removed in this milestone is `libc`.

## Concrete Steps

All commands run from `/home/ben/miru/workbench1/repos/agent` unless otherwise stated.

### Pre-flight

1. Confirm branch:

       git rev-parse --abbrev-ref HEAD

   Expected: `feat/self-privilege-drop`. If not, switch to it.

2. Confirm working tree is clean:

       git status

   Expected: `nothing to commit, working tree clean`.

3. Baseline preflight:

       ./scripts/preflight.sh

   Expected last line: `Preflight clean`. If it does not pass on the unmodified branch, stop and investigate before editing anything.

### Milestone 1 — Test additions

4. Edit `agent/tests/privilege/mod.rs`:
   - Add a top-of-file comment paragraph (after the existing `use` block) noting that `drop_to` is covered by smoke-test steps documented in `agent/plans/completed/20260507-self-privilege-drop.md`, `agent/plans/completed/20260507-privilege-swap-to-nix.md`, and `agent/plans/completed/20260507-privilege-drop-cfg-gating.md` (Fix 2).
   - Add a `#[test] fn run_as_is_noop_when_already_target_user()` that resolves the current effective user via `nix::unistd::User::from_uid(nix::unistd::geteuid())`, gracefully skips on `Ok(None)`/`Err(_)` with an `eprintln!`, and otherwise asserts `privilege::run_as(&user.name).expect("run_as on current effective user is a no-op")` returns `Ok(())` (Fix 1).

5. Run scoped tests:

       ./scripts/test.sh -- privilege

   (Or `RUST_LOG=off cargo test --features test -p miru-agent privilege`.) Expected: all privilege tests pass.

6. Run lint and full preflight:

       ./scripts/lint.sh
       ./scripts/preflight.sh

   Expected last line: `Preflight clean`.

7. Commit milestone 1:

       git add agent/tests/privilege/mod.rs
       git commit -m "test(privilege): add already-target-user no-op test and document drop_to smoke coverage"

### Milestone 2 — `verify_effective_user` + `WrongUser` gid

8. Edit `agent/src/privilege/errors.rs`:
   - Extend `PrivilegeErr::WrongUser` with `actual_gid: u32, expected_uid: u32, expected_gid: u32` fields.
   - Update its `#[error("...")]` Display string to include `gid {actual_gid}` and `expected uid {expected_uid} gid {expected_gid}` (Fix 4).

9. Edit `agent/src/privilege/mod.rs`, `fn verify_effective_user`:
   - Replace its body with the `let user = lookup_user(name)?;` + `if user.uid != geteuid() || user.gid != getegid()` shape that constructs the extended `WrongUser` (Fixes 3 + 4).

10. Edit `agent/tests/privilege/mod.rs`:
    - Update `privilege_err_display_messages_are_human_readable`'s `WrongUser` construction to populate the new `actual_gid`, `expected_uid`, `expected_gid` fields and assert the formatted string contains both new pieces of information.
    - Confirm `run_as_rejects_non_target_user_when_not_root` still compiles — it uses `..` to ignore unmatched fields.

11. Re-run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/lint.sh
        ./scripts/preflight.sh

    Expected: all green; `Preflight clean`.

12. Commit milestone 2:

        git add agent/src/privilege/errors.rs agent/src/privilege/mod.rs agent/tests/privilege/mod.rs
        git commit -m "fix(privilege): propagate UserNotFound and verify gid in verify_effective_user"

### Milestone 3 — `drop_to` hardening

13. Edit `agent/src/privilege/errors.rs`:
    - Replace `PostDropMismatch` with the full-triple `u32` shape from Fix 7. Update the `#[error("...")]` Display string to include `ruid=`, `euid=`, `suid=`, `rgid=`, `egid=`, `sgid=` and the expected uid/gid.

14. Edit `agent/src/privilege/mod.rs`:
    - Replace the `setgid`/`setuid` imports with `setresgid`/`setresuid` and add `getresgid`/`getresuid` to the `nix::unistd` import line (Fix 6).
    - Add the `debug_assert!(geteuid() == nix::unistd::Uid::from_raw(0), "drop_to requires euid=0");` at the top of `fn drop_to` (Fix 5). Update the rustdoc.
    - Replace the two `setgid`/`setuid` calls with `setresgid(target.gid, target.gid, target.gid)` and `setresuid(target.uid, target.uid, target.uid)`. Order: `initgroups` → `setresgid` → `setresuid`.
    - Replace the post-drop check with `getresuid()` + `getresgid()` and a `PrivilegeErr::PostDropMismatch` branch populated via `.as_raw()` on every uid/gid (Fix 6 + 7).

15. Edit `agent/tests/privilege/mod.rs`:
    - Update the `PostDropMismatch` construction in `privilege_err_display_messages_are_human_readable` to use the new `u32` fields. Assert the message contains `expected uid=`, `ruid=`, `euid=`, `suid=`, `rgid=`, `egid=`, `sgid=`.
    - Drop the `use nix::unistd::{Gid, Uid};` import if it becomes unused after switching to `u32` literals.

16. Re-run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/lint.sh
        ./scripts/preflight.sh

    Expected: all green; `Preflight clean`.

17. Commit milestone 3:

        git add agent/src/privilege/errors.rs agent/src/privilege/mod.rs agent/tests/privilege/mod.rs
        git commit -m "fix(privilege): use setresuid/setresgid and verify full uid/gid triple after drop"

### Milestone 4 — Surface reduction + `libc` removal

18. Edit `agent/src/privilege/mod.rs`:
    - Change `pub fn is_root_user`, `pub fn lookup_user`, and `pub type User` to `pub(crate)` (Fix 8).
    - Add `#[cfg(test)] mod tests { use super::*; ... }` at the bottom containing the migrated `lookup_user_returns_root_for_root` and `lookup_user_returns_user_not_found_for_nonexistent` tests. Inside the block, reference `PrivilegeErr` directly (no `crate::privilege::` qualifier).

19. Edit `agent/tests/privilege/mod.rs`:
    - Remove the two migrated tests (`lookup_user_returns_root_for_root`, `lookup_user_returns_user_not_found_for_nonexistent`).
    - Replace `unsafe { libc::getuid() }` in `run_as_rejects_non_target_user_when_not_root` with `nix::unistd::getuid().as_raw()` (Fix 9). Remove the `unsafe` block; `nix::unistd::getuid` is safe.

20. Edit `Cargo.toml` (workspace root): remove the `libc = "0.2"` line from `[workspace.dependencies]`.

21. Edit `agent/Cargo.toml`: remove the `libc = { workspace = true }` line from `[dependencies]`.

22. Verify nothing else references the dropped dep:

        cargo check -p miru-agent

    Expected: builds without complaint. If it errors on a missing `libc::` reference, search the agent crate (`grep -rn "libc::" agent/`) and address that file before continuing.

23. Re-run lint, scoped tests, and preflight:

        ./scripts/lint.sh
        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: `cargo machete` does not flag `libc` (it has been removed); all privilege tests pass; `Preflight clean`.

24. Commit milestone 4:

        git add agent/src/privilege/mod.rs agent/tests/privilege/mod.rs Cargo.toml agent/Cargo.toml Cargo.lock
        git commit -m "refactor(privilege): tighten pub surface and drop workspace libc dep"

### Push

25. Push the four new commits to the existing branch (no new PR — PR #62 picks them up automatically):

        git push origin feat/self-privilege-drop

    Expected: `git status` reports `Your branch is up to date with 'origin/feat/self-privilege-drop'`.

## Validation and Acceptance

The plan is complete when **all** of the following hold from `/home/ben/miru/workbench1/repos/agent`:

- `cargo fmt --check` exits 0 from the workspace root (no diffs).
- `cargo clippy --all-targets -- -D warnings` exits 0 from the workspace root (no warnings, no errors).
- `cargo test -p miru-agent privilege` (or `./scripts/test.sh -- privilege`) passes — all four pre-existing tests plus the new `run_as_is_noop_when_already_target_user` test, including the two relocated `lookup_user_*` tests now living inside the source module.
- `cargo build -p miru-agent` succeeds.
- `./scripts/preflight.sh` final line is `Preflight clean`.
- `grep -rn "libc::" agent/` returns no matches (the only previous match was the now-removed test line).
- `git log --oneline origin/feat/self-privilege-drop..HEAD` shows exactly four new commits, one per milestone, with messages matching the templates in *Concrete Steps*.

**Behavioral acceptance for reviewers**:

- Compile `miru-agent` in debug mode and call `drop_to` with euid != 0 from a unit test stub or REPL: it panics with `drop_to requires euid=0` (Fix 5). This is verified by code inspection — no test is required.
- Inspect the diff: the `Err(PrivilegeErr::UserNotFound { .. }) => false` arm in `verify_effective_user` is gone; `setuid`/`setgid` are gone; `setresuid`/`setresgid`/`getresuid`/`getresgid` are present; `PostDropMismatch` carries six `actual_*` fields.
- Run `cargo doc -p miru-agent --no-deps` and confirm `is_root_user`, `lookup_user`, and `User` no longer appear in the public docs of the `privilege` module.

**Acceptance gate**: the orchestrator must observe `Preflight clean` before pushing the branch. If preflight fails after any milestone, fix the cause inside that milestone's commit (amend) — do not stack a fixup commit on top.

## Idempotence and Recovery

- All edits are pure-source changes; reapplying any milestone is safe.
- Each milestone ends with a single commit. To redo a milestone, run `git reset --soft HEAD~1` (re-stages the changes for editing) and re-run from step N. Do not use `--hard` unless you have already pushed; the orchestrator will commit, not the implementer.
- If `./scripts/preflight.sh` fails partway through a milestone, the failure log is split between `=== Lint ===`, `=== Tests ===`, `=== Tools Lint ===`, and `=== Tools Tests ===` sections — read all four. Fix and re-run preflight before committing.
- The `libc` removal in milestone 4 is the only step that can break unrelated callers. `cargo check -p miru-agent` (step 22) catches that immediately. If it fails, restore `libc = { workspace = true }` in `agent/Cargo.toml`, fix the unexpected caller in a separate change, and resume.
- Pushing in step 25 is fast-forward only because the branch already tracks `origin/feat/self-privilege-drop`. No `--force` is needed; do not use `--force` or `--force-with-lease` unless explicitly directed.
