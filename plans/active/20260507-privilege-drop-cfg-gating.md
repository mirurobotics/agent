# Drop `cfg(target_os = "linux")` gating from the privilege module

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Remove `cfg(target_os = "linux")` gating and the non-Linux stubs from `agent/agent/src/privilege/mod.rs`. Optionally drop now-unnecessary `cfg(target_os = "linux")` gates on tests in `agent/agent/tests/privilege/mod.rs`. Re-measure coverage; adjust `.covgate` only if necessary. |

This plan lives in `agent/plans/` because the only files touched are inside this repo (`/home/ben/miru/workbench1/repos/agent`). No other Miru repo is read or written. The work updates the open PR `mirurobotics/agent#62` on branch `feat/self-privilege-drop` (base: `main`) — it does not open a new PR. The PR is updated by Step 4 of the orchestrator after preflight is clean.

## Purpose / Big Picture

`agent/agent/src/privilege/mod.rs` today carries `#[cfg(target_os = "linux")]` gates around the privilege-drop body and ships matching `#[cfg(not(target_os = "linux"))]` stubs (`lookup_user` returns `UserNotFound`; `ensure_dropped_or_already_unprivileged` returns `Ok(())`). The justification recorded in the swap-to-nix Decision Log was: keep cross-platform `cargo test` compiling for devs on macOS.

That justification no longer holds against the actual repo state:

- `cfg(target_os` appears nowhere else in `agent/agent/src/`. Every other module is implicitly Linux-only via its dependencies. (Verified at plan-write time.)
- `.github/workflows/*.yml` only runs `ubuntu-latest`. There is no macOS CI lane.
- `AGENTS.md` and `README.md` make no mention of macOS as a supported dev environment.
- `nix::unistd::{User::from_name, setuid, setgid, geteuid, getegid}` are cross-platform; the `__errno_location` hazard that motivated stubs in the libc-direct era is gone.

The previous justification for stubbing was speculative cross-platform compile fidelity for a workflow that nobody runs. This plan drops the stubs and the gates: the module becomes a single platform-agnostic implementation that targets POSIX systems; production deploys to Linux.

User-visible acceptance: `sudo MIRU_PROVISIONING_TOKEN=... miru-agent provision ...` and `systemctl restart miru.service` continue to behave as they do on the swap-to-nix branch today. The branch's existing 7 privilege tests continue to pass on Linux.

## Progress

- [ ] M1: Delete `cfg(target_os = "linux")` attributes and the non-Linux stub functions in `agent/agent/src/privilege/mod.rs`. Replace inline doc comments that reference "non-Linux ... stub" with comments reflecting the new POSIX-on-Linux reality. Verify with `cargo build -p miru-agent` from `/home/ben/miru/workbench1/repos/agent`.
- [ ] M2: Decide and apply the test-side cfg policy in `agent/agent/tests/privilege/mod.rs` (drop the three `#[cfg(target_os = "linux")]` gates per Decision Log "Test-side cfg gates"). Run `./scripts/test.sh -- privilege` and confirm 7 tests pass.
- [ ] M3: Re-measure coverage on `agent/src/privilege` via `./scripts/preflight.sh`. Update `.covgate` only if the measured value drops below the current `30`; otherwise leave the gate alone (do NOT regress the gate; do NOT pre-emptively raise it as part of this plan). Run `./scripts/preflight.sh` and confirm the final line is `Preflight clean`.

Use timestamps when you complete steps.

## Surprises & Discoveries

(Empty at plan-write time. Add timestamped entries when implementation deviates from this plan.)

## Decision Log

- Decision: Fully drop the `cfg(target_os = "linux")` gates and the non-Linux stubs, even though `nix::unistd::initgroups` is gated `cfg(not(apple_targets))` in the upstream `nix` crate (verified by reading `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.31.2/src/unistd.rs:2090` at plan-write time — the attribute is `#[cfg(not(any(apple_targets, target_os = "redox", target_os = "haiku", target_os = "emscripten")))]`). After this change, `cargo build --target x86_64-apple-darwin -p miru-agent` will fail with an unresolved-symbol error on `initgroups`. That is acceptable: there is no macOS CI lane, no documented macOS dev support, and no other module in `agent/agent/src/` carries portability shims. The other five `nix::unistd` items we use (`User::from_name`, `setuid`, `setgid`, `geteuid`, `getegid`) are unconditionally available in `nix 0.31.2`; only `initgroups` is excluded on Apple targets.
  Rationale: The user's brief explicitly framed the stubs as dead code for a use case nobody exercises. Picking Option B (a tighter `cfg` like `cfg(not(apple_targets))`) would preserve a partial macOS build for no benefit — `cargo test` would still fail to link `lookup_user_returns_root_for_root` etc. on macOS, and the `cfg` would still be a maintenance hazard. Picking Option C (no `cfg`, accept that macOS builds break) matches the rest of the crate.
  Date/Author: 2026-05-07 / plan author.

