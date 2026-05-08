# Apply privilege module review follow-up fixes

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | All edits land in `agent/agent/src/main.rs`, `agent/agent/src/privilege/{mod.rs,system.rs,errors.rs,fake.rs}` (new file), and `agent/agent/tests/privilege/mod.rs`. |

This plan lives in `agent/plans/` because all edits are confined to the `miru-agent` Rust crate. Work continues on the existing branch `feat/self-privilege-drop` (currently 1 commit ahead of `origin/feat/self-privilege-drop`); the existing PR will pick up the new commits on push — no new PR is opened, no new branch is created.

## Purpose / Big Picture

A second review pass on the `feat/self-privilege-drop` PR surfaced seven concrete issues — two security holes (the tokio runtime boots before privileges drop, and post-drop verification skips supplementary groups), one correctness defect (an integration test panics if the runner happens to be the `miru` user), one test-coverage gap (the six post-drop disjuncts are only verified together), and three design issues (an `i32` errno field that should be `nix::errno::Errno`, an `mod.rs` that is 81% test code, and redundant `geteuid` / `getegid` calls). This plan fixes all seven.

After this plan, a reviewer who runs `./scripts/preflight.sh` from the agent repo root sees `Preflight clean`. They can read the diff and observe:

- `agent/agent/src/main.rs` is a plain `fn main()` that drops privileges before the tokio runtime is built — no async tokio code runs as root.
- `drop_to` calls `sys.getgroups()` after `getresuid` / `getresgid` and rejects any returned gid of 0, surfaced via a new `PrivilegeErr::PrivilegedSupplementaryGroup { gid }` variant.
- The integration test `run_as_rejects_non_target_user_when_not_root` skips (with `eprintln!`) on a host whose euid resolves to the `miru` user instead of panicking.
- The six `PostDropMismatch` disjuncts are each covered by a focused test that flips only that field.
- `PrivilegeErr::Syscall.errno` is `nix::errno::Errno` (no integer cast) — the Display string reads `errno=EPERM` rather than `errno=1`.
- The privilege test scaffolding (`FakeSystem`, `fixture_user`, the `impl System for FakeSystem`) lives in a new `agent/agent/src/privilege/fake.rs` gated `#[cfg(test)]`. The inline tests that remain in `mod.rs` are grouped into nested `mod is_root_user`, `mod lookup_user`, `mod drop_to`, `mod run_as_with` blocks.
- `verify_effective_user` takes the already-captured `(euid, egid)` from `run_as_with` rather than re-calling `sys.geteuid()` / `sys.getegid()` four times.

## Progress

- [ ] M1 — Hoist `euid` / `egid` capture into `run_as_with` (Finding 7)
- [ ] M2 — `Syscall.errno: i32 → nix::errno::Errno` (Finding 5)
- [ ] M3 — Extract `FakeSystem` + `fixture_user` to `fake.rs`; group inline tests into nested sub-modules (Finding 6)
- [ ] M4 — Rewrite `main.rs` to drop privileges before constructing the tokio runtime (Finding 1)
- [ ] M5 — `drop_to` rejects privileged supplementary groups; new `PrivilegedSupplementaryGroup` variant (Finding 2)
- [ ] M6 — Six focused per-disjunct `PostDropMismatch` tests (Finding 4)
- [ ] M7 — Integration test skips when euid resolves to the `miru` user (Finding 3)

Use UTC timestamps when checking off steps. Split partially completed milestones into "done" and "remaining."

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

The agent repo (`mirurobotics/agent`) is a Cargo workspace at `/home/ben/miru/workbench1/repos/agent/`. The binary crate `miru-agent` lives at the nested path `agent/agent/` (manifest `agent/agent/Cargo.toml`). Source paths in this plan use the `agent/agent/...` form throughout.

**Files this plan edits:**

- `agent/agent/src/main.rs` — entry point. Today line 27 declares `#[tokio::main] async fn main()` and line 36 calls `privilege::run_as("miru")` *after* the tokio runtime has been spun up. M4 rewrites this so `run_as` runs in plain synchronous code before the runtime exists.
- `agent/agent/src/privilege/mod.rs` — public entry point and helpers. Today lines 16–24 (`run_as_with`) call `sys.geteuid().as_raw()` once. Lines 49–63 (`verify_effective_user`) call `sys.geteuid()` and `sys.getegid()` twice each — once in the `if` predicate, once in the `WrongUser` field constructor. M1 hoists those into a single capture in `run_as_with`. The same file holds 507 lines of `#[cfg(test)] mod tests` (lines 124–630) including the entire `FakeSystem`, its `impl System`, and `fixture_user`. M3 splits those out.
- `agent/agent/src/privilege/system.rs` — defines `pub(crate) trait System`, `pub(crate) struct RealSystem`, and the `pub(crate) type` aliases for `Gid`, `ResGid`, `ResUid`, `Uid`, `User`, `Errno` (lines 4–9, intentional per commit `cb74999`, **must not be modified**). M2 changes nothing here. M5 adds a `getgroups(&self) -> Result<Vec<Gid>, Errno>` method to the trait and a `RealSystem` impl that delegates to `nix::unistd::getgroups()`.
- `agent/agent/src/privilege/errors.rs` — defines `pub enum PrivilegeErr` with variants `UserNotFound`, `WrongUser`, `Syscall`, `PostDropMismatch`. Today `Syscall { errno: i32 }`; M2 changes the field type to `nix::errno::Errno`. M5 adds a new variant `PrivilegedSupplementaryGroup { gid: u32, trace: Box<Trace> }`.
- `agent/agent/src/privilege/fake.rs` — **new file** introduced by M3, gated `#[cfg(test)] mod fake;` from `mod.rs`. Holds the extracted `FakeSystem`, its setters, the `impl System for FakeSystem`, and the `fixture_user` helper. Importable from the test sub-modules in `mod.rs` via `use super::fake::*;`.
- `agent/agent/src/privilege/.covgate` — single number `44.58`. **Must not be modified.** Current coverage sits at ~74%; the new tests added by M5 and M6 raise coverage further.
- `agent/agent/tests/privilege/mod.rs` — integration tests. Today three tests: `run_as_rejects_non_target_user_when_not_root` (lines 14–40 — panics if euid resolves to `miru`), `privilege_err_display_messages_are_human_readable` (lines 42–99 — exhaustively asserts the Display of every variant), and `run_as_is_noop_when_already_target_user` (lines 102–120 — already has the skip pattern M7 mirrors). M2 updates the `Syscall` Display assertion. M5 adds a `PrivilegedSupplementaryGroup` Display assertion. M7 adds the skip guard to the first test.

