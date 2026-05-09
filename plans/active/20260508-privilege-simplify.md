# Collapse the privilege module to a pure user verification

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | All edits land in `agent/agent/src/main.rs`, `agent/agent/src/privilege/{mod.rs,errors.rs}`, deletion of `agent/agent/src/privilege/{system.rs,fake.rs}`, and `agent/agent/tests/privilege/mod.rs`. |

This plan lives in `agent/plans/` because every edit is confined to the `miru-agent` Rust crate. Work continues on the existing branch `feat/self-privilege-drop`; the existing PR (if open) picks up the new commits on push — no new branch is created.

## Purpose / Big Picture

The agent's privilege module currently performs a self-privilege-drop: when started as root it calls `setresuid` / `setresgid` / `initgroups` to switch to the `miru` user, with extensive verification (`getresuid` / `getresgid` / `getgroups`) and a `System` trait seam that lets `FakeSystem` drive every kernel branch in unit tests. Combined source + tests sit at roughly 800 lines.

We are replacing that machinery with a pure user-verification check. After this plan, `miru-agent`'s startup behavior is:

- Resolve the passwd entry for the user named `miru`.
- Compare the process euid/egid against that entry.
- If they match, proceed. If not, exit with a clear error pointing the operator at `sudo -u miru ...`.

The user-visible improvement: a non-`miru` invocation (including a bare `sudo miru-agent`) now prints `Try: sudo -u miru /usr/sbin/miru-agent ...` instead of the old, ambiguous `Try: sudo miru-agent ...`. The implementation is reduced to roughly 80 source + 80 test lines, the `System` trait and `FakeSystem` are gone, and `agent/src/main.rs` returns to the idiomatic `#[tokio::main] async fn main()` form.

A reviewer who runs `./scripts/preflight.sh` from the agent repo root sees `Preflight clean`. Inspecting the diff they observe: `system.rs` and `fake.rs` deleted; `mod.rs` rewritten with a top-level `pub fn run_as` that calls `lookup_user` then `verify`; two `PrivilegeErr` variants (`PostDropMismatch`, `PrivilegedSupplementaryGroup`) deleted; `Syscall.errno` retained but only constructed inside `lookup_user`; `main.rs` reverted to `#[tokio::main]`.

## Progress

- [x] M1 — Revert `agent/src/main.rs` to `#[tokio::main] async fn main()` — 2026-05-08 (`e7d1a23`)
- [x] M2 — Collapse `privilege/mod.rs` to pure user verification (delete `system.rs`, `fake.rs`, `drop_to`, `is_root_user`, `run_as_with`, `System` trait, `RealSystem`, `FakeSystem`, all FakeSystem-driven inline tests; rewrite `mod.rs`; update `WrongUser` Display message) — 2026-05-08 (`4edb171`)
- [x] M3 — Drop `PostDropMismatch` and `PrivilegedSupplementaryGroup` from `PrivilegeErr` — 2026-05-08 (`12c36c7`)
- [x] M4 — Replace fake-driven inline tests with pure `verify()` unit tests; update integration Display assertions — 2026-05-08

Use UTC timestamps when checking off steps. Split partially completed milestones into "done" and "remaining."

## Surprises & Discoveries