- Decision: Test-side cfg gates — **drop** all three `#[cfg(target_os = "linux")]` attributes in `agent/agent/tests/privilege/mod.rs` (lines 20, 41, 73 today: `lookup_user_returns_root_for_root`, `ensure_dropped_or_already_unprivileged_when_running_as_self_is_ok`, `lookup_user_returns_user_not_found_when_name_contains_null_byte`).
  Rationale: With M1 removing the source-side cfg gates, the production code becomes platform-agnostic-on-POSIX. The test-side gates were placed defensively in the original task because the source had stubs returning `UserNotFound` on non-Linux, which would cause `lookup_user_returns_root_for_root` to fail on macOS. After M1, that concern is gone: on any POSIX host, `root` exists with uid 0, the agent process is non-root and non-`miru` on dev/CI, and the existing test bodies remain correct. Keeping the test-side gates after dropping the source-side gates would be inconsistent — the entire policy of this plan is "we only ship and test on Linux". Either everything is gated or nothing is. The latter is simpler.
  Caveat: this means dropping gates from the test bodies, even though `cargo test --target x86_64-apple-darwin` would still fail at compile time because the source-side `initgroups` import is unresolved. That failure is by design — see the previous Decision Log entry.
  Date/Author: 2026-05-07 / plan author.

- Decision: Do **not** rename the `getpwnam_r` string label inside `PrivilegeErr::Syscall { call }` to anything platform-neutral. The label is internal diagnostic text and the underlying syscall is `getpwnam_r(3)` on Linux; on macOS / BSDs the same name is used in libc and POSIX. Keep it stable.
  Rationale: Avoid churning user-visible error messages for a refactor whose stated scope is removing cfg gates.
  Date/Author: 2026-05-07 / plan author.

- Decision: Coverage policy — re-measure but do not pre-emptively raise the gate. The current `.covgate` value is `30` (set by the swap-to-nix plan, three points below the measured ~33% region coverage on a non-root, non-`miru` runner). Removing ~15 lines of stub code may marginally raise or marginally lower the percent depending on how cargo-llvm-cov counts the deleted lines. M3 measures the post-change value and only edits `.covgate` if preflight fails.
  Rationale: The plan's brief explicitly said "do NOT regress the gate" and "adjust only if necessary". Pre-emptively raising the gate as part of this work would couple a coverage ratchet to a refactor that has nothing to do with test reach. If a future change extends test coverage of the FFI path (e.g. via a mockable trait), raise the gate then.
  Date/Author: 2026-05-07 / plan author.

- Decision: Inline doc-comment replacements — the old `lookup_user` and `ensure_dropped_or_already_unprivileged` doc comments mention "On non-Linux targets ... this is a stub". Replace with text that reflects the new reality: "Production deploys to Linux; the implementation works on any POSIX system that exposes `getpwnam_r` and the standard credential syscalls." Do not promise macOS support and do not document `cargo build --target *-apple-darwin` as expected to work.
  Rationale: We are deliberately not extending the support contract. The inline comment should describe what the code does, not what hypothetical platforms it might serve.
  Date/Author: 2026-05-07 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Crate under change: `miru-agent` at `agent/agent/`. Target file: `agent/agent/src/privilege/mod.rs` (currently 209 lines, zero `unsafe` blocks after the libc-to-nix swap). Public surface re-exported from `agent/agent/src/lib.rs:17` and consumed from `agent/agent/src/main.rs` via `use miru_agent::privilege;`.

**Today's `mod.rs` shape** (reading `/home/ben/miru/workbench1/repos/agent/agent/src/privilege/mod.rs`):

