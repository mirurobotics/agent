# Self-privilege-drop in `miru-agent` so `sudo miru-agent ...` works

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add a privilege-drop module to the `miru-agent` Rust crate, wire it into `main`, update install scripts and docs, add tests. |

This plan lives in `agent/plans/` because all code, scripts, and docs that change live in this repo (`/home/ben/miru/workbench1/repos/agent`). No other Miru repo is read or written.

## Purpose / Big Picture

Today, anyone provisioning or reprovisioning a device must run the agent as the system user `miru` because `/var/lib/miru/` is owned by `miru:miru`. The current documented incantation is

    sudo -u miru -E env MIRU_PROVISIONING_TOKEN=... /usr/sbin/miru-agent provision \
        --backend-host=... --mqtt-broker-host=...

This is awkward (`sudo -u <user>` is uncommon) and a footgun (env vars set before `sudo -u` are stripped unless `-E` / `--preserve-env` is used).

After this change, the same operation works as

    sudo MIRU_PROVISIONING_TOKEN=... /usr/sbin/miru-agent provision \
        --backend-host=... --mqtt-broker-host=...

`miru-agent` detects at startup that it is running as root (EUID 0), looks up the `miru` user/group, and drops privileges to it before doing any work. If the binary is launched directly as the `miru` user (as systemd does via `User=miru`), startup is a no-op. If launched as some other non-root user, the agent prints a clear error pointing the user at the correct invocation and exits with a non-zero status.

User-visible acceptance: a developer with a fresh `.deb` install can run

    sudo MIRU_PROVISIONING_TOKEN=tok_test miru-agent provision \
        --backend-host=https://api.miru.example --mqtt-broker-host=mqtt.miru.example

and observe the same provisioning success message they get today with `sudo -u miru -E env ...`. The systemd unit `miru.service` (which already sets `User=miru`) continues to start the agent without modification.

## Progress

- [x] M1: Add the privilege-drop helper (new module `agent/agent/src/privilege/mod.rs`) with constants, lookup, drop, and a public entry point. No call sites yet. (2026-05-07)
- [x] M2: Wire `privilege::ensure_dropped_or_already_unprivileged()` into `agent/agent/src/main.rs` immediately after the `--version` early-return and before any `provision` / `reprovision` / `run_agent` dispatch. (2026-05-07)
- [x] M3: Update install scripts (`scripts/install/*.sh` and `scripts/jinja/templates/partials/utils/activate.sh`) to drop `sudo -u miru -E env ...` and use `sudo ...` instead. (2026-05-07)
- [x] M4: Update agent docs (`README.md`, `ARCHITECTURE.md` if it documents the invocation, and any other markdown referencing `sudo -u miru`). No-op: pre-implementation grep confirmed neither file contains `sudo -u miru`; only matches under `plans/active/` (historical) remain. (2026-05-07)
- [ ] M5: Run `./scripts/preflight.sh` and confirm it reports `Preflight clean` (mandatory verification gate before publishing).

Use timestamps when you complete steps.

## Surprises & Discoveries

- 2026-05-07 (impl): The `errno` field in `PrivilegeErr::Syscall` was originally specified to be sourced via `*libc::__errno_location()` directly inline; pulled it into a tiny private `errno_now()` helper to keep the three syscall-failure sites readable. Behavior unchanged.
- 2026-05-07 (impl): Plan said the install/activate scripts had matching content, and they did — all 7 lines were verbatim identical. Replaced uniformly. The exit-code numbering on lines 291 vs 382 is just a position artifact (different scripts).
- 2026-05-07 (impl): M4 (docs) found no occurrences of `sudo -u miru` outside of `plans/active/`. Ran the pre-condition grep against `agent/`; only the two historical-plan files matched, both of which the plan instructs to leave untouched. M4 commits no changes.

## Decision Log

- Decision: Use the `libc` crate directly (already a transitive dependency in `Cargo.lock`) for `getpwnam_r`, `setgid`, `setuid`, `initgroups`, rather than the `users = "0.11.0"` workspace dep that is currently unused, and rather than adding `nix`.
  Rationale: `libc` is already vendored. The `users` crate's `0.11.0` is unmaintained (last release 2020) and its `get_user_by_name` calls the same syscalls under the hood; pulling in `users` would add 4 KLOC of unused code. `nix` would add another large dep tree just for three syscalls. Calling `libc` directly via a small `unsafe` block (~25 lines) is simpler and matches how `agent/src/openssl` and similar low-level code is handled in this crate. Re-evaluate if a future task needs more user/group manipulation.
  Date/Author: 2026-05-07 / plan author.