**Definitions (define once, used throughout):**

- **euid / egid**: effective uid / gid — the credentials the kernel uses for permission checks against this process.
- **real / effective / saved uid**: POSIX defines three uids per process. `setresuid` sets all three explicitly; `getresuid` returns a `nix::unistd::ResUid` struct with fields `real`, `effective`, `saved` (each `Uid`). `setresgid` / `getresgid` are the gid analogues. The `nix` crate exposes both behind its `user` feature, which the workspace already enables.
- **supplementary groups**: extra gids the kernel grants to a process beyond its primary gid. Returned by `getgroups(2)`. After `initgroups` runs for a non-root user (uid > 0), these should never include 0; finding 2 is that nothing today checks.
- **`nix::errno::Errno`**: the strongly-typed errno enum; its `Display` impl renders symbolic names (e.g. `"EPERM: Operation not permitted"`). The workspace's `nix = "0.31.2"` already exports it via `nix::errno::Errno`; the `Errno` alias in `system.rs` already re-exports it as `pub(crate) type Errno = nix::errno::Errno;`.
- **`PrivilegeErr::PrivilegedSupplementaryGroup`**: new variant added by M5, returned when `getgroups` after the drop reports any gid of 0.

**Repo conventions** (from `agent/AGENTS.md`):

- **Import ordering**: three groups separated by a blank line and a comment header, in this order: `// standard crates`, `// internal crates`, `// external crates`. `crate::*` and `self::*` go under `// internal crates`.
- **Error types** derive `thiserror::Error` and implement `crate::errors::Error`. The `impl crate::errors::Error for PrivilegeErr {}` line at the bottom of `errors.rs` covers every variant including any added by this plan.
- **Testing**: always use `./scripts/test.sh`, which runs `RUST_LOG=off cargo test --features test`. The `--features test` flag is mandatory.
- **Coverage**: each module has a `.covgate` minimum coverage percentage; `scripts/covgate.sh` enforces. The privilege gate is `44.58`. This plan adds tests that raise coverage; do not touch the gate.
- **Lint**: `./scripts/lint.sh` runs the import linter, `cargo fmt --check`, machete, audit, and clippy with `-D warnings`. The import linter also flags 4+ field-by-field `assert_eq!` patterns inside a single test; suppress in-place with `// lint:allow(field-by-field-assert)` if a focused test trips it.
- **Preflight**: `./scripts/preflight.sh` runs lint + covgate + tools lint + tools tests in parallel and prints `Preflight clean` on success or `Preflight FAILED (...)` on any failure. This is the single canonical "is this ready" command.

**Branch state at plan creation**: `feat/self-privilege-drop` at HEAD `cb74999` ("refactor(privilege): centralize nix type aliases in system module"), 1 commit ahead of `origin/feat/self-privilege-drop`, working tree clean, base `main`. Open PR exists; new commits push to the same remote branch.

**Out-of-scope guards (must not touch):**

- `agent/agent/src/privilege/system.rs:4-9` — the `pub(crate) type` aliases for `Gid`, `ResGid`, `ResUid`, `Uid`, `User`, `Errno`. Intentional per commit `cb74999`. Reuse them; do not rename or duplicate.
- `debug_assert!(sys.geteuid() == nix::unistd::Uid::from_raw(0), ...)` at the top of `drop_to`. Defensible — kernel enforces the precondition; assertion is documentation. Leave it.
- Performance considerations (e.g. inlining `is_root_user`). Out of scope.
- `agent/agent/src/privilege/.covgate` (the `44.58` number). Do not modify under any circumstance — the new tests raise coverage; if `covgate.sh` reports a drop, diagnose, do not lower.

## Plan of Work

The seven findings ship as seven atomic-commit milestones. Order is chosen to minimize merge friction: small refactors first (M1, M2), then test plumbing extraction (M3), then the largest behavioral change in `main.rs` (M4), then the new error variant + supplementary-group check (M5), then the new tests that depend on M5's variant (M6), and finally the integration-test skip (M7).

### M1 — Hoist `euid` / `egid` capture into `run_as_with` (Finding 7)

**Goal:** call `sys.geteuid()` and `sys.getegid()` exactly once per `run_as_with` invocation. `verify_effective_user` accepts the captured values as parameters and uses them when constructing `WrongUser` instead of re-calling the trait.

In `agent/agent/src/privilege/mod.rs`:

- In `run_as_with<S: System>(sys: &S, name: &str)`: capture `let euid = sys.geteuid();` and `let egid = sys.getegid();` near the top. Replace the existing `sys.geteuid().as_raw()` call with `euid.as_raw()`. When the non-root branch fires, call `verify_effective_user(sys, name, euid, egid)`.
- Change the signature of `verify_effective_user` to `fn verify_effective_user<S: System>(sys: &S, name: &str, euid: Uid, egid: Gid) -> Result<(), PrivilegeErr>`. Inside the function, use the passed-in `euid` / `egid` for the comparison and for the `WrongUser` field construction (`euid.as_raw()`, `egid.as_raw()`). `sys` is still needed because `lookup_user` and `sys.argv0()` are called.
- The trait's `geteuid` / `getegid` methods stay; `drop_to`'s `debug_assert!` still calls `sys.geteuid()` directly (it is a one-liner; passing the value through would obscure the assertion). The unit test `run_as_with_returns_wrong_user_when_uid_mismatches` and `run_as_with_returns_wrong_user_when_gid_mismatches` continue to pass without source change because they exercise `run_as_with` end-to-end.
- The `Uid` / `Gid` types referenced in the new signature are the `pub(crate) type` aliases already in `agent/agent/src/privilege/system.rs`; import them via the existing `use self::system::{RealSystem, System, User, Errno};` line by adding `Gid` and `Uid` to that import list.