- **2026-05-08 — `tokio` `macros` feature absent.** `#[tokio::main]` requires the `macros` feature, which the workspace `tokio` features list (`["rt-multi-thread", "fs", "signal"]`) did not include. Added `"macros"` to the workspace tokio features in `Cargo.toml` as part of M1 (anticipated under "M1 ripple risk" in Idempotence and Recovery).
- **2026-05-08 — covgate dips at M2/M3 tip, recovers at M4.** As anticipated, `./scripts/preflight.sh` reports `privilege: 83.82% (requires 93.88%)` at M2's tip — the FakeSystem-driven inline tests are gone but the new `verify()` unit tests don't land until M4. Coverage returns above the gate at M4. No `.covgate` change made (per user directive). Build, lint, machete, audit, clippy, and all tests pass; only the coverage gate fails between M2 and M4. M4's commit boundary is the canonical "preflight clean" gate.
- **2026-05-08 — M4 added two extra `lookup_user` tests beyond the plan.** The plan specified four inline tests (1 seed + 3 `verify`), but two more were needed to clear covgate at M4: `lookup_user_returns_user_not_found_for_missing_name` (exercises `Ok(None)` / ENOENT → `UserNotFound`) and `run_as_returns_user_not_found_when_name_is_missing` (drives `run_as`'s `?` early-return through the lookup-failure path). Both use a synthetic name that cannot collide with any real account. The `Err(other) → Syscall` branch in `lookup_user` remains uncovered — covgate passes without it.

## Decision Log

(Add entries as you go.)

## Outcomes & Retrospective

**Completed 2026-05-08.** All four milestones landed on `feat/self-privilege-drop`:

- `e7d1a23` — M1: `#[tokio::main]` entry point restored; explicit `Builder` runtime removed; `tokio` `macros` feature added to workspace.
- `4edb171` — M2: `privilege/mod.rs` collapsed to `lookup_user` + `verify`; `system.rs` and `fake.rs` deleted; `WrongUser` Display now suggests `sudo -u {expected} {argv0}`.
- `12c36c7` — M3: `PostDropMismatch` and `PrivilegedSupplementaryGroup` variants removed from `PrivilegeErr`; integration Display test trimmed.
- M4: six inline tests on `mod.rs` (`lookup_user_returns_root_for_root`, `lookup_user_returns_user_not_found_for_missing_name`, `run_as_returns_user_not_found_when_name_is_missing`, three `verify_*`); integration tests untouched.

Validation: `./scripts/preflight.sh` clean at M4's tip (privilege: 94.08% ≥ 93.88%); `./scripts/test.sh` reports 1332 passed; `cargo run -p miru-agent -- --version` exits 0. Public API surface (`pub fn run_as`, `pub use PrivilegeErr`) preserved at signature level; the enum narrowed from five to three variants — only consumer is `main.rs` via `Display`.

## Context and Orientation

The agent repo (`mirurobotics/agent`) is a Cargo workspace at `/home/ben/miru/workbench1/repos/agent/`. The binary crate `miru-agent` lives at the nested path `agent/agent/` (manifest `agent/agent/Cargo.toml`). Source paths in this plan use the `agent/agent/...` form throughout.

**Files this plan touches:**

- `agent/agent/src/main.rs` — entry point. Today (HEAD `e1cc7ab`) line 28 declares plain `fn main()` and lines 41–51 build a `tokio::runtime::Builder::new_multi_thread()` runtime explicitly because privilege drop must run before the runtime spawns worker threads. With the drop gone, the explicit runtime construction is unnecessary; the file reverts to `#[tokio::main] async fn main()`.
- `agent/agent/src/privilege/mod.rs` — public entry point and helpers. Today 678 lines: 71 lines of source (`run_as`, `run_as_with`, `is_root_user`, `lookup_user`, `verify_effective_user`, `drop_to`) and 511 lines of `#[cfg(test)] mod tests` covering `is_root_user`, `lookup_user`, `drop_to`, and `run_as_with`. After this plan: roughly 80 source lines + 80 test lines.
- `agent/agent/src/privilege/system.rs` — defines the `pub(crate) trait System`, `pub(crate) struct RealSystem`, the `pub(crate) type` aliases (`Gid`, `ResGid`, `ResUid`, `Uid`, `User`, `Errno`), the `RealSystem` impl (which delegates to `nix::unistd`), and a `real_system_read_only_methods_are_self_consistent` smoke test. M2 deletes this file entirely. The aliases that other code in `mod.rs` uses (`Uid`, `Gid`, `User`, `Errno`) are pulled directly from `nix` in the rewrite.
- `agent/agent/src/privilege/fake.rs` — defines `FakeSystem` (in-memory state machine for `setres*`, `getres*`, `initgroups`, `getgroups`, `lookup_user`, `argv0`), its setters (`with_argv0`, `inject_errno`, `override_getresuid`, `override_getresgid`, `set_supplementary_groups`, `recorded_calls`), the `impl System for FakeSystem`, and the `fixture_user` helper. 180 lines, gated `#[cfg(test)] mod fake;` from `mod.rs`. M2 deletes this file entirely.
- `agent/agent/src/privilege/errors.rs` — defines `pub enum PrivilegeErr` with five variants: `UserNotFound`, `WrongUser`, `Syscall`, `PostDropMismatch`, `PrivilegedSupplementaryGroup`. M2 updates the `WrongUser` Display string. M3 removes `PostDropMismatch` and `PrivilegedSupplementaryGroup`.
- `agent/agent/src/privilege/.covgate` — single number `93.88`. **Do not modify.** The user brief asks for the gate to be left alone. Removing tests will drop measured coverage; if `covgate.sh` fails after the rewrite, surface in Surprises & Discoveries and stop — the gate-vs-coverage decision is out of scope (see Risks below).
- `agent/agent/tests/privilege/mod.rs` — three integration tests: `run_as_rejects_non_target_user_when_not_root` (lines 14–40), `privilege_err_display_messages_are_human_readable` (lines 42–107, exercises the Display of every variant), and `run_as_is_noop_when_already_target_user` (lines 109–128). The first and third are kept verbatim. The second is shortened to assert only the three surviving variants and the new `WrongUser` Display string (`sudo -u miru /usr/sbin/miru-agent`).

**Definitions (define once, used throughout):**

- **euid / egid**: effective uid / gid — the credentials the kernel uses for permission checks against this process.
- **passwd entry**: a row in `/etc/passwd` (or any NSS source) keyed by user name. Resolved by `getpwnam_r(3)`. The `nix` crate exposes this as `nix::unistd::User::from_name(name) -> Result<Option<User>, Errno>`. The `User` struct has fields `name: String`, `uid: Uid`, `gid: Gid`, plus passwd/gecos/dir/shell.
- **`PrivilegeErr`**: the only error type exported by this module. After this plan, three variants — `UserNotFound`, `WrongUser`, `Syscall`.
- **`run_as(name)`**: the public entry point. Verifies the calling process is running as the named user; returns `Ok(())` on match, or one of the three error variants on mismatch / lookup failure.
- **Pure verification**: the verification logic is a free function `fn verify(euid: Uid, egid: Gid, user: &User, name: &str, argv0: String) -> Result<(), PrivilegeErr>`. It calls no syscalls and takes no `&self` — every input is a parameter, so unit tests construct expected values inline and assert directly. No fake-syscall harness is needed.

**Repo conventions** (from `agent/AGENTS.md`):

- **Import ordering**: three groups separated by a blank line and a comment header, in this order: `// standard crates`, `// internal crates`, `// external crates`. `crate::*` and `self::*` go under `// internal crates`.
- **Error types** derive `thiserror::Error` and implement `crate::errors::Error`. The `impl crate::errors::Error for PrivilegeErr {}` line at the bottom of `errors.rs` covers every variant.
- **Testing**: always use `./scripts/test.sh`, which runs `RUST_LOG=off cargo test --features test`. The `--features test` flag is mandatory.
- **Coverage**: each module has a `.covgate` minimum coverage percentage; `scripts/covgate.sh` enforces. The privilege gate is currently `93.88` (the previous plan referenced `44.58` — the gate has since been bumped). This plan does not modify `.covgate`.
- **Lint**: `./scripts/lint.sh` runs the import linter, `cargo fmt --check`, machete, audit, and clippy with `-D warnings`. The import linter also flags 4+ field-by-field `assert_eq!` patterns inside a single test; suppress in-place with `// lint:allow(field-by-field-assert)` if a focused test trips it.
- **Preflight**: `./scripts/preflight.sh` runs lint + covgate + tools lint + tools tests in parallel and prints `Preflight clean` on success or `Preflight FAILED (...)` on any failure. This is the single canonical "is this ready" command.

**Branch state at plan creation**: `feat/self-privilege-drop` at HEAD `e1cc7ab` ("chore: bump covgates"). Working tree clean. Base `main`. The branch is the cumulative result of the prior plans `plans/completed/20260508-privilege-system-trait.md` and `plans/completed/20260508-privilege-review-followup.md` — both of which built the machinery this plan removes.

**Out-of-scope guards (must not touch):**

- `agent/agent/.service` files / install scripts / debian packaging. The deployment story for verification-only startup is a follow-up.
- `agent/agent/src/privilege/.covgate`. Do not modify under any circumstance. If covgate enforcement fails after the rewrite, document under Surprises & Discoveries and stop — the user explicitly directed the gate to remain untouched.
- The public API surface of the privilege module: `pub fn run_as(name: &str) -> Result<(), PrivilegeErr>` and `pub use self::errors::PrivilegeErr` must remain byte-identical across every commit boundary.
- The integration test `agent/agent/tests/privilege/mod.rs::run_as_is_noop_when_already_target_user` (lines 109–128 today) — preserved verbatim including its skip pattern when `User::from_uid(euid)` returns `None` or an error.
- The integration test `run_as_rejects_non_target_user_when_not_root` is kept and only its Display assertion ripples (it doesn't directly assert the `WrongUser` Display string — re-read step 33 of M4 to confirm).

## Plan of Work

The four commits below map directly to the user-supplied milestone breakdown. Each milestone ends with `Preflight clean` and a single commit; the order minimizes chained breakage (M1 keeps the binary compiling under the simpler runtime; M2 is the destructive rewrite; M3 trims now-unreferenced error variants; M4 backfills tests).

### M1 — Revert `agent/src/main.rs` to `#[tokio::main]` (commit 1)

**Goal:** the entry point is `#[tokio::main(flavor = "multi_thread")] async fn main() { ... }` (or simply `#[tokio::main]`, which defaults to multi-thread). The `privilege::run_as("miru")` call moves back inside the async body — pure verification has no kernel-state hazard from running on a tokio worker thread.

In `agent/agent/src/main.rs`:

- Delete `fn main() { ... }` and the `let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() { ... }` / `runtime.block_on(run_main(cli_args));` block.
- Delete the `async fn run_main(cli_args: cli::Args) { ... }` wrapper.
- Replace with a single `#[tokio::main]` async fn whose body is, in order:
  1. `let cli_args = cli::Args::parse(&env::args().collect::<Vec<String>>());`
  2. `if cli_args.display_version { println!("{}", version::format()); return; }`
  3. `if let Err(e) = privilege::run_as("miru") { eprintln!("miru-agent: {e}"); std::process::exit(1); }`
  4. The original async body that today lives in `run_main`: the two `if let Some(...)` arms for provision / reprovision and the trailing `run_agent().await;`.
- The helper functions `run_provision`, `handle_provision_result`, `run_reprovision`, `handle_reprovision_result`, `run_agent`, `get_bootstrap_base_url`, `await_shutdown_signal` are unchanged.
- The `#[tokio::main]` attribute expands to a `Builder::new_multi_thread().enable_all().build()` runtime — equivalent to today's hand-rolled construction. The macro itself requires the `macros` feature on `tokio`; see Idempotence and Recovery for the feature-flag check before editing. The workspace `tokio` features include `rt-multi-thread`, `fs`, `signal` (verified at `Cargo.toml` workspace-deps level); `enable_all()` activates `time`, `io`, etc. behind those features.
- Ordering rationale: `display_version` short-circuits before `privilege::run_as` so a stub user (e.g. CI without a `miru` passwd entry) can still print `--version` for diagnostics. This matches today's order in `fn main()`.

### M2 — Collapse `privilege/mod.rs` to pure user verification (commit 2)

**Goal:** the privilege module is reduced to two pieces of logic — `lookup_user` (passwd lookup, ENOENT/ESRCH funneled to `UserNotFound`, other errno → `Syscall`) and `verify` (compare effective uid/gid against the resolved user). The `System` trait, `FakeSystem`, `drop_to`, `is_root_user`, `run_as_with` are deleted. The `WrongUser` Display string is updated to suggest `sudo -u <expected>`.

Delete files entirely:

- `agent/agent/src/privilege/system.rs`
- `agent/agent/src/privilege/fake.rs`

Rewrite `agent/agent/src/privilege/mod.rs` to the following exact shape (target ~80 source lines):

    // standard crates
    // (none)

    // internal crates
    pub mod errors;
    pub use self::errors::PrivilegeErr;
    use crate::trace;

    // external crates
    use nix::errno::Errno;
    use nix::unistd::{Gid, Uid, User};

    /// Verify that the current effective user matches `name`. Returns
    /// `Ok(())` on match. Returns `WrongUser` if euid or egid does not match,
    /// `UserNotFound` if `name` has no passwd entry, or `Syscall` if the
    /// passwd lookup itself fails.
    pub fn run_as(name: &str) -> Result<(), PrivilegeErr> {
        let user = lookup_user(name)?;
        let euid = nix::unistd::geteuid();
        let egid = nix::unistd::getegid();
        let argv0 = std::env::args()
            .next()
            .unwrap_or_else(|| "miru-agent".into());
        verify(euid, egid, &user, name, argv0)
    }

    fn lookup_user(name: &str) -> Result<User, PrivilegeErr> {
        let not_found = || PrivilegeErr::UserNotFound {
            name: name.to_string(),
            trace: trace!(),
        };
        match User::from_name(name) {
            Ok(Some(u)) => Ok(u),
            // Some libc implementations report missing entries via ENOENT/ESRCH
            // rather than `Ok(None)`.
            Ok(None) => Err(not_found()),
            Err(Errno::ENOENT | Errno::ESRCH) => Err(not_found()),
            Err(e) => Err(PrivilegeErr::Syscall {
                call: "getpwnam_r",
                errno: e,
                trace: trace!(),
            }),
        }
    }

    fn verify(
        euid: Uid,
        egid: Gid,
        user: &User,
        name: &str,
        argv0: String,
    ) -> Result<(), PrivilegeErr> {
        if user.uid != euid || user.gid != egid {
            return Err(PrivilegeErr::WrongUser {
                expected: name.to_string(),
                actual_uid: euid.as_raw(),
                actual_gid: egid.as_raw(),
                expected_uid: user.uid.as_raw(),
                expected_gid: user.gid.as_raw(),
                argv0,
                trace: trace!(),
            });
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        // internal crates
        use super::*;

        #[test]
        fn lookup_user_returns_root_for_root() {
            // Exercises the production path against the host passwd database;
            // root is guaranteed present on every Linux system.
            let user = lookup_user("root").expect("root should always be present");
            assert_eq!(user.uid.as_raw(), 0);
            assert_eq!(user.gid.as_raw(), 0);
            assert_eq!(user.name, "root");
        }
    }

This `mod tests` block is the seed for M2; M4 expands it with three more `verify`-targeted tests. The seed test alone keeps the module non-empty so the file compiles cleanly under `cargo test --features test` at M2's tip.

In `agent/agent/src/privilege/errors.rs`:

- Update the `#[error(...)]` attribute on `WrongUser`. Today:

      #[error(
          "miru-agent must be run as root or the '{expected}' user, but is running as \
           uid {actual_uid} gid {actual_gid} (expected uid {expected_uid} gid \
           {expected_gid}).\nTry: sudo {argv0} ..."
      )]

  Replace with:

      #[error(
          "miru-agent must be run as the '{expected}' user, but is running as \
           uid {actual_uid} gid {actual_gid} (expected uid {expected_uid} gid \
           {expected_gid}).\nTry: sudo -u {expected} {argv0} ..."
      )]

  Two changes: drop "or root" from the prose (root is no longer an accepted invocation; root must explicitly use `sudo -u miru ...`), and switch the suggestion from `sudo {argv0}` to `sudo -u {expected} {argv0}`.

