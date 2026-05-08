# Refactor `privilege` module behind a `System` trait seam for testability

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Refactors `agent/agent/src/privilege/mod.rs`, adds `agent/agent/src/privilege/system.rs`, and updates the inline unit tests inside `mod.rs`. No other crates change. |

This plan lives in `agent/plans/` because all edits are confined to the `miru-agent` Rust crate at `agent/agent/`.

## Purpose / Big Picture

The privilege module owns a security-critical syscall sequence (`initgroups` → `setresgid` → `setresuid` → verify with `getresuid`/`getresgid`) that today calls `nix::unistd` and `std::env::args()` directly. Because each function reaches straight into the OS, the only way to reach branches like `WrongUser`, `PostDropMismatch`, or a non-zero `setres*` errno is to actually run as that user — making most of the logic untestable on a developer or CI machine and forcing reliance on smoke tests.

After this change the module funnels every external interaction through one trait, `System`, with two implementations: `RealSystem` (production, delegates to `nix::unistd` + `std::env::args()`) and a fake the test code can drive. The public entry point `pub fn privilege::run_as(name: &str)` keeps the same signature and behavior — `main.rs` is unchanged. What is gained is a seam: a follow-up `/write-tests` pass can deterministically exercise every branch of `run_as`, `verify_effective_user`, and `drop_to` (e.g. supplementary group failure, post-drop mismatch, partial errno propagation) against a `FakeSystem` in-memory state machine, without root and without flake.

A user runs `./scripts/test.sh` after the change and observes the existing privilege tests (both inline unit tests in `mod.rs` and the three integration tests in `agent/agent/tests/privilege/mod.rs`) pass, plus one new tiny smoke test that demonstrates the `FakeSystem` seam works end-to-end.

## Progress

- [ ] M1 — Add `agent/agent/src/privilege/system.rs` with `pub(crate) trait System` + `pub(crate) struct RealSystem`, build-clean, no callers yet.
- [ ] M2 — Refactor `mod.rs` to call through the trait: introduce `run_as_with<S: System>`, route `lookup_user`, `is_root_user`, `verify_effective_user`, `drop_to` through `&S`, keep `pub fn run_as(name: &str)` as a thin wrapper that constructs `RealSystem`.
- [ ] M3 — Convert the inline unit tests in `mod.rs` to use `FakeSystem`; add one tiny `FakeSystem` smoke test that demonstrates a successful `run_as_with` drop sequence updates fake uid/gid state. Keep `lookup_user_returns_root_for_root` using `RealSystem` explicitly.
- [ ] M4 — Preflight clean: `scripts/preflight.sh` reports `Preflight clean`. Coverage gate at `agent/src/privilege/.covgate` (44.58) still passes.

Use timestamps when you complete steps.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Use `&impl System` (generic monomorphization) rather than `&dyn System`.
  Rationale: There is no stored trait object anywhere in the module — every callee is private and called on a fresh stack frame. Generics keep the existing call shape (no `dyn` indirection, no `Box`) and let the compiler inline `RealSystem` calls so the production binary stays equivalent. `dyn System` would force `Box<dyn>` on the smoke test or a lifetime parameter on every helper for no gain.
  Date/Author: 2026-05-08 / plan author.

- Decision: Trait visibility is `pub(crate)`. `RealSystem` and `System` are not exposed in the `agent` crate's public API.
  Rationale: `AGENTS.md` calls out that `#[cfg(feature = "test")]` is reserved for production code paths that must still compile in test mode (mocks, state setters), but these privilege helpers are crate-internal and only need to be reachable from the `#[cfg(test)] mod tests` block in the same file. `pub(crate)` keeps the seam invisible from `agent/tests/privilege/mod.rs` (which only consumes `pub` items) while letting the inline unit tests in `mod.rs` see it via `use super::*;`.
  Date/Author: 2026-05-08 / plan author.

- Decision: Trait method return types use the same `nix` types (`Uid`, `Gid`, `ResUid`, `ResGid`, `User`, `Errno`) that `mod.rs` uses today.
  Rationale: The `Syscall { call, errno: e as i32, trace }` mapping in `mod.rs` consumes `Errno`. Returning anything else would force a translation layer and risk changing the integer value baked into `PrivilegeErr::Syscall.errno`. The `privilege_err_display_messages_are_human_readable` integration test asserts on those literal values; changing the type ripples into the test.
  Date/Author: 2026-05-08 / plan author.