### M2 — `Syscall.errno: i32 → nix::errno::Errno` (Finding 5)

**Goal:** the strongly-typed `Errno` propagates intact into the error variant; the integer cast disappears at every site.

In `agent/agent/src/privilege/errors.rs`:

- Add the import `use nix::errno::Errno;` near the existing `use crate::errors::Trace;` (under the `// internal crates` group; `nix` is external but the aliasing in `system.rs` keeps `Errno` an internal name in `mod.rs`. In `errors.rs`, however, there is no alias — `Errno` is fetched directly from `nix::errno::Errno`, so place the `use` under `// external crates` per repo import-ordering conventions).
- Change `errno: i32` → `errno: Errno` in the `Syscall` variant.
- Change the `#[error("syscall '{call}' failed: errno={errno}")]` attribute to keep the same string. `Errno`'s `Display` impl prints the symbolic name (`EPERM`, `EIO`, …), so the rendered message becomes `syscall 'X' failed: errno=EPERM` rather than `errno=1`. This is intended.

In `agent/agent/src/privilege/mod.rs`:

- In `lookup_user`, the construction `PrivilegeErr::Syscall { call: "getpwnam_r", errno: e as i32, trace: trace!() }` becomes `errno: e,`.
- In `drop_to`, the `syscall` closure currently reads `let syscall = |call: &'static str, e: Errno| PrivilegeErr::Syscall { call, errno: e as i32, trace: trace!() };` — change to `errno: e,`.

In `agent/agent/src/privilege/mod.rs` inline tests (the `#[cfg(test)] mod tests { ... }` block):

- Five existing tests assert `assert_eq!(errno, Errno::<X> as i32)`. Replace each with `assert_eq!(errno, Errno::<X>)`. Affected tests: `lookup_user_maps_other_errno_to_syscall`, `drop_to_short_circuits_on_initgroups_failure`, `drop_to_short_circuits_on_setresgid_failure`, `drop_to_propagates_setresuid_errno`, `drop_to_propagates_getresuid_errno`, `drop_to_propagates_getresgid_errno`, `run_as_with_returns_syscall_when_root_and_lookup_user_errors`. Search regex: `errno, Errno::\w+ as i32`.

In `agent/agent/tests/privilege/mod.rs`, in `privilege_err_display_messages_are_human_readable`:

- The `Syscall` test case currently constructs `PrivilegeErr::Syscall { call: "setuid", errno: 1, trace: dummy_trace() }` and asserts `s.contains("errno=1")`. Replace with `errno: nix::errno::Errno::EPERM` and assert `s.contains("EPERM")` (and keep `s.contains("setuid")`). This exercises the new symbolic-rendering behavior.

### M3 — Extract `FakeSystem` + `fixture_user` to `fake.rs`; group inline tests (Finding 6)

**Goal:** `agent/agent/src/privilege/mod.rs` no longer carries 500+ lines of fake-system scaffolding. The remaining inline tests are grouped into nested sub-modules so a reader can scan by responsibility.

Create new file `agent/agent/src/privilege/fake.rs`:

- Add `#![cfg(test)]` at the top of the file (file-level cfg gate; the only other public attribute is `cfg(test)` on the `mod fake;` declaration in `mod.rs`, so `#![cfg(test)]` here is belt-and-suspenders and matches the pattern used by other test-only helper files — if no other test-only helper files exist in the agent crate, omit `#![cfg(test)]` and rely solely on the `mod.rs`-side `#[cfg(test)] mod fake;` gate).
- Move verbatim from `agent/agent/src/privilege/mod.rs:139-293`: the `FakeSystem` struct, its `impl FakeSystem` (with `new`, `with_argv0`, `inject_errno`, `override_getresuid`, `override_getresgid`, `recorded_calls`), the `impl System for FakeSystem`, and the `fixture_user` helper.
- Imports must use the three-group ordering. Required imports inside `fake.rs`:
  - `// standard crates`: `use std::cell::RefCell;`, `use std::collections::HashMap;`, `use std::ffi::{CStr, CString};`, `use std::path::PathBuf;`.
  - `// internal crates`: `use super::system::{Errno, Gid, ResGid, ResUid, System, Uid, User};`.
  - `// external crates`: none beyond what `super::system` re-exports.
- Make `FakeSystem`, its struct fields (`euid`, `egid`, `users`, `argv0`, `errno_on`, `getresuid_override`, `getresgid_override`, `calls`), every method (`new`, `with_argv0`, `inject_errno`, `override_getresuid`, `override_getresgid`, `recorded_calls`), and `fixture_user` `pub(super)` so `super::tests::*` can drive them. The `impl System for FakeSystem` items inherit the trait's `pub(crate)` visibility; no extra `pub` needed there.

In `agent/agent/src/privilege/mod.rs`:

- Add `#[cfg(test)] mod fake;` near the other module declarations (right after `pub(crate) mod system;`).
- Delete the relocated items from the `#[cfg(test)] mod tests { ... }` block (lines 139–293 of the current file).
- Inside the remaining `mod tests` block, replace `use super::*;` with a fuller import set so the sub-modules see what they need. Required:
  - `// standard crates`: drop the now-unused `RefCell`, `HashMap`, `CStr`, `CString`, `PathBuf`.
  - `// internal crates`: `use super::*;` (still needed for `lookup_user`, `is_root_user`, `run_as_with`, `verify_effective_user`, `drop_to`, `PrivilegeErr`); add `use super::fake::*;` to bring the moved helpers back into scope.
  - `// external crates`: `use nix::unistd::{Gid, ResGid, ResUid, Uid};` may simplify to `use super::system::{Errno, Gid, ResGid, ResUid, Uid};` since the aliases live there. Keep whichever path makes the diff minimal.