- The `PostDropMismatch` and `PrivilegedSupplementaryGroup` variants stay in this commit (they are removed in M3). The `Syscall` variant is unchanged. The `UserNotFound` variant is unchanged.

The integration tests in `agent/agent/tests/privilege/mod.rs` continue to assert the existing five-variant Display behavior in this commit — this temporarily passes because the variants still exist. M3 removes the variants and M4 prunes the integration assertions in the same commit. The intermediate state at M2's tip is therefore: source compiles, tests pass, two `PrivilegeErr` variants remain unused (the compiler does not warn on unused enum variants without an explicit lint).

Verify with `grep` that no source file outside `mod.rs` and `errors.rs` references the removed names. The complete deletion checklist (gathered during research) — every reference must be gone:

- `drop_to` (declarations + tests)
- `run_as_with` (declarations + tests)
- `is_root_user` (declarations + tests)
- `verify_effective_user` (declaration)
- `RealSystem`, `FakeSystem` (declarations + tests)
- `System` (the privilege trait — declarations + bounds + impls)
- `PostDropMismatch`, `PrivilegedSupplementaryGroup` (only in `errors.rs` and integration test until M3 removes them)
- `setresuid`, `setresgid`, `initgroups`, `getresuid`, `getresgid`, `getgroups` (every reference)
- `ResUid`, `ResGid` (alias / type / use)
- `nix::unistd::Group` (already not referenced today — confirm)
- `c_name`, `CString` and the `std::ffi::CString` import (used only by `drop_to`)