- Decision: Drop privileges immediately after the `--version` early-return in `fn main()` and before dispatching to `run_provision` / `run_reprovision` / `run_agent`. Argument parsing (`cli::Args::parse`) runs first because it is pure (no filesystem or network I/O) and the `--version` branch must continue to work for any user (it is used by packaging smoke tests). The privilege drop comes next.
  Rationale: We want the smallest possible window between process start and de-privileging while keeping `miru-agent --version` runnable as any user. Logging is initialized inside `run_provision` / `run_reprovision` / `run_agent` (each has its own `logs::init`), so the drop must complete before any of them runs. Errors from the drop are written to `eprintln!` since `tracing` is not yet wired up.
  Date/Author: 2026-05-07 / plan author.

- Decision: Env vars are preserved across `setuid` automatically by the Linux kernel — the `setuid(2)` syscall does not touch the process environment block. We rely on this and document it inline in `privilege/mod.rs`. No explicit re-export step is needed.
  Rationale: `man 2 setuid` is silent on environment because environment lives in user-space process memory, not kernel state; the kernel only touches credentials. Verified empirically: `unsafe { libc::setuid(uid); std::env::var("FOO") }` returns the value set by the parent. Adding explicit re-export logic would be cargo-culting.
  Date/Author: 2026-05-07 / plan author.

- Decision: When invoked as a non-root, non-`miru` user, error out with a message that quotes the corrected invocation, and exit with status `1` via `std::process::exit`. Do not panic.
  Rationale: A clean, actionable error is better UX than a backtrace. Status `1` matches the existing `handle_provision_result` error path.
  Date/Author: 2026-05-07 / plan author.

- Decision: Gate the privilege-drop module on `cfg(target_os = "linux")`. On other targets the helper compiles to a no-op `Ok(())`.
  Rationale: The agent only ships on Linux (`.deb` packages, systemd unit). Developers may run `cargo test` on macOS where `getpwnam_r` for `miru` will fail. Stubbing on non-Linux keeps unit tests cross-platform. Production behavior is unchanged.
  Date/Author: 2026-05-07 / plan author.

- Decision: Do not modify the systemd unit `build/debian/miru.service`. It already declares `User=miru` and `Group=miru`; with self-drop wired in, the agent starts already-as-`miru`, the EUID check matches, and the drop is a no-op (function returns `Ok(())` on the "already correct user" path).
  Rationale: Out of scope per task brief. Verified by reading `build/debian/miru.service` (lines 10–11).
  Date/Author: 2026-05-07 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Crate under change: `miru-agent` at `agent/agent/` (manifest `agent/agent/Cargo.toml`, workspace root `agent/Cargo.toml`). Binary entry point: `agent/agent/src/main.rs`. Module list: `agent/agent/src/lib.rs` (currently 22 modules; this plan adds a 23rd — `privilege`).

**Today's `main.rs` flow** (verified in repo):

1. `let cli_args = cli::Args::parse(&env::args().collect::<Vec<String>>());` — pure parse, no I/O.
2. If `--version`, print and return.
3. If `provision`, call `run_provision(...)` — initializes logging into a temp dir, reads `MIRU_PROVISIONING_TOKEN` via `provisioning::read_token_from_env()`, runs the provisioning flow.
4. If `reprovision`, similar to provision.
5. Else, `run_agent()` — long-running daemon path used by the systemd unit.

**Provisioning token read site:** `agent/agent/src/provisioning/shared.rs:14`:

    const TOKEN_ENV_VAR: &str = "MIRU_PROVISIONING_TOKEN";

`std::env::var(TOKEN_ENV_VAR)` is called at the top of `run_provision` / `run_reprovision`. Because `setuid(2)` does not touch the process environment, this read continues to succeed after privileges are dropped.

**System layout (from `build/debian/postinst`):**