- Group the remaining tests into four nested sub-modules. Existing tests and target sub-module:
  - `mod is_root_user`: `is_root_user_truth_table`.
  - `mod lookup_user`: `lookup_user_returns_root_for_root` (note: this one calls `RealSystem` against the host passwd database — keep `use super::super::system::RealSystem;` in the sub-module's import block), `lookup_user_returns_user_not_found_for_nonexistent`, `lookup_user_maps_enoent_to_user_not_found`, `lookup_user_maps_esrch_to_user_not_found`, `lookup_user_maps_other_errno_to_syscall`.
  - `mod drop_to`: `drop_to_invokes_syscalls_in_correct_order`, `drop_to_short_circuits_on_initgroups_failure`, `drop_to_short_circuits_on_setresgid_failure`, `drop_to_propagates_setresuid_errno`, `drop_to_propagates_getresuid_errno`, `drop_to_propagates_getresgid_errno`, `drop_to_returns_post_drop_mismatch_with_all_fields_populated` (the existing all-six-disjuncts smoke; the six new per-disjunct tests added by M6 also live here).
  - `mod run_as_with`: `run_as_with_drops_to_target_when_root`, `run_as_with_no_op_when_already_target_user`, `run_as_with_returns_wrong_user_when_uid_mismatches`, `run_as_with_returns_wrong_user_when_gid_mismatches`, `run_as_with_returns_user_not_found_when_root_and_user_missing`, `run_as_with_returns_syscall_when_root_and_lookup_user_errors`.
- Each nested sub-module needs its own `use super::*;` (and any test-specific imports). The outer `mod tests` block's imports cascade into nested modules unless re-exported, so each sub-module must redeclare what it needs via `use super::*;`.

### M4 — Drop privileges before constructing the tokio runtime (Finding 1)

**Goal:** `privilege::run_as("miru")` runs in plain synchronous code, before any tokio worker thread is spawned. After the drop, a multi-threaded runtime is built explicitly and the existing async logic runs inside `runtime.block_on(run_main(cli_args))`.

In `agent/agent/src/main.rs`:

- Remove the `#[tokio::main]` attribute from `main`.
- Change `async fn main()` to `fn main()`.
- Move every line currently inside `main`'s body **except** CLI parsing and the `privilege::run_as` block into a new `async fn run_main(cli_args: cli::Args)` (signature uses the same `cli::Args` type that `cli::Args::parse` already returns).
- The new `fn main()` body, in order:
  1. `let cli_args = cli::Args::parse(&env::args().collect::<Vec<String>>());`
  2. `if cli_args.display_version { println!("{}", version::format()); return; }`
  3. `if let Err(e) = privilege::run_as("miru") { eprintln!("miru-agent: {e}"); std::process::exit(1); }`
  4. Construct the runtime: `let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() { Ok(rt) => rt, Err(e) => { eprintln!("miru-agent: failed to build tokio runtime: {e}"); std::process::exit(1); } };`
  5. `runtime.block_on(run_main(cli_args));`
- `run_main` contains the rest of the original async body, in order:
  1. `if let Some(provision_args) = cli_args.provision_args { ... return; }`
  2. `if let Some(reprovision_args) = cli_args.reprovision_args { ... return; }`
  3. `run_agent().await;`
- The function `run_agent`, helper `await_shutdown_signal`, the two `run_provision` / `run_reprovision` async fns, and the two sync `handle_*_result` fns are unchanged.
- The workspace `tokio` dependency already enables the `rt-multi-thread` feature (verified in `Cargo.toml` workspace deps line `tokio = { version = "1.41.1", features = ["rt-multi-thread", "fs", "signal"] }`) so `tokio::runtime::Builder::new_multi_thread()` and `.enable_all()` compile without a feature change.

### M5 — `drop_to` rejects privileged supplementary groups (Finding 2)

**Goal:** after `setresgid` / `setresuid` and the `getresuid` / `getresgid` verification, query the supplementary-group list and refuse if any returned gid equals 0.

In `agent/agent/src/privilege/system.rs`:

- Add a method `fn getgroups(&self) -> Result<Vec<Gid>, Errno>;` to `pub(crate) trait System`.
- Add the corresponding `RealSystem` implementation: `fn getgroups(&self) -> Result<Vec<Gid>, Errno> { nix::unistd::getgroups() }`. The existing workspace `nix` features already include `user`, which provides `getgroups`.
- Update the `real_system_read_only_methods_are_self_consistent` smoke test in `system.rs` to call `sys.getgroups().expect("getgroups must succeed")` and assert the returned Vec is well-formed (e.g. `let _ = sys.getgroups().expect("getgroups must succeed");` — cannot assert membership semantics in a generic CI environment).

In `agent/agent/src/privilege/errors.rs`:

- Add a new variant after `PostDropMismatch`:

      #[error(
          "post-drop verification failed: supplementary group list still contains \
           privileged gid {gid} after dropping to the target user"
      )]
      PrivilegedSupplementaryGroup {
          gid: u32,
          trace: Box<Trace>,
      },

- The blanket `impl crate::errors::Error for PrivilegeErr {}` at the bottom continues to cover the new variant — no further error-trait wiring needed.

In `agent/agent/src/privilege/mod.rs`, in `drop_to`:

- After the `if ruid.real != ... { return Err(PrivilegeErr::PostDropMismatch { ... }) }` block, add a `let groups = sys.getgroups().map_err(|e| syscall("getgroups", e))?;`.
- Iterate: `for g in groups { if g.as_raw() == 0 { return Err(PrivilegeErr::PrivilegedSupplementaryGroup { gid: 0, trace: trace!() }); } }`. Capture the offending gid by value into the variant: the loop becomes `for g in groups { if g.as_raw() == 0 { return Err(PrivilegeErr::PrivilegedSupplementaryGroup { gid: g.as_raw(), trace: trace!() }); } }` — this is equivalent and lets a future audit log capture the actual matched value.
- Place the new check *after* the `PostDropMismatch` block but *before* the trailing `Ok(())`. Order matters: `PostDropMismatch` reflects a kernel-level mismatch; `PrivilegedSupplementaryGroup` reflects an `initgroups`-level miss. Both must abort the drop.

In `agent/agent/src/privilege/fake.rs` (created in M3):