After M2's edits, the privilege module's only external imports beyond `crate::*` are `nix::errno::Errno` and `nix::unistd::{Gid, Uid, User}`. The `nix::unistd::geteuid` / `nix::unistd::getegid` calls are direct (no aliasing).

### M3 — Drop `PostDropMismatch` and `PrivilegedSupplementaryGroup` (commit 3)

**Goal:** `PrivilegeErr` has exactly three variants — `UserNotFound`, `WrongUser`, `Syscall`. No source path constructs the deleted variants (M2 already removed every constructor); this commit makes the deletion explicit so the public API surface accurately reflects what the module can return.

In `agent/agent/src/privilege/errors.rs`:

- Delete the `PostDropMismatch { ... }` variant and its `#[error("post-drop verification failed: ...")]` attribute.
- Delete the `PrivilegedSupplementaryGroup { gid, trace }` variant and its `#[error("post-drop verification failed: supplementary ...")]` attribute.
- The remaining file is roughly 30 lines.

In `agent/agent/tests/privilege/mod.rs`, in `privilege_err_display_messages_are_human_readable`:

- Delete the `let post_drop = PrivilegeErr::PostDropMismatch { ... };` block and its eight `assert!(s.contains(...))` calls.
- Delete the `let priv_supp = PrivilegeErr::PrivilegedSupplementaryGroup { ... };` block and its two `assert!(s.contains(...))` calls.
- Update the `WrongUser` Display assertion to also check the new suggestion: replace `assert!(s.contains("sudo"));` with `assert!(s.contains("sudo -u miru /usr/sbin/miru-agent"));`. (The other `WrongUser` assertions about uid/gid/argv0 fields stay.)