- Decision: `argv0` is a method on the trait (not a free `std::env::args` call), even though only `verify_effective_user` reads it.
  Rationale: Routing argv0 through `System` lets a future test verify that `WrongUser.argv0` propagates exactly what the runtime supplied (today the integration test `run_as_rejects_non_target_user_when_not_root` only asserts non-empty). `RealSystem::argv0` returns `std::env::args().next().unwrap_or_else(|| "miru-agent".into())` — same fallback as today.
  Date/Author: 2026-05-08 / plan author.

- Decision: The `debug_assert!` in `drop_to` is preserved and routed through `sys.geteuid()` rather than calling `nix::unistd::geteuid()` directly.
  Rationale: A `FakeSystem` whose internal euid is set to 0 must satisfy the assertion; otherwise the fake is unusable for testing the success path of `drop_to`. The semantics stay the same — debug builds still abort if the precondition is violated.
  Date/Author: 2026-05-08 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Crate: `miru-agent` at `agent/agent/` (manifest `agent/agent/Cargo.toml`). The privilege module is a small (147-line) self-contained sub-tree:

- `agent/agent/src/privilege/mod.rs` — public + crate-private functions + inline `#[cfg(test)] mod tests`.
- `agent/agent/src/privilege/errors.rs` — `pub enum PrivilegeErr` (must NOT change).
- `agent/agent/src/privilege/.covgate` — single number `44.58`, the minimum coverage threshold (do NOT modify).

**Current public surface** (verified by reading `mod.rs`):

    pub mod errors;
    pub use self::errors::PrivilegeErr;
    pub fn run_as(name: &str) -> Result<(), PrivilegeErr>;
    pub(crate) type User = nix::unistd::User;
    pub(crate) fn is_root_user(uid: u32) -> bool;
    pub(crate) fn lookup_user(name: &str) -> Result<User, PrivilegeErr>;
    // private:
    fn verify_effective_user(name: &str) -> Result<(), PrivilegeErr>;
    fn drop_to(target: &User) -> Result<(), PrivilegeErr>;

The only external caller of any function in this module (verified via `rg`) is `agent/agent/src/main.rs:36`:

    if let Err(e) = privilege::run_as("miru") { ... }

The internal helpers (`lookup_user`, `is_root_user`, `verify_effective_user`, `drop_to`) are not called from outside the module. That makes refactoring their signatures safe.

**`nix` calls used today** (from `mod.rs`):

- `geteuid() -> Uid` — used in `run_as` (raw u32 compare) and in `drop_to`'s `debug_assert!`.
- `getegid() -> Gid` — used in `verify_effective_user`.
- `getresuid() -> nix::Result<ResUid>` — used in `drop_to` post-verification.
- `getresgid() -> nix::Result<ResGid>` — same.
- `setresuid(real, eff, saved) -> nix::Result<()>` — used in `drop_to`.
- `setresgid(real, eff, saved) -> nix::Result<()>` — used in `drop_to`.
- `initgroups(&CStr, Gid) -> nix::Result<()>` — used in `drop_to`.
- `User::from_name(&str) -> nix::Result<Option<User>>` — used in `lookup_user`.
- `std::env::args().next()` — used in `verify_effective_user` for `argv0`.

`nix::Result<T>` is `Result<T, nix::errno::Errno>`. The trait must preserve this so the existing error-mapping closure `|e| syscall("call", e)` keeps compiling untouched.