- Add a `supplementary_groups: RefCell<Vec<Gid>>` field to the `FakeSystem` struct.
- Initialize it to `RefCell::new(Vec::new())` in `FakeSystem::new`.
- Add a setter `pub(super) fn set_supplementary_groups(&self, groups: Vec<Gid>) { *self.supplementary_groups.borrow_mut() = groups; }`.
- Add the `getgroups` method to `impl System for FakeSystem`:

      fn getgroups(&self) -> Result<Vec<Gid>, Errno> {
          self.calls.borrow_mut().push("getgroups");
          if let Some(&e) = self.errno_on.borrow().get("getgroups") {
              return Err(e);
          }
          Ok(self.supplementary_groups.borrow().clone())
      }

In `agent/agent/src/privilege/mod.rs` `mod tests::drop_to`:

- Add `drop_to_returns_privileged_supplementary_group_when_gid_zero_present`: build a `FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)])`, call `fake.set_supplementary_groups(vec![Gid::from_raw(0), Gid::from_raw(100)])`, run `run_as_with(&fake, "miru")`, expect `Err(PrivilegeErr::PrivilegedSupplementaryGroup { gid: 0, .. })`.
- Add `drop_to_accepts_unprivileged_supplementary_groups`: same fake, `set_supplementary_groups(vec![Gid::from_raw(100), Gid::from_raw(200)])`, expect `Ok(())`. This pins the happy path so a future change does not accidentally reject all non-empty group lists.
- Add `drop_to_propagates_getgroups_errno`: same fake, `inject_errno("getgroups", Errno::EIO)`, expect `Err(PrivilegeErr::Syscall { call: "getgroups", errno: Errno::EIO, .. })`.

In `agent/agent/tests/privilege/mod.rs`, in `privilege_err_display_messages_are_human_readable`:

- Append a new `let priv_supp = PrivilegeErr::PrivilegedSupplementaryGroup { gid: 0, trace: dummy_trace() };` block. Format and assert `s.contains("privileged gid 0")` and `s.contains("supplementary")`.

### M6 — Six focused per-disjunct `PostDropMismatch` tests (Finding 4)

**Goal:** each of the six disjuncts in the post-drop mismatch check has its own focused test, so a future regression that, say, swaps `ruid.real` and `ruid.saved` in the comparison can be pinpointed by which test fails.

In `agent/agent/src/privilege/mod.rs` `mod tests::drop_to`:

Six new tests, each named after the field that differs. Each test seeds the fake with target uid/gid `(1234, 5678)`, then calls one of `override_getresuid` / `override_getresgid` to flip exactly one field while keeping the other five at the target value, and asserts `PostDropMismatch` is returned.

- `drop_to_returns_post_drop_mismatch_when_ruid_real_differs`: `fake.override_getresuid(9999, 1234, 1234)`. Assert `actual_ruid == 9999`, the other five match the target.
- `drop_to_returns_post_drop_mismatch_when_ruid_effective_differs`: `fake.override_getresuid(1234, 9999, 1234)`. Assert `actual_euid == 9999`, others match.
- `drop_to_returns_post_drop_mismatch_when_ruid_saved_differs`: `fake.override_getresuid(1234, 1234, 9999)`. Assert `actual_suid == 9999`, others match.
- `drop_to_returns_post_drop_mismatch_when_rgid_real_differs`: `fake.override_getresgid(9999, 5678, 5678)`. Assert `actual_rgid == 9999`, others match.
- `drop_to_returns_post_drop_mismatch_when_rgid_effective_differs`: `fake.override_getresgid(5678, 9999, 5678)`. Assert `actual_egid == 9999`, others match.
- `drop_to_returns_post_drop_mismatch_when_rgid_saved_differs`: `fake.override_getresgid(5678, 5678, 9999)`. Assert `actual_sgid == 9999`, others match.

The full pre-existing `drop_to_returns_post_drop_mismatch_with_all_fields_populated` test stays. The six new tests are additive.

The lint `field-by-field-assert` rule may flag tests that read 4+ fields with `assert_eq!` from the same destructure. Each new test asserts exactly two of the six `actual_*` fields (the flipped one and one matching control) plus `expected_uid` and `expected_gid` — that is four asserts on the same destructured `match` arm. If lint flags it, suppress in-place with `// lint:allow(field-by-field-assert)` immediately inside the test body before the `match`.

### M7 — Integration test skips when euid resolves to the `miru` user (Finding 3)

**Goal:** `run_as_rejects_non_target_user_when_not_root` no longer panics on a host whose euid happens to map to a passwd entry named `miru`.

In `agent/agent/tests/privilege/mod.rs`, at the top of `run_as_rejects_non_target_user_when_not_root`:

- Resolve the current effective user before the existing assertion. Mirror the skip pattern from `run_as_is_noop_when_already_target_user` (lines 102–120 of the same file, already present). Insert at the very start of the function:

      let euid = nix::unistd::geteuid();
      match nix::unistd::User::from_uid(euid) {
          Ok(Some(u)) if u.name == "miru" => {
              eprintln!(
                  "skipping: euid {} resolves to 'miru'; this test asserts the \
                   non-miru rejection path",
                  euid.as_raw(),
              );
              return;
          }
          Ok(Some(_)) | Ok(None) => {}
          Err(e) => {
              eprintln!("skipping: User::from_uid({}) failed: {e}", euid.as_raw());
              return;
          }
      }

- The remaining body (the `let err = privilege::run_as("miru").expect_err(...)` and the `match err { ... }` block) stays unchanged.

## Concrete Steps

All commands run from `/home/ben/miru/workbench1/repos/agent` unless otherwise stated.

### Pre-flight

1. Confirm branch:

       git rev-parse --abbrev-ref HEAD

   Expected: `feat/self-privilege-drop`. If not, switch via `git checkout feat/self-privilege-drop`.

2. Confirm working tree is clean:

       git status

   Expected: `nothing to commit, working tree clean`.

3. Baseline preflight:

       ./scripts/preflight.sh

   Expected last line: `Preflight clean`. If it does not pass on the unmodified branch, stop and investigate before editing anything.

### M1 — Hoist `euid` / `egid` capture (Finding 7)