After M3, the integration test asserts Display for exactly three variants (`UserNotFound`, `WrongUser`, `Syscall`) and the new `WrongUser` string asserts the `sudo -u <expected> <argv0>` form.

### M4 — Pure `verify()` unit tests (commit 4)

**Goal:** the `mod tests` block in `agent/agent/src/privilege/mod.rs` exercises the pure `verify` function and the surviving `lookup_user` real-system path. No fake-system harness is reintroduced. Test count is small (target ~80 lines) and every test is deterministic without root.

In `agent/agent/src/privilege/mod.rs`, in the `mod tests` block, retain the seed test from M2 (`lookup_user_returns_root_for_root`) and add the following tests:

- `verify_returns_ok_when_euid_and_egid_match`: construct a `User { name: "miru", uid: Uid::from_raw(1234), gid: Gid::from_raw(5678), passwd: CString::new("x").unwrap(), gecos: CString::new("").unwrap(), dir: PathBuf::from("/nonexistent"), shell: PathBuf::from("/bin/false") }` (the `User` struct fields match `nix::unistd::User`), call `verify(Uid::from_raw(1234), Gid::from_raw(5678), &user, "miru", "miru-agent".into())`, expect `Ok(())`.
- `verify_returns_wrong_user_when_uid_mismatches`: same `user` (uid=1234, gid=5678), call `verify(Uid::from_raw(2000), Gid::from_raw(5678), &user, "miru", "/usr/sbin/test-agent".into())`, expect `Err(PrivilegeErr::WrongUser { expected: "miru", actual_uid: 2000, actual_gid: 5678, expected_uid: 1234, expected_gid: 5678, argv0: "/usr/sbin/test-agent", .. })`.
- `verify_returns_wrong_user_when_gid_mismatches`: same `user`, call `verify(Uid::from_raw(1234), Gid::from_raw(9999), &user, "miru", "miru-agent".into())`, expect `Err(WrongUser { actual_uid: 1234, actual_gid: 9999, .. })`.