- A system group `miru` is created via `groupadd -r miru`.
- A system user `miru` is created via `useradd -r -g miru -s /bin/false miru`.
- `/var/lib/miru`, `/var/log/miru`, `/srv/miru` are owned `miru:miru` mode `755`.

**Systemd unit (`build/debian/miru.service`):** Type=simple, `User=miru`, `Group=miru`, `ExecStart=/usr/sbin/miru-agent`. With self-drop, this still works: the agent starts already EUID=miru-uid and the drop helper returns early.

**Existing `sudo -u miru ...` references** (full sweep run on 2026-05-07; update the search before implementing in case anything new lands):

    grep -rn 'sudo -u miru\|sudo  *-u' agent/ \
        --include='*.md' --include='*.sh' --include='*.service' --include='*.yml' --include='*.yaml' --include='*.toml'

Hits today (relative to repo root `agent/`):

- `scripts/install/install.sh:291`
- `scripts/install/staging-install.sh:291`
- `scripts/install/uat-install.sh:291`
- `scripts/install/provision.sh:382`
- `scripts/install/staging-provision.sh:382`
- `scripts/install/uat-provision.sh:382`
- `scripts/jinja/templates/partials/utils/activate.sh:35`
- `plans/active/20260406-agent-absolute-config-paths.md:897` — historical doc inside an active plan; do **not** rewrite, the plan is a record of past work.

Note: the install/activate scripts currently invoke `/usr/sbin/miru-agent --install $args` with `MIRU_ACTIVATION_TOKEN` set. This subcommand and env var name do not match anything in `agent/agent/src/cli/mod.rs` today (which only handles `provision` / `reprovision` and reads `MIRU_PROVISIONING_TOKEN`). Treat the install scripts as stale w.r.t. the binary. **Out of scope for this plan**: aligning the install scripts with the current binary subcommands. **In scope**: replacing the `sudo -u miru -E env ...` prefix with `sudo ...` in those same scripts, leaving the rest of the line intact.

**Workspace deps (`agent/Cargo.toml`):**

- `libc` is in `Cargo.lock` (version 0.2.185) as a transitive dep.
- `users = "0.11.0"` is declared as a workspace dep but not consumed by any crate.

**Linting and testing entry points:**

- `./scripts/test.sh` — `RUST_LOG=off cargo test --features test`. Used for repeated local test runs.
- `./scripts/lint.sh` — fmt, machete, audit, clippy, custom import linter.
- `./scripts/preflight.sh` — runs lint + covgate + tools lint + tools tests in parallel; prints `Preflight clean` on success.
- `cargo clippy --package miru-agent --all-features -- -D warnings` — exact command CI runs (from `AGENTS.md`).

**Module conventions** (`agent/AGENTS.md`):

- Import grouping: standard / internal / external, separated by blank line and a comment.
- Errors derive `thiserror::Error` and implement the local `crate::errors::Error` trait. Aggregating enums use the `impl_error!` macro from `agent/src/errors/mod.rs`.
- Each new module gets a `.covgate` file with a minimum coverage percent.

**Term definitions:**

- *EUID* (effective user id): the uid the kernel uses for permission checks. After `setuid(uid)` from a process with EUID 0, EUID, RUID, and saved-set-UID all become `uid`. This is irreversible: a non-root process cannot regain root.
- *EGID* (effective group id): same, for groups.
- *Supplementary groups*: extra groups a process is a member of beyond its primary group. `initgroups(name, gid)` reads `/etc/group` and sets the process's supplementary group list to all groups containing `name`, plus the primary `gid`. Required because `setgid` alone leaves the parent's supplementary groups in place — those would be root's groups, a leak.
- *`getpwnam_r`*: the thread-safe variant of `getpwnam`. Looks up a passwd entry by name; fills in a caller-provided `passwd` struct and a buffer for the strings it points into.

## Plan of Work

### M1: Add `privilege` module

Create `agent/agent/src/privilege/mod.rs` with the following surface. All low-level work is `#[cfg(target_os = "linux")]`; on other targets the public functions return `Ok(())` immediately so cross-platform `cargo test` keeps working.