4. Edit `agent/agent/src/privilege/mod.rs`:

   - In the `use self::system::{...};` line, add `Gid` and `Uid` to the imported aliases.
   - In `run_as_with`, capture `let euid = sys.geteuid();` and `let egid = sys.getegid();`. Replace `sys.geteuid().as_raw()` with `euid.as_raw()`. Pass `euid, egid` to `verify_effective_user` in the non-root branch.
   - Change `verify_effective_user`'s signature to take `euid: Uid, egid: Gid`. Inside, drop the now-unused `sys.geteuid()` / `sys.getegid()` calls; use the parameters directly in the comparison and in the `WrongUser` field constructor.
   - The `drop_to` function still calls `sys.geteuid()` directly inside its `debug_assert!` — leave that.

5. Run scoped tests, lint, and preflight:

       ./scripts/test.sh -- privilege
       ./scripts/preflight.sh

   Expected: all privilege tests pass; final line `Preflight clean`. The existing `run_as_with_returns_wrong_user_when_uid_mismatches` and `run_as_with_returns_wrong_user_when_gid_mismatches` tests still pass — they call `run_as_with` end-to-end and only inspect the resulting `WrongUser` fields.

6. Commit M1:

       git add agent/agent/src/privilege/mod.rs
       git commit -m "refactor(privilege): hoist euid/egid capture into run_as_with"

### M2 — Errno type tightening (Finding 5)

7. Edit `agent/agent/src/privilege/errors.rs`:

   - Under the `// external crates` group, add `use nix::errno::Errno;` (`errors.rs` does not currently have an `// external crates` block; add one if needed, respecting three-group ordering).
   - Change the `Syscall` variant's `errno: i32` to `errno: Errno`.
   - The `#[error("syscall '{call}' failed: errno={errno}")]` attribute is unchanged — `Errno` already implements `Display`.

8. Edit `agent/agent/src/privilege/mod.rs`:

   - In `lookup_user`'s `Err(e) => Err(PrivilegeErr::Syscall { ..., errno: e as i32, ... })` arm, change `errno: e as i32,` to `errno: e,`.
   - In `drop_to`'s `let syscall = |call: &'static str, e: Errno| PrivilegeErr::Syscall { call, errno: e as i32, trace: trace!() };` closure, change `errno: e as i32,` to `errno: e,`.

9. Edit the inline `mod tests` block in `agent/agent/src/privilege/mod.rs`:

   - Find every `assert_eq!(errno, Errno::<X> as i32)` (regex `errno, Errno::\w+ as i32`) and drop the ` as i32`. Affected: the seven sites listed in Plan of Work (M2). Each becomes `assert_eq!(errno, Errno::<X>)`.

10. Edit `agent/agent/tests/privilege/mod.rs`:

    - In `privilege_err_display_messages_are_human_readable`, the `Syscall` test case constructs `PrivilegeErr::Syscall { call: "setuid", errno: 1, trace: dummy_trace() };`. Change `errno: 1` to `errno: nix::errno::Errno::EPERM`. Replace the assertion `assert!(s.contains("errno=1"));` with `assert!(s.contains("EPERM"));`. Keep `assert!(s.contains("setuid"));`.

11. Run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: all privilege tests pass; final line `Preflight clean`. The Display assertion shift from numeric to symbolic is the most visible change in this milestone.

12. Commit M2:

        git add agent/agent/src/privilege/errors.rs agent/agent/src/privilege/mod.rs agent/agent/tests/privilege/mod.rs
        git commit -m "refactor(privilege): use nix::errno::Errno directly in Syscall variant"

### M3 — Extract test fakes to `fake.rs`; group inline tests (Finding 6)

13. Create `agent/agent/src/privilege/fake.rs`. Use the three-group import ordering. Inside, paste verbatim from `mod.rs`:

    - The `FakeSystem` struct (with `RefCell` fields).
    - The `impl FakeSystem` block (`new`, `with_argv0`, `inject_errno`, `override_getresuid`, `override_getresgid`, `recorded_calls`).
    - The `impl System for FakeSystem` block (every method).
    - The `fixture_user` helper.

    Make every relocated item `pub(super)` so the test sub-modules in `mod.rs` can use them.

    Required imports:

        // standard crates
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::ffi::{CStr, CString};
        use std::path::PathBuf;

        // internal crates
        use super::system::{Errno, Gid, ResGid, ResUid, System, Uid, User};

14. Edit `agent/agent/src/privilege/mod.rs`:

    - Add `#[cfg(test)] mod fake;` directly after the `pub(crate) mod system;` line.
    - Delete the `FakeSystem` struct, its `impl FakeSystem`, the `impl System for FakeSystem`, and the `fixture_user` helper from inside the `#[cfg(test)] mod tests { ... }` block. Keep the `use` statements in `mod tests` and add `use super::fake::*;` so the sub-modules pick up the relocated items.
    - Group the remaining tests into nested sub-modules: `mod is_root_user`, `mod lookup_user`, `mod drop_to`, `mod run_as_with`. Each sub-module needs its own `use super::*;` (and any module-specific imports such as `use super::super::system::RealSystem;` in `mod lookup_user` for the `lookup_user_returns_root_for_root` test).

15. Run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: every previously-passing test still passes (no test bodies are modified by M3 — only their location within nested modules changes). `cargo machete` is unchanged. Final line `Preflight clean`.

16. Commit M3:

        git add agent/agent/src/privilege/mod.rs agent/agent/src/privilege/fake.rs
        git commit -m "refactor(privilege): split FakeSystem into fake.rs and group inline tests"

### M4 — `main.rs` privilege-drop-before-runtime (Finding 1)

17. Edit `agent/agent/src/main.rs`:

    - Remove the `#[tokio::main]` attribute above `main`.
    - Convert `async fn main()` to `fn main()`.
    - Inside `main`, after `privilege::run_as("miru")`, build a multi-threaded runtime explicitly:

          let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
              Ok(rt) => rt,
              Err(e) => {
                  eprintln!("miru-agent: failed to build tokio runtime: {e}");
                  std::process::exit(1);
              }
          };
          runtime.block_on(run_main(cli_args));

    - Define `async fn run_main(cli_args: cli::Args) { ... }` next to `main`. Move into it the body that today follows the `privilege::run_as` block (the two `if let Some(...)` arms for provision / reprovision and the trailing `run_agent().await;`).