**Error type** (`errors.rs`, must NOT change):

    pub enum PrivilegeErr {
        UserNotFound { name: String, trace: Box<Trace> },
        WrongUser { expected, actual_uid, actual_gid, expected_uid, expected_gid, argv0, trace },
        Syscall { call: &'static str, errno: i32, trace },
        PostDropMismatch { expected_uid, expected_gid, actual_ruid, actual_euid, actual_suid, actual_rgid, actual_egid, actual_sgid, trace },
    }

The integration test `privilege_err_display_messages_are_human_readable` (in `agent/agent/tests/privilege/mod.rs`) asserts on substrings of every variant's `Display` output. None of those strings depend on the `System` trait — they live entirely in `errors.rs` — so the test is unaffected by this refactor.

**Inline unit tests in `mod.rs` today** (lines 123–146):

1. `lookup_user_returns_root_for_root` — calls `lookup_user("root")` and asserts uid=0/gid=0/name="root". Hits the real passwd database (always present on Linux). Will be retained as a `RealSystem` smoke test.
2. `lookup_user_returns_user_not_found_for_nonexistent` — calls `lookup_user("nonexistent_user_xyz_123_miru_test")` and asserts `UserNotFound`. Hits the real passwd database and depends on the user being absent. Will be converted to `FakeSystem` so it no longer depends on the host environment.

**Integration tests in `agent/agent/tests/privilege/mod.rs`** (must continue to pass unchanged):

1. `run_as_rejects_non_target_user_when_not_root` — asserts `WrongUser` (or `UserNotFound` on hosts without a `miru` passwd entry).
2. `privilege_err_display_messages_are_human_readable` — asserts `Display` substrings.
3. `run_as_is_noop_when_already_target_user` — asserts `run_as(<current user>)` returns `Ok(())`.

All three call only `privilege::run_as`, `PrivilegeErr`, and `Trace` — none of which change shape.

**Repo conventions** (from `agent/AGENTS.md`):

- Import ordering: three groups separated by a blank line and a comment header, in this order: `// standard crates`, `// internal crates`, `// external crates`.
- Error types derive `thiserror::Error` and implement `crate::errors::Error`.
- The `--features test` flag is required when running the test suite (it gates test-only mocks). Even though the privilege module itself does not use `#[cfg(feature = "test")]` today, the canonical command is `./scripts/test.sh` which already passes the flag.
- Coverage: each module has a `.covgate` minimum coverage percentage; `scripts/covgate.sh` enforces. The privilege gate is `44.58`. We are not adding test coverage in this plan (out of scope), so the gate must continue to pass with whatever coverage the existing tests provide.

**Preflight commands** (from `agent/scripts/`):

- `./scripts/test.sh` runs `cargo test --package miru-agent --features test` with `RUST_LOG=off`.
- `./scripts/lint.sh` runs the import linter, `cargo fmt --check`, `cargo clippy --all-features -- -D warnings`, machete, diet, audit.
- `./scripts/covgate.sh` runs tests with coverage and enforces every `.covgate` threshold.
- `./scripts/preflight.sh` runs lint + covgate + tools lint + tools tests in parallel and prints `Preflight clean` on success or `Preflight FAILED (...)` on any failure. This is the single canonical "is this ready" command.

## Plan of Work

Two new ideas to introduce, then one in-place refactor:

**1. New file `agent/agent/src/privilege/system.rs`** — defines:

    use std::ffi::CStr;

    use nix::errno::Errno;
    use nix::unistd::{Gid, ResGid, ResUid, Uid, User};

    pub(crate) trait System {
        fn geteuid(&self) -> Uid;
        fn getegid(&self) -> Gid;
        fn getresuid(&self) -> Result<ResUid, Errno>;
        fn getresgid(&self) -> Result<ResGid, Errno>;
        fn setresuid(&self, real: Uid, eff: Uid, saved: Uid) -> Result<(), Errno>;
        fn setresgid(&self, real: Gid, eff: Gid, saved: Gid) -> Result<(), Errno>;
        fn initgroups(&self, user: &CStr, primary_gid: Gid) -> Result<(), Errno>;
        fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno>;
        fn argv0(&self) -> String;
    }

    pub(crate) struct RealSystem;

    impl System for RealSystem {
        fn geteuid(&self) -> Uid { nix::unistd::geteuid() }
        fn getegid(&self) -> Gid { nix::unistd::getegid() }
        fn getresuid(&self) -> Result<ResUid, Errno> { nix::unistd::getresuid() }
        fn getresgid(&self) -> Result<ResGid, Errno> { nix::unistd::getresgid() }
        fn setresuid(&self, r: Uid, e: Uid, s: Uid) -> Result<(), Errno> { nix::unistd::setresuid(r, e, s) }
        fn setresgid(&self, r: Gid, e: Gid, s: Gid) -> Result<(), Errno> { nix::unistd::setresgid(r, e, s) }
        fn initgroups(&self, user: &CStr, gid: Gid) -> Result<(), Errno> { nix::unistd::initgroups(user, gid) }
        fn lookup_user(&self, name: &str) -> Result<Option<User>, Errno> { User::from_name(name) }
        fn argv0(&self) -> String { std::env::args().next().unwrap_or_else(|| "miru-agent".into()) }
    }

Add `mod system;` to `mod.rs`. Trait + struct are `pub(crate)` only.

**2. Refactor `mod.rs` to thread `&impl System` through every helper.** Pseudo-diff (literal target shape, not a copy-paste):

    pub mod errors;
    pub(crate) mod system;
    pub use self::errors::PrivilegeErr;
    use self::system::{RealSystem, System};

    pub fn run_as(name: &str) -> Result<(), PrivilegeErr> {
        run_as_with(&RealSystem, name)
    }

    pub(crate) fn run_as_with<S: System>(sys: &S, name: &str) -> Result<(), PrivilegeErr> {
        let euid = sys.geteuid().as_raw();
        if !is_root_user(euid) {
            verify_effective_user(sys, name)
        } else {
            let target = lookup_user(sys, name)?;
            drop_to(sys, &target)
        }
    }

    pub(crate) fn is_root_user(uid: u32) -> bool { uid == 0 }

    pub(crate) fn lookup_user<S: System>(sys: &S, name: &str) -> Result<User, PrivilegeErr> {
        match sys.lookup_user(name) {
            Ok(Some(u)) => Ok(u),
            Ok(None) | Err(Errno::ENOENT | Errno::ESRCH) => Err(/* UserNotFound */),
            Err(e) => Err(/* Syscall { call: "getpwnam_r", ... } */),
        }
    }

    fn verify_effective_user<S: System>(sys: &S, name: &str) -> Result<(), PrivilegeErr> {
        let user = lookup_user(sys, name)?;
        if user.uid != sys.geteuid() || user.gid != sys.getegid() {
            return Err(PrivilegeErr::WrongUser {
                expected: name.to_string(),
                actual_uid: sys.geteuid().as_raw(),
                actual_gid: sys.getegid().as_raw(),
                expected_uid: user.uid.as_raw(),
                expected_gid: user.gid.as_raw(),
                argv0: sys.argv0(),
                trace: trace!(),
            });
        }
        Ok(())
    }

    fn drop_to<S: System>(sys: &S, target: &User) -> Result<(), PrivilegeErr> {
        debug_assert!(sys.geteuid() == nix::unistd::Uid::from_raw(0), "drop_to requires euid=0");
        // ... CString construction unchanged ...
        // syscall closure unchanged ...
        sys.initgroups(&c_name, target.gid).map_err(|e| syscall("initgroups", e))?;
        sys.setresgid(target.gid, target.gid, target.gid).map_err(|e| syscall("setresgid", e))?;
        sys.setresuid(target.uid, target.uid, target.uid).map_err(|e| syscall("setresuid", e))?;
        let ruid = sys.getresuid().map_err(|e| syscall("getresuid", e))?;
        let rgid = sys.getresgid().map_err(|e| syscall("getresgid", e))?;
        // ... PostDropMismatch check unchanged ...
        Ok(())
    }

`is_root_user` keeps its `u32` signature — it's a pure predicate and adding a `&S` parameter buys nothing.

The public function `pub fn run_as(name: &str)` is the only `pub` boundary. Its signature is unchanged, so `agent/agent/src/main.rs:36` does not need to change.

**3. Update inline `#[cfg(test)] mod tests` in `mod.rs`** — define a small `FakeSystem` struct inside the test module (not under `cfg(feature = "test")`, just `#[cfg(test)]`). The fake holds:

- `RefCell<u32>` euid, `RefCell<u32>` egid (so `setres*` can mutate),
- a `Vec<User>` of registered passwd entries (each with name/uid/gid),
- an optional `Errno` to inject into `setresuid`/`setresgid`/`initgroups` (default `None` = success),
- an `argv0: String` field.

Implement `System` for `&FakeSystem` (or `FakeSystem` with `&self`), with `getresuid`/`getresgid` reading the current state and returning `ResUid { real, effective, saved }` set to the same value (we model the post-`setres*` state — fine for our purposes), and `setresuid`/`setresgid` updating the cells.

Then:

- `lookup_user_returns_root_for_root` — keep as-is, but call `lookup_user(&RealSystem, "root")`. Add a comment that this is the one host-coupled test (asserts the real passwd database has a root entry).
- `lookup_user_returns_user_not_found_for_nonexistent` — convert to use a `FakeSystem` with no users registered; assert `lookup_user(&fake, "anything").unwrap_err()` is `UserNotFound`.
- Add **one** new tiny smoke test, e.g. `run_as_with_drops_to_target_when_root`: build a `FakeSystem` with euid=0, register a `User { name: "miru", uid: 1234, gid: 1234, ... }`, call `run_as_with(&fake, "miru")`, assert `Ok(())` and that the fake's euid is now 1234. This proves the seam end-to-end.

Constructing a `nix::unistd::User` for the fake passwd table requires populating all fields. We use the canonical fixture pattern: a small helper inside the test module:

    fn fixture_user(name: &str, uid: u32, gid: u32) -> User {
        User {
            name: name.to_string(),
            passwd: CString::new("x").unwrap(),
            uid: Uid::from_raw(uid),
            gid: Gid::from_raw(gid),
            gecos: CString::new("").unwrap(),
            dir: PathBuf::from("/nonexistent"),
            shell: PathBuf::from("/bin/false"),
        }
    }

(Field list verified by the fact that `nix::unistd::User` is a public struct with those public fields in the version pinned by the workspace; if a field is missing or extra after the build runs, adjust accordingly — see Idempotence and Recovery.)

Out of scope for this plan: any expanded test coverage of `WrongUser`, `PostDropMismatch`, `Syscall` errno paths, or supplementary group failures. A follow-up `/write-tests` invocation will add that coverage on top of the seam this plan ships.

## Concrete Steps

Working directory for every step is `/home/ben/miru/workbench1/repos/agent` unless otherwise stated.

### M1 — Add `system.rs` with the trait + `RealSystem` (no callers yet)

1. Create `agent/agent/src/privilege/system.rs` with the trait and `RealSystem` impl as shown in Plan of Work section 1. Use the three-group import ordering from `AGENTS.md`.
2. Add `pub(crate) mod system;` to `agent/agent/src/privilege/mod.rs` (top of file, after `pub mod errors;`).
3. Build to confirm the new file compiles in isolation:

        cargo build --package miru-agent

   Expected: build succeeds with no new warnings. The trait is unused at this point — Rust's `dead_code` lint does not fire on `pub(crate)` items in a library crate, but if it does add `#[allow(dead_code)]` temporarily on `RealSystem` and remove it in M2.

4. Commit (working directory `/home/ben/miru/workbench1/repos/agent`, the agent repo root; the crate lives at the nested `agent/agent/` path):

        git add agent/agent/src/privilege/system.rs agent/agent/src/privilege/mod.rs
        git commit -m "feat(privilege): introduce System trait seam with RealSystem impl"

### M2 — Route every helper through `&impl System`

1. Edit `agent/agent/src/privilege/mod.rs`:
   - Add `use self::system::{RealSystem, System};` to the internal-crates import group (per `AGENTS.md`, `self::*` belongs with `crate::*` imports under the `// internal crates` header).
   - Change `pub fn run_as(name: &str)` to delegate to a new `pub(crate) fn run_as_with<S: System>(sys: &S, name: &str)` that contains the original logic but calls `sys.geteuid()`.
   - Add `<S: System>` and `sys: &S` parameter to `lookup_user`, `verify_effective_user`, `drop_to`. Replace every `geteuid()`, `getegid()`, `getresuid()`, `getresgid()`, `setresuid(...)`, `setresgid(...)`, `initgroups(...)`, `User::from_name(...)`, and `std::env::args().next()...` call with the corresponding `sys.<method>(...)` call.
   - Remove the now-unused `use nix::unistd::{getegid, geteuid, getresgid, getresuid, initgroups, setresgid, setresuid};` items. Keep `use nix::errno::Errno;` (still referenced by the `Errno::ENOENT | Errno::ESRCH` match arm). Keep `use nix::unistd::Uid` if needed for the `Uid::from_raw(0)` literal in the `debug_assert!`.
   - `is_root_user` is unchanged — still takes `u32`.
2. Build:

        cargo build --package miru-agent

   Expected: clean build. If clippy/format check is wanted earlier, run `cargo clippy --package miru-agent --all-features -- -D warnings`.
3. Run the integration tests to confirm the public behavior is unchanged:

        ./scripts/test.sh

   Expected: all three tests in `agent/agent/tests/privilege/mod.rs` pass; the two pre-existing inline tests in `mod.rs` also still pass (they still call `lookup_user` directly — see M3).

   At this point the inline unit tests still call `lookup_user(name)` with a single argument, which no longer compiles. Either make them temporarily use `lookup_user(&RealSystem, name)` here, or do the M3 conversion in the same edit pass to avoid a broken intermediate state. Recommended: do M2 + M3 changes in two separate commits but in the same working session — first edit saves a broken-test intermediate, second edit fixes it. If preferred, fold M3 into M2 and skip the broken intermediate (acceptable; only the final commit boundary matters).
4. Commit:

        git add agent/agent/src/privilege/mod.rs
        git commit -m "refactor(privilege): route helpers through System trait"

### M3 — Convert inline tests; add one FakeSystem smoke test

1. In the `#[cfg(test)] mod tests` block at the bottom of `agent/agent/src/privilege/mod.rs`:
   - Add the `FakeSystem` struct, `fixture_user` helper, and `impl System for FakeSystem` as described in Plan of Work section 3.
   - `lookup_user_returns_root_for_root`: change `lookup_user("root")` to `lookup_user(&RealSystem, "root")`. Document that this test exercises the production `RealSystem` against the host passwd database.
   - `lookup_user_returns_user_not_found_for_nonexistent`: build a `FakeSystem` with an empty user list and call `lookup_user(&fake, "nonexistent_user_xyz_123_miru_test")`. Assert `UserNotFound { name }` matches the input name. The test is now host-independent.
   - Add `run_as_with_drops_to_target_when_root`: build the fake with `euid` and `egid` cells set to 0, register `fixture_user("miru", 1234, 1234)` in its user list, leave `inject_errno: None` and `argv0: "miru-agent".into()`, then `run_as_with(&fake, "miru").expect("drop succeeds")` and assert the fake's `euid` and `egid` cells now contain 1234. The fake's fields are accessible directly (same module as the test), so the assertions are `assert_eq!(*fake.euid.borrow(), 1234); assert_eq!(*fake.egid.borrow(), 1234);` — no extra accessor method needed. This proves the seam works for the success path.
2. Build and test:

        cargo build --package miru-agent --tests --features test
        ./scripts/test.sh

   Expected: 4 inline unit tests pass (2 converted + 1 untouched in spirit + 1 new) and the 3 integration tests pass.
3. Commit:

        git add agent/agent/src/privilege/mod.rs
        git commit -m "test(privilege): use FakeSystem for inline tests; add seam smoke test"

### M4 — Preflight

1. Refresh lockfile if needed:

        ./scripts/update-deps.sh

   No new dependencies are added by this plan, so this is normally a no-op. Skip if `Cargo.lock` is unchanged.
2. Run preflight:

        ./scripts/preflight.sh

   Expected final line: `Preflight clean`. The script runs lint + covgate + tools lint + tools tests in parallel; if any fails, it prints `Preflight FAILED (lint=X tests=Y tools_lint=Z tools_tests=W)` and exits non-zero. Resolve all failures before declaring done.
3. If `covgate.sh` reports the privilege threshold (`44.58`) is no longer met, investigate before committing. Coverage should not drop because: (a) the FakeSystem smoke test exercises a previously-untested code path (`drop_to` success), and (b) the converted `lookup_user_returns_user_not_found_for_nonexistent` still hits the same lines in `lookup_user`. If coverage paradoxically drops, do **not** edit `.covgate` — instead diagnose what coverage was lost. (See Idempotence and Recovery.)
4. If preflight is clean and there are uncommitted formatting fixups from `cargo fmt`:

        git add -A agent/agent/src/privilege/
        git commit -m "chore(privilege): apply rustfmt after System trait refactor"

   (Only run this if `cargo fmt --check` reported anything; usually the developer ran `cargo fmt` already.)

## Validation and Acceptance

**Acceptance** = the following observable behavior:

1. `cargo build --package miru-agent` succeeds with no new warnings.
2. `./scripts/test.sh` reports all tests pass, including specifically:
   - `tests::lookup_user_returns_root_for_root` (inline, uses `RealSystem`).
   - `tests::lookup_user_returns_user_not_found_for_nonexistent` (inline, uses `FakeSystem`).
   - `tests::run_as_with_drops_to_target_when_root` (inline, new, uses `FakeSystem`).
   - `privilege::run_as_rejects_non_target_user_when_not_root` (integration).
   - `privilege::privilege_err_display_messages_are_human_readable` (integration — unchanged file content, must still pass).
   - `privilege::run_as_is_noop_when_already_target_user` (integration).
3. `./scripts/preflight.sh` final line reads `Preflight clean` (the script's own success marker; see `agent/scripts/preflight.sh`).
4. `agent/agent/src/main.rs` is **unchanged** — the public signature `pub fn privilege::run_as(name: &str) -> Result<(), PrivilegeErr>` is preserved.
5. `agent/agent/src/privilege/errors.rs` is **unchanged** — the integration test that asserts on `Display` substrings continues to pass with no edits.
6. `agent/agent/src/privilege/.covgate` is **unchanged** (still contains `44.58`).
7. `git diff main..HEAD --stat -- agent/agent/src/privilege/` shows the new `system.rs`, modified `mod.rs`, and nothing else inside the privilege subtree.

**Constraints** that must hold at every commit boundary:

- Public API surface unchanged: `pub fn run_as`, `pub use PrivilegeErr`, `pub mod errors`. Any other added items in `mod.rs` are `pub(crate)` or private. The new `system` module is `pub(crate) mod system;` with only `pub(crate)` items inside.
- `PrivilegeErr` variants, fields, and `Display` strings unchanged. Verified by `privilege_err_display_messages_are_human_readable` continuing to pass without edits.
- `debug_assert!(geteuid() == 0)` in `drop_to` remains, but routed through `sys.geteuid()` so a `FakeSystem` with `euid=0` satisfies it.
- `agent/agent/src/privilege/.covgate` is not modified.
- Preflight (`./scripts/preflight.sh`) reports `Preflight clean` before the work is considered complete.

## Idempotence and Recovery

- **All edits are confined to two files** (`mod.rs` and the new `system.rs`). Each milestone ends with a commit, so any breakage can be reverted by `git revert <sha>` or `git reset --soft HEAD~1` (non-destructive — restores changes to the working tree).
- **Broken intermediate state in M2**: if the build is committed before M3, the inline unit tests will not compile because they still call the old `lookup_user(name)` signature. Either fold M3 into the M2 working session before committing, or make the M2 edit pre-emptively patch the two test calls to `lookup_user(&RealSystem, name)` (then M3 only adds the `FakeSystem` and the new smoke test). Either is acceptable; do not push a commit where `cargo test` fails.
- **`User` struct field mismatch** when constructing `fixture_user`: if the workspace's pinned `nix` version exposes a different field set on `nix::unistd::User`, the build will fail with a clear "missing field X" or "no field Y" error. Fix by reading the resolved version's `User` struct (e.g. `cargo doc --open -p nix` or look at `~/.cargo/registry/src/.../nix-*/src/unistd.rs`) and adjusting the fixture. This is purely mechanical.
- **Coverage drop**: if `./scripts/covgate.sh` reports the `44.58` gate failed after M4, do NOT lower the threshold. Run `./scripts/coverage.sh` to get the per-line report, identify which lines are no longer covered, and either (a) drop the fake's coverage of the same lines and re-test, or (b) add a single targeted assertion in the existing smoke test. Lowering `.covgate` is explicitly out of scope.
- **Reverting the entire refactor**: `git revert -n <m1>..<m4>` and `git commit` will return the privilege module to its pre-refactor state. `main.rs` is untouched throughout, so a revert never affects the binary entry point.
- **Re-running a milestone**: every command shown is idempotent (`cargo build`, `cargo test`, `./scripts/preflight.sh`). Source edits are absolute (specific lines and functions), not appended, so re-applying is safe.