Public surface:

    pub const TARGET_USER: &str = "miru";
    pub const TARGET_GROUP: &str = "miru";

    #[derive(Debug, thiserror::Error)]
    pub enum PrivilegeErr {
        #[error("user '{name}' not found in /etc/passwd; is the miru .deb installed?")]
        UserNotFound { name: String, trace: Box<crate::errors::Trace> },
        #[error(
            "miru-agent must be run as root or the '{expected}' user, but is running as uid {actual_uid}.\n\
             Try: sudo {argv0} ..."
        )]
        WrongUser { expected: String, actual_uid: u32, argv0: String, trace: Box<crate::errors::Trace> },
        #[error("syscall '{call}' failed: {errno}")]
        Syscall { call: &'static str, errno: i32, trace: Box<crate::errors::Trace> },
        #[error("post-drop verification failed: expected uid={expected_uid} gid={expected_gid}, got uid={actual_uid} gid={actual_gid}")]
        PostDropMismatch {
            expected_uid: u32, expected_gid: u32, actual_uid: u32, actual_gid: u32,
            trace: Box<crate::errors::Trace>,
        },
    }

    impl crate::errors::Error for PrivilegeErr {}

    /// Look up the uid/gid for `TARGET_USER`. Returns `Err(PrivilegeErr::UserNotFound)`
    /// if the user is not present in /etc/passwd.
    pub fn lookup_user(name: &str) -> Result<UserInfo, PrivilegeErr> { ... }

    /// If running as root, drop to the target user. If running already as the
    /// target user, no-op. Otherwise, error out.
    ///
    /// Note on environment: the Linux setuid(2) / setgid(2) syscalls only mutate
    /// process credentials; they do NOT touch the environ block. Env vars set
    /// before this call (e.g. MIRU_PROVISIONING_TOKEN) remain readable
    /// afterwards via std::env::var. Do not introduce explicit env preservation
    /// logic — there is nothing to preserve.
    pub fn ensure_dropped_or_already_unprivileged() -> Result<(), PrivilegeErr> { ... }

    pub struct UserInfo { pub uid: u32, pub gid: u32, pub name: String }

Implementation (linux):

1. `lookup_user(name)` — call `libc::getpwnam_r` with a 1024-byte buffer (grow with `ERANGE`); on `pwd == null`, return `UserNotFound`. Convert the `*const c_char` `pw_name` back to `String` via `CStr::from_ptr(...).to_string_lossy().into_owned()`.

2. `ensure_dropped_or_already_unprivileged()`:

   - Read `unsafe { libc::geteuid() }`.
   - If `euid != 0`: look up `TARGET_USER`. If found and `info.uid == euid`, return `Ok(())` (already running as miru). Otherwise return `WrongUser { actual_uid: euid, argv0: std::env::args().next().unwrap_or_else(|| "miru-agent".into()), ... }`.
   - If `euid == 0`: look up `TARGET_USER`. Then in this exact order:
     1. `unsafe { libc::initgroups(c_name.as_ptr(), info.gid) }` — sets supplementary groups. Must happen before `setuid` (root only).
     2. `unsafe { libc::setgid(info.gid) }` — switch primary group. Must happen before `setuid`.
     3. `unsafe { libc::setuid(info.uid) }` — switch uid. Irreversible.
   - After dropping, re-read `geteuid()` / `getegid()` and verify they match `info.uid` / `info.gid`. If not, return `PostDropMismatch`.
   - Each syscall failure (return value `-1`) is wrapped as `Syscall { call: "initgroups" | "setgid" | "setuid", errno: *libc::__errno_location(), ... }`.

3. Add `pub mod privilege;` to `agent/agent/src/lib.rs` (alphabetically between `network` and `provisioning`).

4. Add `agent/agent/src/privilege/.covgate` with `80` (matches the looser bar for new modules; tighten later if desired).

Tests in `agent/agent/tests/privilege/mod.rs` (mirror src layout per `AGENTS.md`):