18. Verify there are no other `#[tokio::main]` annotations that need updating:

        grep -rn "#\[tokio::main\]" agent/agent/src/

    Expected: no matches (or only matches in deliberately-isolated test binaries, which this plan does not touch).

19. Build and run scoped tests, lint, and preflight:

        cargo build --package miru-agent
        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: clean build; privilege tests pass; final line `Preflight clean`.

20. Smoke check the binary still starts and exits gracefully without args:

        cargo run --package miru-agent -- --version

    Expected: prints the version string and exits 0. (`run_as` runs but returns `Ok(())` because no `miru` user exists on the dev host — wait, it does call `verify_effective_user` and may fail. Reality check: the workspace baseline preflight in step 3 was already passing, so the existing `cargo test` machinery exercises the same path. If `cargo run --version` fails, look at whether the new `--version` short-circuit (step 17 first sub-bullet) is still reached *before* `privilege::run_as`. The order in `fn main` must be: parse → version-print-and-return → run_as → runtime.)

21. Commit M4:

        git add agent/agent/src/main.rs
        git commit -m "fix(main): drop privileges before constructing the tokio runtime"

### M5 — Supplementary-group rejection (Finding 2)

22. Edit `agent/agent/src/privilege/system.rs`:

    - Add `fn getgroups(&self) -> Result<Vec<Gid>, Errno>;` to `pub(crate) trait System`.
    - Add the `RealSystem` impl: `fn getgroups(&self) -> Result<Vec<Gid>, Errno> { nix::unistd::getgroups() }`.
    - Update `real_system_read_only_methods_are_self_consistent` to call `let _ = sys.getgroups().expect("getgroups must succeed");` so the new method is exercised.

23. Edit `agent/agent/src/privilege/errors.rs`:

    - Add the new variant `PrivilegedSupplementaryGroup { gid: u32, trace: Box<Trace> }` after `PostDropMismatch`, with the `#[error(...)]` string from Plan of Work (M5).

24. Edit `agent/agent/src/privilege/mod.rs`:

    - In `drop_to`, after the `PostDropMismatch` block, call `let groups = sys.getgroups().map_err(|e| syscall("getgroups", e))?;`.
    - Iterate: `for g in groups { if g.as_raw() == 0 { return Err(PrivilegeErr::PrivilegedSupplementaryGroup { gid: g.as_raw(), trace: trace!() }); } }`.
    - Place the loop *before* the trailing `Ok(())`.

25. Edit `agent/agent/src/privilege/fake.rs`:

    - Add field `supplementary_groups: RefCell<Vec<Gid>>` to `FakeSystem`. Initialize to `RefCell::new(Vec::new())` in `FakeSystem::new`.
    - Add `pub(super) fn set_supplementary_groups(&self, groups: Vec<Gid>) { *self.supplementary_groups.borrow_mut() = groups; }`.
    - Add the `getgroups` method to `impl System for FakeSystem` per Plan of Work (M5).

26. Edit `agent/agent/src/privilege/mod.rs` `mod tests::drop_to`: add three new tests (`drop_to_returns_privileged_supplementary_group_when_gid_zero_present`, `drop_to_accepts_unprivileged_supplementary_groups`, `drop_to_propagates_getgroups_errno`) per Plan of Work (M5).

27. Edit `agent/agent/tests/privilege/mod.rs` `privilege_err_display_messages_are_human_readable`: append a `PrivilegedSupplementaryGroup { gid: 0, trace: dummy_trace() }` case and assert `s.contains("privileged gid 0")` and `s.contains("supplementary")`.

28. Run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: all tests pass (including the three new `drop_to_*` cases and the extended Display test); final line `Preflight clean`. Coverage rises because new branches are exercised.

29. Commit M5:

        git add agent/agent/src/privilege/system.rs agent/agent/src/privilege/errors.rs \
                agent/agent/src/privilege/mod.rs agent/agent/src/privilege/fake.rs \
                agent/agent/tests/privilege/mod.rs
        git commit -m "fix(privilege): reject privileged supplementary groups after drop"

### M6 — Per-disjunct `PostDropMismatch` tests (Finding 4)

30. Edit `agent/agent/src/privilege/mod.rs` `mod tests::drop_to`. Add the six tests named in Plan of Work (M6). Each test:

    - Constructs `let fake = FakeSystem::new(0, 0, vec![fixture_user("miru", 1234, 5678)]);`.
    - Calls exactly one of `override_getresuid` / `override_getresgid` to flip exactly one of the six fields; the other five fields keep target values (`1234` / `5678`).
    - Calls `let err = run_as_with(&fake, "miru").expect_err("post-drop mismatch must propagate");`.
    - Matches `PrivilegeErr::PostDropMismatch { ... }` and asserts the flipped field == `9999` plus that `expected_uid == 1234` and `expected_gid == 5678`. If the import linter flags `field-by-field-assert`, prepend `// lint:allow(field-by-field-assert)` inside the test body.

31. Run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: every existing test still passes; six new tests pass; final line `Preflight clean`.

32. Commit M6:

        git add agent/agent/src/privilege/mod.rs
        git commit -m "test(privilege): add per-disjunct PostDropMismatch coverage"

### M7 — Integration-test skip on euid==miru (Finding 3)

33. Edit `agent/agent/tests/privilege/mod.rs`. In `run_as_rejects_non_target_user_when_not_root`, prepend the `match nix::unistd::User::from_uid(euid)` skip block from Plan of Work (M7) at the top of the function, before the existing `let err = privilege::run_as("miru").expect_err(...)` line.

34. Run scoped tests, lint, and preflight:

        ./scripts/test.sh -- privilege
        ./scripts/preflight.sh

    Expected: all tests pass; final line `Preflight clean`. On the dev/CI host (which is not the `miru` user), the test still exercises the `WrongUser` / `UserNotFound` path. On a hypothetical host where the runner *is* `miru`, it now `eprintln!`s and returns instead of panicking.

35. Commit M7:

        git add agent/agent/tests/privilege/mod.rs
        git commit -m "test(privilege): skip non-target rejection test when euid is miru"

### Push

36. Push all seven new commits to the existing branch:

        git push origin feat/self-privilege-drop

    Expected: `git status` reports `Your branch is up to date with 'origin/feat/self-privilege-drop'`. The existing PR picks up the commits automatically — no new PR is opened.