Each test imports the necessary types via `use super::*;` (already in scope from `mod.rs`'s top-level use lines for `Gid`, `Uid`, `User`, `PrivilegeErr`). `CString` and `PathBuf` need explicit imports inside `mod tests`:

    // standard crates
    use std::ffi::CString;
    use std::path::PathBuf;

    // internal crates
    use super::*;

A small helper to build the `User` fixture keeps each test focused:

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

The `lookup_user_returns_root_for_root` test from M2 stays and exercises the production `User::from_name` call against the host passwd database (root is guaranteed present on every Linux system). The errno-funnel branches in `lookup_user` (`Err(ENOENT|ESRCH) → UserNotFound`, `Err(other) → Syscall`) lose all automated coverage in this collapse: the integration test only exercises `Ok(Some(_))` (WrongUser path) or `Ok(None)` (UserNotFound path), depending on whether the host has a `miru` passwd entry. This is the explicit tradeoff documented in the user brief: pure-function design, smallest correct implementation, even at the cost of automated coverage of two error mappings that effectively never fire in practice.

In `agent/agent/tests/privilege/mod.rs`:

- The three tests stay: `run_as_rejects_non_target_user_when_not_root` (verbatim), `privilege_err_display_messages_are_human_readable` (post-M3 trim), `run_as_is_noop_when_already_target_user` (verbatim, including the skip pattern).
- The `run_as_rejects_non_target_user_when_not_root` body already accepts both `WrongUser` and `UserNotFound` outcomes (lines 21–38) and does not assert the Display string — no edit needed in M4. The skip-when-euid-is-miru pattern is **not** present in this test today (it was on the to-do list in the previous plan but deferred); since the user brief says "existing skip pattern when current user IS miru", and the only file with that pattern is `run_as_is_noop_when_already_target_user`, leave the rejection test as-is.

  If during implementation the rejection test panics on a host where the runner *is* `miru`, that is the `run_as_is_noop_when_already_target_user` happy path firing in the wrong test — surface in Surprises & Discoveries and decide whether to add the skip guard. Default: do not pre-emptively add it.

After M4 the test surface is:

- Inline (`mod.rs`): `lookup_user_returns_root_for_root` + three `verify` tests = four tests.
- Integration (`tests/privilege/mod.rs`): three tests (rejection, Display, noop).

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

### M1 — Revert `agent/src/main.rs` to `#[tokio::main]`

4. Edit `agent/agent/src/main.rs`:

   - Replace `fn main()` (today lines 28–52) and `async fn run_main(cli_args: cli::Args) { ... }` (today lines 54–68) with a single `#[tokio::main]` async fn whose body, in order, is: parse args → version short-circuit → `privilege::run_as("miru")` → provision arm → reprovision arm → `run_agent().await`. See M1's "Plan of Work" entry for the exact ordering.
   - Confirm via `grep "tokio::runtime::Builder" agent/agent/src/main.rs` that no occurrences remain.
   - Confirm via `grep "fn run_main" agent/agent/src/main.rs` that no occurrences remain.

5. Build and run scoped tests, lint, and preflight:

       cargo build -p miru-agent
       ./scripts/test.sh
       ./scripts/preflight.sh

   Expected: clean build; privilege tests pass; final line `Preflight clean`. The privilege module is untouched in M1, so all existing privilege tests (including the FakeSystem-driven ones) still pass.

6. Commit M1:

       git add agent/agent/src/main.rs
       git commit -m "refactor(main): revert to #[tokio::main] entry point"

### M2 — Collapse `privilege/mod.rs` to pure user verification

7. Delete the trait-and-fake files:

       git rm agent/agent/src/privilege/system.rs
       git rm agent/agent/src/privilege/fake.rs

8. Overwrite `agent/agent/src/privilege/mod.rs` with the rewrite shown in M2 of "Plan of Work". Verify the file is roughly 80 source lines + a small `mod tests` block (containing the seed test `lookup_user_returns_root_for_root`).

9. Edit `agent/agent/src/privilege/errors.rs` — update only the `WrongUser` `#[error(...)]` attribute string. The `PostDropMismatch` and `PrivilegedSupplementaryGroup` variants stay in this commit; M3 removes them. The exact replacement string is in M2's "Plan of Work" entry.

10. Verify nothing outside `mod.rs` / `errors.rs` references the deleted names:

        cd /home/ben/miru/workbench1/repos/agent
        grep -rn -E "drop_to|run_as_with|is_root_user|verify_effective_user|RealSystem|FakeSystem|privilege::system|privilege::fake" --include="*.rs"

    Expected: no matches outside the two integration tests (`tests/privilege/mod.rs`) — and even those should be clean since none of them reference the internal helpers.

11. Verify the syscall-list deletions:

        grep -rn -E "setresuid|setresgid|initgroups|getresuid|getresgid|getgroups|nix::unistd::Group|ResUid|ResGid" --include="*.rs" agent/

    Expected: no matches anywhere under `agent/`.

12. Build and test:

        cargo build -p miru-agent
        ./scripts/test.sh

    Expected: clean build; the seed test `lookup_user_returns_root_for_root` passes; the existing integration tests still pass (the two doomed variants are still constructed by the integration Display test — that's fine until M3).

13. Run preflight:

        ./scripts/preflight.sh

    Expected: final line `Preflight clean`. **Caveat**: covgate enforcement may fail because the test suite shrank dramatically. If it does, **stop**, record under Surprises & Discoveries with the exact percentage covgate reports, and consult the user before proceeding. Per the user brief, `.covgate` must not be modified. Possible recovery options to discuss: (a) accept the failure and lower the gate (out of scope per the brief), (b) reintroduce a tiny stub fake to recover coverage (against the spirit of the simplification), (c) defer the simplification PR until the coverage policy is revisited. Default: stop and ask.

14. Commit M2. The deletions of `system.rs` and `fake.rs` are already staged by `git rm` in step 7 and ride along automatically:

        git add agent/agent/src/privilege/mod.rs agent/agent/src/privilege/errors.rs
        git status   # expected: deleted system.rs, deleted fake.rs, modified mod.rs, modified errors.rs
        git commit -m "refactor(privilege): collapse module to pure user verification"

### M3 — Drop `PostDropMismatch` and `PrivilegedSupplementaryGroup`

15. Edit `agent/agent/src/privilege/errors.rs`. Delete the `PostDropMismatch { ... }` variant (and its `#[error("post-drop verification failed: expected uid=...")]` attribute) and the `PrivilegedSupplementaryGroup { gid, trace }` variant (and its `#[error("post-drop verification failed: supplementary ...")]` attribute). The remaining file has three variants and roughly 30 lines.

16. Edit `agent/agent/tests/privilege/mod.rs`, function `privilege_err_display_messages_are_human_readable`:

    - Delete the `let post_drop = PrivilegeErr::PostDropMismatch { ... };` block and the eight following `assert!(s.contains(...))` calls (today roughly lines 79–98).
    - Delete the `let priv_supp = PrivilegeErr::PrivilegedSupplementaryGroup { gid: 0, trace: dummy_trace() };` block and the two following `assert!(s.contains(...))` calls (today roughly lines 100–106).
    - In the `WrongUser` Display assertion block, replace `assert!(s.contains("sudo"));` with `assert!(s.contains("sudo -u miru /usr/sbin/miru-agent"));` so the new Display string is verified.

17. Build and test:

        cargo build -p miru-agent
        ./scripts/test.sh

    Expected: clean build; all tests pass. The `WrongUser` Display test now asserts the `sudo -u miru /usr/sbin/miru-agent` substring.

18. Run preflight:

        ./scripts/preflight.sh

    Expected: final line `Preflight clean`. (Same covgate caveat as step 13 — the test count is unchanged in M3, so coverage is unchanged from M2's tip.)

19. Commit M3:

        git add agent/agent/src/privilege/errors.rs agent/agent/tests/privilege/mod.rs
        git commit -m "refactor(privilege): drop PostDropMismatch and PrivilegedSupplementaryGroup variants"

### M4 — Pure `verify()` unit tests

20. Edit `agent/agent/src/privilege/mod.rs` `mod tests`. Add the three `verify`-targeted tests (`verify_returns_ok_when_euid_and_egid_match`, `verify_returns_wrong_user_when_uid_mismatches`, `verify_returns_wrong_user_when_gid_mismatches`) and the small `fixture_user` helper from M4's "Plan of Work" entry. Required imports inside `mod tests`:

        // standard crates
        use std::ffi::CString;
        use std::path::PathBuf;

        // internal crates
        use super::*;

    The seed test `lookup_user_returns_root_for_root` from M2 stays.

21. Build and test:

        cargo build -p miru-agent
        ./scripts/test.sh

    Expected: four privilege unit tests pass (`lookup_user_returns_root_for_root`, `verify_returns_ok_when_euid_and_egid_match`, `verify_returns_wrong_user_when_uid_mismatches`, `verify_returns_wrong_user_when_gid_mismatches`). All three integration tests still pass.

22. Run preflight:

        ./scripts/preflight.sh

    Expected: final line `Preflight clean`. (If covgate failed at M2/M3, this is where the recovered coverage from the new `verify` tests can offset the loss — but unit coverage of `verify` is small. Same caveat applies; stop and surface if covgate trips.)

23. Smoke check the binary:

        cargo run --package miru-agent -- --version

    Expected: prints the version string and exits 0. Order of operations in `#[tokio::main] async fn main()`: parse args → version-print-and-return short-circuits before `privilege::run_as` is reached, so this works without a `miru` user on the dev host.

24. Commit M4:

        git add agent/agent/src/privilege/mod.rs
        git commit -m "test(privilege): replace fake-driven tests with pure verify() unit tests"

### Push

25. Push all four new commits:

        git push origin feat/self-privilege-drop

    Expected: `git status` reports `Your branch is up to date with 'origin/feat/self-privilege-drop'`. The existing PR (if open) picks up the commits; no new PR is opened.

## Validation and Acceptance

The plan is complete when **all** of the following hold from `/home/ben/miru/workbench1/repos/agent`:

- `./scripts/preflight.sh` final line reads `Preflight clean`.
- `./scripts/test.sh` (which runs `RUST_LOG=off cargo test --features test --package miru-agent`; the script does not forward extra CLI arguments — for a name-filtered run use `RUST_LOG=off cargo test --features test -p miru-agent privilege` directly) passes — every test that exists post-rewrite:
  - Inline (`agent/agent/src/privilege/mod.rs::tests`): `lookup_user_returns_root_for_root`, `verify_returns_ok_when_euid_and_egid_match`, `verify_returns_wrong_user_when_uid_mismatches`, `verify_returns_wrong_user_when_gid_mismatches`.
  - Integration (`agent/agent/tests/privilege/mod.rs`): `run_as_rejects_non_target_user_when_not_root`, `privilege_err_display_messages_are_human_readable`, `run_as_is_noop_when_already_target_user`.
- `cargo build -p miru-agent` succeeds with no warnings.
- `cargo fmt -p miru-agent -- --check` exits 0 from the workspace root (matches what `./scripts/lint.sh` runs).
- `cargo clippy --all-features --package miru-agent -- -D warnings` exits 0.
- `git log --oneline origin/feat/self-privilege-drop..HEAD` shows exactly four new commits, in this order:
  1. `refactor(main): revert to #[tokio::main] entry point`
  2. `refactor(privilege): collapse module to pure user verification`
  3. `refactor(privilege): drop PostDropMismatch and PrivilegedSupplementaryGroup variants`
  4. `test(privilege): replace fake-driven tests with pure verify() unit tests`

**Constraints** that must hold at every commit boundary:

- Public API surface unchanged: `pub fn privilege::run_as(name: &str) -> Result<(), PrivilegeErr>` (signature byte-for-byte preserved); `pub use self::errors::PrivilegeErr`. `PrivilegeErr` ends with three variants — `UserNotFound { name, trace }`, `WrongUser { expected, actual_uid, actual_gid, expected_uid, expected_gid, argv0, trace }`, `Syscall { call, errno, trace }`. The set of variants narrows; this is a breaking change to the error enum shape, but the only constructors live inside the privilege module and the only consumer is `agent/agent/src/main.rs` which uses `Display`-only formatting (`eprintln!("miru-agent: {e}")`), so no external caller is affected.
- `agent/agent/src/privilege/.covgate` is unchanged. If covgate enforcement fails, surface in Surprises & Discoveries and stop (see step 13).
- `./scripts/preflight.sh` reports `Preflight clean` at the tip of every milestone commit (not only at M4). This enables `git bisect` and matches the per-milestone-commit convention.

**Behavioral acceptance for reviewers:**

- Inspect `agent/agent/src/main.rs` diff: `#[tokio::main]` re-added; explicit `Builder` block removed; `run_main` removed; the body is the same as today's plus the `privilege::run_as` call.
- Inspect `agent/agent/src/privilege/mod.rs` diff: file shrinks from 678 lines to roughly 100 (source + tests). `pub fn run_as` calls `lookup_user` then `verify`; `verify` is a free function with no `&self`.
- Inspect `git status` between commits: `agent/agent/src/privilege/system.rs` and `agent/agent/src/privilege/fake.rs` are deleted at M2's tip.
- Inspect `agent/agent/src/privilege/errors.rs` diff at M2: `WrongUser` `#[error]` string changes to include `sudo -u {expected} {argv0}` and drop "or root". At M3: `PostDropMismatch` and `PrivilegedSupplementaryGroup` variants are gone.
- Inspect `agent/agent/tests/privilege/mod.rs` diff at M3: the Display test asserts only three variants; the `WrongUser` assertion includes `sudo -u miru /usr/sbin/miru-agent`.
- Run the binary on a dev host as a non-`miru` user: `cargo run --package miru-agent`. Expected stderr (subject to passwd resolution): either `miru-agent: user 'miru' not found in /etc/passwd; ...` or `miru-agent: miru-agent must be run as the 'miru' user, but is running as uid X gid Y (...). Try: sudo -u miru <argv0> ...`. Process exits 1.

**Acceptance gate**: the orchestrator must observe `Preflight clean` at the tip of M4 before pushing. If preflight fails after any milestone, fix the cause inside that milestone's commit (`git reset --soft HEAD~1` and re-edit) — do not stack a fixup commit.

## Idempotence and Recovery

- All edits are pure-source changes; reapplying any milestone is safe. Each milestone ends with a single commit, so `git revert <sha>` or `git reset --soft HEAD~1` rolls back atomically.
- **M1 ripple risk**: `#[tokio::main]` requires `tokio` features `rt-multi-thread` + `macros`. The workspace `tokio` features are `["rt-multi-thread", "fs", "signal"]` in `Cargo.toml`; `macros` is not currently listed. Confirm before editing — if `macros` is absent, add it to the workspace `tokio` features or use `#[tokio::main(flavor = "multi_thread")]` (which still requires the `macros` feature to expand the attribute). If `cargo build` fails after step 4 with "the macro `tokio::main` is not in scope" or similar, add `"macros"` to the workspace `tokio` features and retry. Decision Log this if the feature was missing.
- **M2 ripple risk — coverage drop**: removing the FakeSystem-driven test suite drops measured coverage of `privilege/mod.rs`. If `./scripts/covgate.sh` reports the gate failed, **stop** and surface in Surprises & Discoveries. Do not lower `.covgate`.
- **M2 ripple risk — leftover references**: if step 10 finds any reference to `drop_to` / `RealSystem` / `System` / etc. outside `mod.rs`, fix in place before committing M2. The complete deletion checklist is in M2's "Plan of Work" entry.
- **M2 ripple risk — `nix` features**: the pure-`verify` rewrite uses `nix::unistd::User::from_name`, `nix::unistd::geteuid`, `nix::unistd::getegid`, `nix::errno::Errno`. All four are already enabled in the workspace `nix` features (as evidenced by today's working code). No `Cargo.toml` change.
- **M3 ripple risk**: deleting variants from a `pub enum` is a breaking change. Run `grep -rn "PostDropMismatch\|PrivilegedSupplementaryGroup" --include="*.rs"` and confirm the only matches are inside `errors.rs` and the integration test (both edited in M3). If any source file constructs or matches these variants, the build will fail — fix in place.
- **M4 ripple risk**: the `User` struct constructor in `fixture_user` requires every field (`passwd`, `gecos`, `dir`, `shell`) to be present. The exact field set comes from `nix = "0.31.2"`; if the `nix` version pins differently, `cargo build` after step 21 fails with "missing field `<name>`". Fix by reading the current `nix::unistd::User` definition (`cargo doc -p nix --open` or `target/doc/nix/unistd/struct.User.html`) and matching every field.
- **Reverting the entire simplification**: `git revert --no-commit <m1-sha>^..<m4-sha>` applies four inverse patches into the working tree (review with `git diff --staged`); then `git commit -m "revert: privilege simplification"` lands a single revert commit that returns the privilege module and `main.rs` to the state at HEAD `e1cc7ab`. The earlier System-trait refactor is restored.
- **Re-running a milestone**: every command shown is idempotent (`cargo build`, `cargo test`, `./scripts/preflight.sh`). Source edits are absolute (specific functions and lines), not appended, so re-applying after `git reset --soft HEAD~1` is safe.