- `lookup_user_returns_root_for_root` — calls `lookup_user("root")` and asserts `Ok` with `uid == 0`. Skip-on-non-linux: gate the test body on `cfg(target_os = "linux")`. Rationale: `root` exists on every Linux dev machine and CI runner; `miru` does not.
- `lookup_user_round_trips_current_user` — get the current uid via `unsafe { libc::getuid() }`, look it up via `getpwuid_r` to get the name, then call `lookup_user(name)` and assert the returned `uid` matches. Linux-only.
- `lookup_user_returns_user_not_found_for_nonexistent` — call `lookup_user("__definitely_not_a_user_xyz__")` and assert `Err(PrivilegeErr::UserNotFound { .. })`.
- `ensure_dropped_or_already_unprivileged_when_running_as_self_is_ok` — call the entry point. Test process is unprivileged and not named `miru`; so this asserts `Err(PrivilegeErr::WrongUser { .. })` with `actual_uid == libc::getuid()` and `argv0` non-empty. (We do NOT spawn a subprocess; we test the wrong-user path because that is the only branch reachable from a CI runner that is neither root nor `miru`.)

Register the test module by adding `pub mod privilege;` to `agent/agent/tests/mod.rs` in alphabetical order (between `pub mod network;` and `pub mod provisioning;`). The repo uses a single integration-test target rooted at `tests/mod.rs` that re-exports each subdirectory as a module — verified by reading `agent/agent/tests/mod.rs` (26 sibling `pub mod` lines). No `[[test]]` block in `agent/agent/Cargo.toml` is needed.

### M2: Wire into `main`