## Validation and Acceptance

The plan is complete when **all** of the following hold from `/home/ben/miru/workbench1/repos/agent`:

- `./scripts/preflight.sh` final line reads `Preflight clean`.
- `./scripts/test.sh -- privilege` (or `RUST_LOG=off cargo test --features test -p miru-agent privilege`) passes — every pre-existing test plus the new tests added by M5 and M6:
  - M5 adds three: `drop_to_returns_privileged_supplementary_group_when_gid_zero_present`, `drop_to_accepts_unprivileged_supplementary_groups`, `drop_to_propagates_getgroups_errno` (in `mod tests::drop_to`).
  - M6 adds six: `drop_to_returns_post_drop_mismatch_when_{ruid_real,ruid_effective,ruid_saved,rgid_real,rgid_effective,rgid_saved}_differs` (in `mod tests::drop_to`).
- `cargo build -p miru-agent` succeeds with no warnings.
- `cargo fmt --check` exits 0 from the workspace root (no diffs).
- `cargo clippy --all-features -- -D warnings` exits 0.
- `git log --oneline origin/feat/self-privilege-drop..HEAD` shows exactly seven new commits, one per milestone, with messages matching the templates in *Concrete Steps*. The order maps to findings: 7 → 5 → 6 → 1 → 2 → 4 → 3.

**Constraints** that must hold at every commit boundary:

- Public API surface unchanged: `pub fn privilege::run_as(name: &str) -> Result<(), PrivilegeErr>` (signature byte-for-byte preserved); `pub use self::errors::PrivilegeErr;`. Only added items are the new `PrivilegedSupplementaryGroup` variant on `PrivilegeErr` (additive). The existing `i32` errno field on `Syscall` becomes `nix::errno::Errno` — a breaking change in error-variant shape, but `PrivilegeErr` is constructed only inside the privilege module so no external caller is affected. The integration-test Display assertion is updated to match.
- `agent/agent/src/privilege/system.rs:4-9` `pub(crate) type` aliases for `Gid`, `ResGid`, `ResUid`, `Uid`, `User`, `Errno` are unchanged.
- `debug_assert!(sys.geteuid() == nix::unistd::Uid::from_raw(0), "drop_to requires euid=0");` at the top of `drop_to` is unchanged.
- `agent/agent/src/privilege/.covgate` is unchanged (still `44.58`). The new tests raise coverage; the gate is not adjusted.
- `./scripts/preflight.sh` reports `Preflight clean` at the tip of every milestone commit (not only at the end of M7) — this enables `git bisect` and matches the per-milestone-commit convention.

**Behavioral acceptance for reviewers:**

- Inspect `agent/agent/src/main.rs` diff: `#[tokio::main]` removed; `fn main()` is plain sync; `privilege::run_as("miru")` runs before the `tokio::runtime::Builder` line; the async body has been factored into `run_main`.
- Inspect `agent/agent/src/privilege/mod.rs` `drop_to` diff: the call sequence after `setresuid` is `getresuid → getresgid → PostDropMismatch check → getgroups → PrivilegedSupplementaryGroup loop → Ok(())`.
- Inspect `agent/agent/src/privilege/errors.rs` diff: `Syscall.errno: nix::errno::Errno`; new variant `PrivilegedSupplementaryGroup { gid: u32, trace: Box<Trace> }`.
- Inspect `agent/agent/src/privilege/fake.rs` diff: new file containing the `FakeSystem` scaffolding.
- Inspect `agent/agent/tests/privilege/mod.rs` diff: the first test now begins with the `User::from_uid` skip pattern; the Display test asserts `EPERM` and `privileged gid 0`.

**Acceptance gate**: the orchestrator must observe `Preflight clean` at the tip of M7 before pushing the branch. If preflight fails after any milestone, fix the cause inside that milestone's commit (`git reset --soft HEAD~1` and re-edit) — do not stack a fixup commit on top.

## Idempotence and Recovery

- All edits are pure-source changes; reapplying any milestone is safe. Each milestone ends with a single commit, so `git revert <sha>` or `git reset --soft HEAD~1` rolls back atomically.
- **M2 ripple risk**: changing `Syscall.errno: i32 → Errno` is a breaking change to a single private struct shape. Every construction site is in `mod.rs` (two sites — `lookup_user` and the `syscall` closure in `drop_to`) and every consumer is in `mod tests` plus the one integration test. If `cargo build` after step 9 fails on a missing site, run `grep -rn "errno:" agent/agent/src/privilege/ agent/agent/tests/privilege/` to find every constructor and fix in place.
- **M3 ripple risk**: moving items between modules with `pub(super)` visibility can fail with "private item used outside its module" if the test sub-modules in `mod.rs` reference an item without `pub(super)`. Fix by adding `pub(super)` to the missed item and rerun `cargo test --features test -p miru-agent`.
- **M4 ripple risk**: the runtime construction must use `tokio::runtime::Builder::new_multi_thread()` (not `new_current_thread`) to preserve the previous behavior of `#[tokio::main]`, which defaults to multi-thread. The workspace `tokio` features include `rt-multi-thread`; `cargo build` after step 19 confirms.
- **M5 ripple risk**: forgetting to add `getgroups` to `FakeSystem`'s `impl System` block produces a compile error "not all trait items implemented." Add the method in `fake.rs`.
- **Coverage drop**: if `./scripts/covgate.sh` reports the `44.58` privilege threshold failed after any milestone, do **not** lower the gate. Run `./scripts/coverage.sh` (the per-line report) and inspect which lines lost coverage. M2 and M3 do not introduce any new lines; M5 and M6 add new branches with matching tests, so coverage should rise. If it falls, diagnose and add an assertion.
- **Reverting the entire follow-up**: `git revert -n <m1>..<m7>` and `git commit` returns the privilege module and `main.rs` to their pre-follow-up state. The earlier System-trait refactor is preserved.
- **Re-running a milestone**: every command shown is idempotent (`cargo build`, `cargo test`, `./scripts/preflight.sh`). Source edits are absolute (specific functions and lines), not appended, so re-applying after `git reset --soft HEAD~1` is safe.