- Lines 2, 10, 12: `#[cfg(target_os = "linux")]` on the `std::ffi::CString` import, the `nix::errno::Errno` import, and the `nix::unistd::{...}` import.
- Line 76: `#[cfg(target_os = "linux")]` on the real `lookup_user`.
- Line 117: `#[cfg(not(target_os = "linux"))]` on the stub `lookup_user` (lines 117–123, 7 lines).
- Line 133: `#[cfg(target_os = "linux")]` on the real `ensure_dropped_or_already_unprivileged`.
- Line 205: `#[cfg(not(target_os = "linux"))]` on the stub `ensure_dropped_or_already_unprivileged` (lines 205–208, 4 lines).
- Lines 73–75: doc comment paragraph "On non-Linux targets (e.g. macOS dev machines) this is a stub that always returns `UserNotFound` so cross-platform `cargo test` keeps compiling. The agent only ships on Linux." — needs a rewrite.
- Lines 115–116: doc comment "Non-Linux stub: the binary does not ship on these platforms, but tests must compile." — deleted with the stub.
- Lines 203–204: doc comment "Non-Linux stub: privilege drop is a no-op on platforms the agent does not ship on." — deleted with the stub.

After M1, the file should have:

- Three `nix`-related imports (no `#[cfg]`), one `std::ffi::CString` import (no `#[cfg]`), and the existing internal-crate imports.
- One `lookup_user` function (no `#[cfg]`), one `ensure_dropped_or_already_unprivileged` function (no `#[cfg]`).
- A revised doc comment on `lookup_user` describing the POSIX-on-Linux reality (no mention of stubs).
- A revised doc comment on `ensure_dropped_or_already_unprivileged` that keeps the verbatim "Note on environment: ... setuid(2) and setgid(2) syscalls only mutate process credentials" paragraph (load-bearing per the `privilege_err_display_messages_are_human_readable` test indirectly and per the swap-to-nix plan's preserve-verbatim constraint), and drops any non-Linux references.

**Today's tests** (`/home/ben/miru/workbench1/repos/agent/agent/tests/privilege/mod.rs`, 144 lines, 7 tests):

1. `target_user_and_target_group_are_miru` — no `cfg`. Cross-platform.
2. `lookup_user_returns_root_for_root` — `#[cfg(target_os = "linux")]` at line 20. Asserts `root` exists with uid 0; true on macOS too.
3. `lookup_user_returns_user_not_found_for_nonexistent` — no `cfg`. Cross-platform.
4. `ensure_dropped_or_already_unprivileged_when_running_as_self_is_ok` — `#[cfg(target_os = "linux")]` at line 41. Tolerates either `WrongUser` or `UserNotFound` outcomes.
5. `lookup_user_returns_user_not_found_when_name_contains_null_byte` — `#[cfg(target_os = "linux")]` at line 73. Pre-checks NUL and asserts `UserNotFound`.
6. `user_info_struct_round_trips_fields` — no `cfg`. Cross-platform.
7. `privilege_err_display_messages_are_human_readable` — no `cfg`. Cross-platform.

After M2, all three remaining `#[cfg(target_os = "linux")]` lines (20, 41, 73) are deleted. No test body changes.

**Other call sites (out of scope, listed for orientation):**

- `agent/agent/src/main.rs:40` (or thereabouts): calls `privilege::ensure_dropped_or_already_unprivileged()`. Not edited.
- `agent/agent/src/lib.rs:17`: `pub mod privilege;`. Not edited.
- `agent/agent/tests/mod.rs`: contains `pub mod privilege;` registering the test sub-module. Not edited.

**`nix` crate macOS gating** (load-bearing for the Decision Log):

`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.31.2/src/unistd.rs`:

- Line 1771: `pub fn geteuid() -> Uid` — no `cfg`.
- Line 1791: `pub fn getegid() -> Gid` — no `cfg`.
- Line 1819: `pub fn setuid(uid: Uid)` — no `cfg`.
- Line 1829: `pub fn setgid(gid: Gid)` — no `cfg`.
- Line 2090: `#[cfg(not(any(apple_targets, target_os = "redox", target_os = "haiku", target_os = "emscripten")))]` on `pub fn initgroups`.
- Line 3700: `#[cfg(not(target_os = "redox"))] impl User`.

Net: dropping the source-side gates compiles on Linux, BSDs, and Solaris-like, but **fails on macOS** at the `initgroups` import. This is intentional per Decision Log "fully drop the cfg gates".

**Linting and testing entry points** (unchanged from previous work):

- `./scripts/test.sh` — `RUST_LOG=off cargo test --features test`. Filtering: `./scripts/test.sh -- privilege`.
- `./scripts/preflight.sh` — fmt, machete, audit, clippy (`-D warnings`), covgate, full test suite. Final line is `Preflight clean` on success.
- `cargo clippy --package miru-agent --all-features -- -D warnings` — exact command CI runs.

## Plan of Work

### M1 — drop the source-side cfg gates and stubs

Edit `/home/ben/miru/workbench1/repos/agent/agent/src/privilege/mod.rs`:

1. Delete the four `#[cfg(target_os = "linux")]` attributes on:
   - `use std::ffi::CString;` (line 2)
   - `use nix::errno::Errno;` (line 10)
   - `use nix::unistd::{...}` (line 12)
   - `pub fn lookup_user(...)` (line 76)
   - `pub fn ensure_dropped_or_already_unprivileged(...)` (line 133)
   (That is five attributes total — the brief said "four" but the actual count is five.)

2. Delete the two `#[cfg(not(target_os = "linux"))]` stub blocks:
   - Lines 115–123 (the stub `lookup_user`).
   - Lines 203–208 (the stub `ensure_dropped_or_already_unprivileged`).

3. Rewrite the doc comment block for `lookup_user` (currently at lines 69–75). Remove the "On non-Linux targets ..." paragraph. Keep the existing first paragraph ("Look up the uid/gid for a passwd entry by name. ..."). Add a single short paragraph describing the platform contract:

   > Production deploys to Linux. The implementation works on any POSIX system that exposes `getpwnam_r` via the libc backing of `nix::unistd::User::from_name`.

4. Rewrite the doc comment block for `ensure_dropped_or_already_unprivileged` (currently at lines 125–133). Keep the verbatim "Note on environment: ..." paragraph (load-bearing — it documents the env-var preservation behavior the provisioning flow depends on). Remove any wording that implies non-Linux paths exist.

5. Verify the build:

       cargo build -p miru-agent

   Expected: clean build, no new warnings.

6. Sanity-check zero remaining cfg gates in the file:

       grep -nE 'cfg\(target_os' /home/ben/miru/workbench1/repos/agent/agent/src/privilege/mod.rs

   Expected: no output.

### M2 — drop the test-side cfg gates

Edit `/home/ben/miru/workbench1/repos/agent/agent/tests/privilege/mod.rs`:

1. Delete the `#[cfg(target_os = "linux")]` attribute at line 20 (above `fn lookup_user_returns_root_for_root`).
2. Delete the `#[cfg(target_os = "linux")]` attribute at line 41 (above `fn ensure_dropped_or_already_unprivileged_when_running_as_self_is_ok`).
3. Delete the `#[cfg(target_os = "linux")]` attribute at line 73 (above `fn lookup_user_returns_user_not_found_when_name_contains_null_byte`).

No test body changes. The three test bodies are POSIX-portable as written; `root` exists on macOS dev hosts with uid 0, the wrong-user branch behaves identically, and `CString::new` rejects NUL bytes everywhere.

4. Run the privilege tests:

       ./scripts/test.sh -- privilege

   Expected: 7 tests under `privilege::*`, all `ok`. None of the test names are gated out.

### M3 — re-measure coverage and (only if needed) update `.covgate`

After M1+M2 are committed, run preflight and read the per-module coverage line for `agent/src/privilege`:

    ./scripts/preflight.sh

If `Preflight clean` is the last line: nothing more to do — the deletions did not regress coverage past the gate. **Do not** edit `.covgate` "just because" the measured percent is now higher — pre-emptive ratcheting is out of scope per Decision Log "Coverage policy".

If preflight fails on covgate (i.e. measured percent dropped below `30`): inspect the cargo-llvm-cov per-module summary. The expected explanation is that the deletion changed the line/region totals such that the same uncovered branches now make up a slightly different fraction. Lower `.covgate` to `<measured> - 3` (the same cushion convention the swap-to-nix plan used), document the new value under Surprises & Discoveries, and re-run preflight. Do **not** lower `.covgate` below `25` without asking the maintainer — anything below that warrants a separate conversation about whether the FFI deserves a mockable-trait refactor.

### Commit cadence

One commit per code-change milestone. Conventional-Commits-style subjects:

- M1: `refactor(privilege): drop cfg(target_os = linux) gates and non-Linux stubs`
- M2: `chore(privilege): drop test-side cfg gates after source-side gate removal`
- M3: no commit on a clean run. If `.covgate` needs adjustment, commit as `chore(privilege): retune covgate after removing non-Linux stubs`.

## Concrete Steps

All commands run from `/home/ben/miru/workbench1/repos/agent`.

### Step 1 — M1 (source-side)

1. Open `agent/src/privilege/mod.rs` in `$EDITOR`.
2. Delete the five `#[cfg(target_os = "linux")]` lines and the two `#[cfg(not(target_os = "linux"))]` blocks per M1 step 1–2.
3. Rewrite the two doc-comment blocks per M1 step 3–4.
4. Build:

       cargo build -p miru-agent

   Expected: clean.
5. Verify no cfg gates remain:

       grep -nE 'cfg\(target_os' agent/src/privilege/mod.rs

   Expected: no output.
6. Commit:

       git add agent/src/privilege/mod.rs
       git commit -m "refactor(privilege): drop cfg(target_os = linux) gates and non-Linux stubs"

### Step 2 — M2 (test-side)

1. Open `agent/tests/privilege/mod.rs` in `$EDITOR`.
2. Delete the three `#[cfg(target_os = "linux")]` lines per M2 step 1–3.
3. Run the privilege tests:

       ./scripts/test.sh -- privilege

   Expected: 7 tests pass.
4. Commit:

       git add agent/tests/privilege/mod.rs
       git commit -m "chore(privilege): drop test-side cfg gates after source-side gate removal"

### Step 3 — M3 (preflight + covgate)

1. Run preflight:

       ./scripts/preflight.sh

   Expected last line: `Preflight clean`.

2. If covgate fails (it would be reported earlier in the output, before the `Preflight clean` line is otherwise printed): read the failing module's measured percent. Edit `agent/src/privilege/.covgate` to `<measured> - 3` (floor at `25`). Re-run preflight. Commit as `chore(privilege): retune covgate after removing non-Linux stubs`.

3. If preflight is clean on the first run: do nothing. The plan is complete.

## Test Steps

The 7-test suite in `agent/agent/tests/privilege/mod.rs` is the unit/integration test surface. No new tests are added; M2 removes three `#[cfg]` attributes from existing tests.

1. **Module-focused build + test.** From `/home/ben/miru/workbench1/repos/agent`:

       cargo build -p miru-agent
       ./scripts/test.sh -- privilege

   Expected: 7 tests pass on Linux. After M2, no test names are skipped on Linux.

2. **Full integration test target.**

       cargo test -p miru-agent --test mod

   Expected: every test in the integration target passes. The cfg-gate removal should not affect any other test, but a regression elsewhere would still surface here.

3. **Smoke check, wrong-user path** (manual, on a Linux dev machine where the current user is neither root nor `miru`):

       ./target/debug/miru-agent provision --backend-host=https://example.invalid --mqtt-broker-host=example.invalid

   Expected: exits with status 1. Stderr begins with `miru-agent: miru-agent must be run as root or the 'miru' user, but is running as uid <N>.` and contains `Try: sudo ./target/debug/miru-agent ...`. The wording must match what the swap-to-nix branch produces today — `git diff HEAD~3 -- agent/agent/src/privilege/mod.rs` should show only deletions of cfg gates / stubs / non-Linux comments, never wording changes in the `WrongUser` variant.

4. **Smoke check, version path.**

       ./target/debug/miru-agent --version

   Expected: exits 0, prints the version string. The `--version` short-circuit is upstream of the privilege check and continues to work.

5. **Smoke check, systemd path** (manual, on a `.deb`-installed test device — not required for this plan, but recommended before merging PR #62):

       sudo systemctl restart miru.service
       sudo systemctl status miru.service

   Expected: the service starts as `miru:miru` (the `User=miru` directive in `build/debian/miru.service` puts EUID at non-zero before the privilege check; the check sees `info.uid == euid` and returns `Ok(())`). No regression vs. the swap-to-nix branch.

6. **Smoke check, `sudo` root path** (manual, on a `.deb`-installed test device — not required for this plan):

       sudo MIRU_PROVISIONING_TOKEN=tok_test /usr/sbin/miru-agent provision \
           --backend-host=https://api.miru.example --mqtt-broker-host=mqtt.miru.example

   Expected: same provisioning code path as the swap-to-nix branch. Outcome (success or backend error) matches pre-change behavior for the same inputs.

Steps 5–6 require a Linux device with the `.deb` installed and cannot run in CI. They were validated for the swap-to-nix work and do not need re-validation for this refactor unless the implementer's changes drift from M1's "deletions only, no logic edits" constraint.

## Validation and Acceptance

The change is accepted when **all** of the following hold:

1. **Unit/integration tests pass.** `cargo test -p miru-agent --test mod -- privilege::` passes 7 tests on a Linux dev machine. None of the 7 test bodies are modified by this work.

2. **No `cfg(target_os` in the privilege module.** Verify with:

       grep -nE 'cfg\(target_os' /home/ben/miru/workbench1/repos/agent/agent/src/privilege/mod.rs \
                                  /home/ben/miru/workbench1/repos/agent/agent/tests/privilege/mod.rs

   Expected: no output.

3. **Public surface unchanged.** Every item the existing tests reference (`TARGET_USER`, `TARGET_GROUP`, `UserInfo`, `UserInfo` field names `uid` / `gid` / `name`, `PrivilegeErr` and its four variants with their existing fields, `lookup_user`, `ensure_dropped_or_already_unprivileged`) continues to compile and behave identically. No item is added, removed, or renamed.

4. **Preflight clean (mandatory before publishing).** `./scripts/preflight.sh` from `/home/ben/miru/workbench1/repos/agent` prints `Preflight clean` as its final line. Do not push the branch update to PR #62 until preflight is clean on the latest commit. Preflight covers fmt, the custom import linter, machete, audit, clippy (`-D warnings`), covgate, and the test suite.

5. **`.covgate` not regressed.** The value in `agent/agent/src/privilege/.covgate` is `≥ 30` (today's value) — unless preflight fails on covgate, in which case the new value is documented under Surprises & Discoveries with the measured percent and the chosen cushion. The plan does **not** raise the gate as part of this work.

6. **`main.rs` and tests-other-than-cfg-deletions untouched.** `git diff main -- agent/agent/src/main.rs agent/agent/tests/mod.rs` produces no output. `git diff main -- agent/agent/tests/privilege/mod.rs` produces only the three cfg-attribute deletions.

7. **No mention of macOS / non-Linux added or kept.** `grep -nE 'macOS|non-Linux|cross-platform' agent/agent/src/privilege/mod.rs` returns no output (or only output that the implementer has reviewed and explicitly chose to keep — by default, none).

## Idempotence and Recovery

All steps are safe to repeat:

- Deleting cfg attributes and stub functions is idempotent — re-running the edit produces the same end state.
- Re-running `cargo build` / `./scripts/test.sh` / `./scripts/preflight.sh` is idempotent.
- Editing `.covgate` (only if M3 needs it) is idempotent at the file level.

Risky steps:

- **M1 (source rewrite).** A subtle wording drift in the doc comments — e.g. accidentally renaming `TARGET_USER` references or trimming the verbatim "Note on environment" paragraph — could regress the swap-to-nix invariants. Mitigation: M1 step 4 is explicit about preserving that paragraph verbatim; the diff against `main` should be deletions plus a small doc-comment rewrite, nothing else.
- **M3 (covgate).** If the implementer pre-emptively raises the gate to lock in any incidental coverage gain, future PRs unrelated to this work could be blocked by the higher floor. The Decision Log explicitly forbids this.

Rollback: if any post-merge issue surfaces, `git revert <M1-sha> <M2-sha>` restores the cfg-gated implementation. No external state (deps, schema, services) is touched.

## Out of Scope

- Changing the public API of the `privilege` module (signatures, variant names, field names, `Display` strings).
- Modifying `agent/agent/src/main.rs`, `agent/agent/src/lib.rs`, or `agent/agent/tests/mod.rs`.
- Touching install scripts (`scripts/install/*.sh`, `scripts/jinja/templates/partials/utils/activate.sh`) or docs (`README.md`, `ARCHITECTURE.md`).
- Refactoring the FFI behind a mockable trait so that the root-drop / `PostDropMismatch` arms are unit-testable. That is a separate, larger piece of work that would let `.covgate` ratchet meaningfully — out of scope here.
- Removing the `nix` crate, switching crates, or changing the `nix` feature set.
- Restoring or extending macOS / BSD support. The deliberate stance after this plan is "Linux only, with implicit POSIX portability via `nix`; we do not test or build for non-Linux targets".
- Pre-emptively raising `.covgate`. Coverage ratcheting belongs to a coverage-improving change, not a cfg-gate cleanup.