Edit `agent/agent/src/main.rs`. The current top of `fn main()` is:

    let cli_args = cli::Args::parse(&env::args().collect::<Vec<String>>());

    if cli_args.display_version {
        println!("{}", version::format());
        return;
    }

    if let Some(provision_args) = cli_args.provision_args {
        ...

Insert the privilege check **between** the `display_version` early-return and the `provision_args` dispatch — i.e. after the closing `}` of the `if cli_args.display_version { ... }` block and before `if let Some(provision_args) = cli_args.provision_args { ... }`:

    if let Err(e) = privilege::ensure_dropped_or_already_unprivileged() {
        eprintln!("miru-agent: {e}");
        std::process::exit(1);
    }

This placement keeps `miru-agent --version` runnable as any user (including non-root, non-`miru`) while ensuring every other subcommand goes through the privilege check before any I/O.

Add `use miru_agent::privilege;` to the internal-crates import group (alphabetical placement: between `network` and `provisioning`).

No other source changes are needed: `read_token_from_env` continues to work because env vars survive `setuid`. Logging init in `run_*` happens after the drop, writing to dirs already owned by `miru:miru`.

### M3: Update install scripts

For each of:

- `scripts/install/install.sh:291`
- `scripts/install/staging-install.sh:291`
- `scripts/install/uat-install.sh:291`
- `scripts/install/provision.sh:382`
- `scripts/install/staging-provision.sh:382`
- `scripts/install/uat-provision.sh:382`
- `scripts/jinja/templates/partials/utils/activate.sh:35`

Replace

    sudo -u miru -E env MIRU_ACTIVATION_TOKEN="$MIRU_ACTIVATION_TOKEN" /usr/sbin/miru-agent --install $args

with

    sudo MIRU_ACTIVATION_TOKEN="$MIRU_ACTIVATION_TOKEN" /usr/sbin/miru-agent --install $args

(Note: the `--install` subcommand vs current binary mismatch is **out of scope** — see Context and Orientation. We only swap the privilege-elevation prefix.)

Leave the existing `sudo chown -R miru:miru /srv/miru` line that immediately precedes the changed line in each script in place. It is idempotent and harmless; auditing whether it is now strictly necessary is out of scope for this plan.

### M4: Update docs

- `agent/README.md` — search for any `sudo -u miru` and replace with `sudo`. (Today the README does not contain this string; verify with grep before editing.)
- `agent/ARCHITECTURE.md` — same.
- Any other markdown under `agent/` that documents the invocation. Run

      grep -rn 'sudo -u miru' agent/ --include='*.md'

  before and after; the only file that is allowed to retain `sudo -u miru` is `plans/active/20260406-agent-absolute-config-paths.md` (a historical record).

### M5: Preflight

Run `./scripts/preflight.sh` from `agent/` and confirm the final line is `Preflight clean`. M5 is a verification gate, not a code change — if preflight passes with no edits, no commit is created. If preflight surfaces issues, fix them and commit the fixes (see Commit cadence).

### Commit cadence

One commit per code-change milestone, with a Conventional-Commits-style subject:

- M1: `feat(privilege): add self-privilege-drop helper for miru user`
- M2: `feat(main): drop privileges on entry when run as root`
- M3: `chore(install): replace sudo -u miru with sudo in install scripts`
- M4: `docs: update agent invocation to sudo miru-agent`
- M5: no commit on a clean run. If preflight requires fixes, commit them as `chore: address preflight feedback for self-privilege-drop` after applying the smallest fix that makes preflight pass.

## Concrete Steps

All commands are run from `agent/` (i.e. `/home/ben/miru/workbench1/repos/agent`) unless stated.

### M1 — module scaffolding

1. Create the module:

       mkdir -p agent/src/privilege agent/tests/privilege
       $EDITOR agent/src/privilege/mod.rs   # write per "Plan of Work" M1
       $EDITOR agent/src/privilege/.covgate # contents: 80
       $EDITOR agent/tests/privilege/mod.rs # write the four tests

2. Register the module in `agent/src/lib.rs`. Add `pub mod privilege;` between `pub mod network;` and `pub mod provisioning;`.

3. Add the `libc` dependency to `agent/agent/Cargo.toml` `[dependencies]` if it is not already a direct dep:

       libc = { workspace = true }

   Check `agent/Cargo.toml` for the `libc` workspace entry; add if missing:

       [workspace.dependencies]
       ...
       libc = "0.2"

   (Confirmed at plan time: `libc` is in `Cargo.lock` transitively but is not declared as a direct workspace dep. Add it.)

4. Run a focused build:

       cargo build -p miru-agent

   Expected: clean build, no new warnings.

5. Run the new tests via the canonical project command, filtering to the new module:

       ./scripts/test.sh -- privilege

   (`scripts/test.sh` runs `RUST_LOG=off cargo test --features test`; positional args after `--` become test-name filters.) Expected: 4 tests under `privilege::*` are listed, all `ok`, with `test result: ok. 4 passed; 0 failed`. On non-Linux dev machines (e.g. macOS), the Linux-only tests are skipped via their `#[cfg(target_os = "linux")]` gate; the wrong-user test still runs and passes.

6. Commit:

       git add agent/src/privilege agent/src/lib.rs \
               agent/tests/privilege agent/tests/mod.rs \
               agent/Cargo.toml Cargo.toml Cargo.lock
       git commit -m "feat(privilege): add self-privilege-drop helper for miru user"

### M2 — wire into main

1. Edit `agent/src/main.rs`:
   - Add `use miru_agent::privilege;` to the internal-crates import group.
   - At the top of `fn main()`, insert:

         if let Err(e) = privilege::ensure_dropped_or_already_unprivileged() {
             eprintln!("miru-agent: {e}");
             std::process::exit(1);
         }

2. Build:

       cargo build -p miru-agent

   Expected: clean build.

3. Smoke test from a dev machine (non-root, not `miru`):

       ./target/debug/miru-agent --version

   Expected: prints the version string and exits 0. The `--version` branch returns before the privilege check, so this works for any user.

       ./target/debug/miru-agent provision --backend-host=https://example.invalid --mqtt-broker-host=example.invalid

   Expected: prints the wrong-user error to stderr and exits 1:

       miru-agent: miru-agent must be run as root or the 'miru' user, but is running as uid 1000.
       Try: sudo ./target/debug/miru-agent ...

4. Commit:

       git add agent/src/main.rs
       git commit -m "feat(main): drop privileges on entry when run as root"

### M3 — install scripts

1. Sweep current state:

       grep -rn 'sudo -u miru -E env' scripts/

2. For each match (7 files), rewrite the offending line as described in M3 of the plan-of-work.

3. Sanity-check no `sudo -u miru` remains in `scripts/`:

       grep -rn 'sudo -u miru' scripts/  # expected: no output

4. Commit:

       git add scripts/install scripts/jinja/templates/partials/utils/activate.sh
       git commit -m "chore(install): replace sudo -u miru with sudo in install scripts"

### M4 — docs

1. Sweep:

       grep -rn 'sudo -u miru' agent/ --include='*.md'

2. Rewrite each match outside of `plans/active/` and `plans/completed/` (those are historical records; do not edit). Edit `README.md`, `ARCHITECTURE.md`, and any other current docs.

3. Re-sweep to confirm only historical-plan matches remain.

4. Commit:

       git add README.md ARCHITECTURE.md  # plus any other touched docs
       git commit -m "docs: update agent invocation to sudo miru-agent"

### M5 — preflight

1. Refresh `Cargo.lock` if M1 added new direct deps:

       ./scripts/update-deps.sh

2. Run preflight:

       ./scripts/preflight.sh

   Expected last line:

       Preflight clean

3. On a clean run, no commit is needed (verification only). If preflight fails, fix the smallest thing required to make it pass, re-run, and commit as `chore: address preflight feedback for self-privilege-drop`.

## Validation and Acceptance

The change is accepted when **all** of the following hold:

1. **Unit tests pass.** `cargo test -p miru-agent --test mod -- privilege::` passes 4 tests on a Linux dev machine. The `lookup_user_returns_user_not_found_for_nonexistent` test fails before this change is implemented (because the module doesn't exist yet) and passes after.

2. **Preflight clean (mandatory before publishing).** `./scripts/preflight.sh` from the `agent/` repo root prints `Preflight clean` as its final line. Preflight runs `cargo fmt -- --check`, the custom import linter, `cargo machete`, `cargo audit`, `cargo clippy --package miru-agent --all-features -- -D warnings`, and `cargo test --features test` via `covgate.sh`. Changes must not be pushed to a shared branch or opened in a PR until preflight reports `Preflight clean` on the latest commit.

3. **CLI smoke test, wrong user.** On a Linux machine where the current user is neither root nor `miru`:

       ./target/debug/miru-agent provision --backend-host=https://example.invalid --mqtt-broker-host=example.invalid

   Exits with status 1 and prints a stderr line beginning with `miru-agent: miru-agent must be run as root or the 'miru' user`.

4. **CLI smoke test, root path.** On a Linux machine with the `.deb` installed (so user `miru` exists), running

       sudo MIRU_PROVISIONING_TOKEN=tok_test /usr/sbin/miru-agent provision \
           --backend-host=https://api.miru.example --mqtt-broker-host=mqtt.miru.example

   reaches the same provisioning code path as the old `sudo -u miru -E env MIRU_PROVISIONING_TOKEN=... /usr/sbin/miru-agent provision ...` invocation. The outcome (success or backend error) matches what the old form produced for the same inputs. Run with `-v` or check the agent log dir if needed.

5. **Already-miru smoke test.** On the same machine:

       sudo -u miru MIRU_PROVISIONING_TOKEN=tok_test /usr/sbin/miru-agent --version

   prints the version string. (This proves the no-op path: EUID already matches `miru`, the helper returns `Ok(())`, and `--version` continues to short-circuit.)

6. **systemd unchanged.** `systemctl restart miru.service` followed by `systemctl status miru.service` shows the service running as user `miru`, in the same way it did before this change. (Out-of-scope to verify in CI; verify manually on a test device when bumping the package version.)

7. **No stale invocations.** `grep -rn 'sudo -u miru' agent/ --include='*.md' --include='*.sh'` returns only matches inside `plans/active/` and `plans/archived/` (historical records).

## Idempotence and Recovery

All steps are safe to repeat:

- The privilege-drop helper is a pure function call; calling it twice is fine (the second call sees `euid != 0` and either no-ops or errors — not destructive).
- Editing source files and re-running `cargo build` / `cargo test` is idempotent.
- Editing install scripts is idempotent — the new `sudo ...` form does not conflict with itself if the script is run twice on the same host.
- `./scripts/preflight.sh` is read-only with respect to source.

Risky step: M2 (wiring into `main`). If the privilege drop is misordered (e.g. placed before `--version` short-circuit or before logging init in a way that surprises us), the agent could fail to start at all on a `.deb`-installed device. Mitigation:

- Validation step 5 explicitly tests the already-miru no-op path.
- Rollback: revert the M2 commit (`git revert <sha>`) and rebuild. Source is otherwise unchanged.

If `getpwnam_r` fails on a `.deb`-installed device because the postinst's `useradd` didn't run (corrupted install): the agent will print `user 'miru' not found in /etc/passwd; is the miru .deb installed?` and exit 1, leaving `/var/lib/miru/` untouched. Recovery: reinstall the package (`sudo dpkg -i miru-agent_*.deb` or `sudo apt-get install --reinstall miru-agent`), which re-runs `postinst` and recreates the user.
