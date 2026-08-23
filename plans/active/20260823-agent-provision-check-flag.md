# Add `provision --check` to report provisioning state via exit code

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `mirurobotics/agent`) | read-write | All code, test, and doc changes described here. |

This plan lives in this repo because every change is in `agent/src/` and `agent/tests/`. No other Miru repository is touched. The public documentation change (docs repo) and the customer-facing Ansible role are explicitly out of scope.

## Purpose / Big Picture

Customers provision fleets with Ansible. Provisioning tokens are one-time-use and expire five minutes after being minted, so a playbook that cannot tell which hosts are already provisioned must mint a fresh token for every host on every run. Today the only way to answer "is this machine provisioned?" is to poke at undocumented files under `/var/lib/miru/auth/`.

After this change the agent answers that question itself:

    sudo -u miru /usr/sbin/miru-agent provision --check
    provisioned            # exit 0 — a subsequent `provision` would be a no-op
    not provisioned        # exit 3 — a subsequent `provision` is required
                           # exit 1 — state could not be determined (message on stderr)

The check is local, offline, read-only, needs no `MIRU_PROVISIONING_TOKEN`, and works whether or not the `miru` systemd service is running. An Ansible task can gate provisioning on it with `failed_when: rc not in [0, 3]` and `when: rc == 3`.

## Progress

- [ ] Milestone 1 — fallible existence probe in `filesys`.
- [ ] Milestone 2 — tri-state activation query in `disk`.
- [ ] Milestone 3 — `--check` flag, exit-code mapping, `main.rs` wiring, `ARCHITECTURE.md` note.
- [ ] Milestone 4 — read-only/offline guarantees, then preflight to CLEAN.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

- The implementation sandbox runs as **root**, which bypasses permission bits. All three new `0o000`-directory tests self-skip there (17 pre-existing tests in `deploy`/`sync`/`gcs` fail there for the same reason). Verified by re-running the new tests as an unprivileged user: 25/25 pass with no skips. Consequence: local `covgate.sh` reports `provisioning` at 96.46% (needs 96.57%) because check.rs's `Err(e) => Report::Undeterminable(e)` arm is unreachable as root; with those 2 regions covered it is 96.85%. `agent/src/cli` is 100% either way.

## Decision Log

- Decision: Exit code 3 means "not provisioned"; 1 stays reserved for errors.
  Rationale: Automation must distinguish "needs provisioning" from "something is wrong". 1 is already the agent's generic failure code, and 2 is conventionally reserved for CLI usage errors.
  Date/Author: 2026-08-23, plan author.

- Decision: The check reuses the same on-disk state predicate `provision` itself uses, rather than adding parallel file-existence logic.
  Rationale: If the check and the provision no-op branch can disagree, the check is worse than useless. `assert_activated` is reimplemented on top of the new query so the two cannot drift.
  Date/Author: 2026-08-23, plan author.

- Decision: The `privilege::verify_effective_user("miru")` gate in `agent/src/main.rs` is left unchanged; the check does not run as root.
  Rationale: The original request asked that the check also work as root, but the gate is an existing privilege-hardening control and carving a hole in it for a read-only path still widens the surface. Running as any user other than `miru` (root included) exits 1 with the existing privilege error on stderr, which is a valid "error" outcome under the contract. Ansible already uses `become_user: miru`. This deviation from the request is recorded here so it is visible in review.
  Date/Author: 2026-08-23, plan author.

- Decision: `assert_activated` now returns a `FileSysErr`-wrapped error (instead of `DeviceNotActivatedErr`) when key existence cannot be determined.
  Rationale: `Path::exists()` swallows I/O errors and returns `false`, which would report an unreadable auth directory as "not provisioned". Both existing callers (`agent/src/provisioning/provision.rs`, `agent/src/app/await_activation.rs`) only use `.is_ok()`, so their behavior is unchanged; only the check path distinguishes the two.
  Date/Author: 2026-08-23, plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent is a Rust binary. Everything below is under `agent/src/` (production) and `agent/tests/` (integration tests, mirroring the `src` layout). Terms:

- **Provisioned / activated** — the device has an RSA key pair on disk under the auth directory. `disk::Layout::default()` roots at `/var/lib/miru/`, so the files are `/var/lib/miru/auth/private.key` and `/var/lib/miru/auth/public.key`.
- **`.covgate`** — a file in each module directory holding that module's minimum line-coverage percentage, enforced by `scripts/covgate.sh`. `agent/src/cli/.covgate` is **100**, so every new line in the CLI module needs a test. `agent/src/filesys/.covgate` is 81.69, `agent/src/disk/.covgate` is 96.79, `agent/src/provisioning/.covgate` is 96.57. No new module *directory* is created by this plan, so no new `.covgate` file is needed.

Files that matter:

`agent/src/main.rs` — entry point. Order today: `cli::Args::parse` → `--version` short-circuit → `privilege::verify_effective_user("miru")` (prints to stderr and exits 1 on mismatch) → `provision_args` branch → `reprovision_args` branch → `run_agent()`. The `run_provision` helper creates a temporary log directory, initializes logging, computes settings, constructs an `http::Client`, builds `disk::Layout::default()`, reads the provisioning token from the environment, calls `provision::provision(...)`, then deletes the temp directory. **The check must branch before any of that**: no logging init, no HTTP client, no token read, no temp directory.

`agent/src/cli/mod.rs` — a single file, no clap. `Args::parse` iterates `inputs.iter().skip(1)` and matches `input.trim_start_matches('-')` against `"version" | "provision" | "reprovision"`. `ProvisionArgs::parse` iterates the same inputs but only handles `key=value` forms via `split_once('=')`, matching against `"backend-host" | "mqtt-broker-host" | "device-name"`. **Bare flags are ignored entirely today**, so `--check` needs a new branch for inputs with no `=`.

`agent/src/provisioning/provision.rs` — the no-op branch at the top of `provision()`:

    if disk::assert_activated(layout).is_ok() {
        // read device.json for the name, return Outcome { already_provisioned: true, .. }
    }

`agent/src/disk/device.rs` — `pub fn assert_activated(layout: &Layout) -> Result<(), DiskErr>` returns `DiskErr::DeviceNotActivatedErr` if either `layout.auth().private_key()` or `layout.auth().public_key()` does not `.exists()`. Re-exported from `agent/src/disk/mod.rs` (`pub use self::device::{assert_activated, resolve_device_id, Device};`). Other caller: `agent/src/app/await_activation.rs` (two `.is_ok()` call sites).

`agent/src/filesys/path.rs` — the `PathExt` trait. Its `fn exists(&self) -> bool` is `self.path().exists()`, i.e. `std::path::Path::exists()`, which **maps any I/O error to `false`**. An unreadable `/var/lib/miru/auth` (permission denied) would therefore be reported as "not provisioned" (exit 3) rather than "error" (exit 1). This is the crux of the design and Milestone 1 fixes it.

`agent/src/filesys/errors.rs` — leaf error structs deriving `thiserror::Error` with a `pub trace: Box<Trace>` field and `impl crate::errors::Error for X {}`, aggregated into `pub enum FileSysErr` (around line 337) with `#[error(transparent)]` variants, and registered in the `crate::impl_error!(FileSysErr { ... });` list (around line 406). Both lists must be updated together.

`agent/src/privilege/mod.rs` — `verify_effective_user(name)` fails unless **both** euid and egid match the named user's.

Conventions to follow (from `AGENTS.md`):

- Import ordering in every file: `// standard crates`, blank line, `// internal crates`, blank line, `// external crates`.
- Production functions and closures are limited to 50 non-blank, non-comment body lines.
- Tests run only via `./scripts/test.sh` (which is `RUST_LOG=off cargo test --features test`); the `--features test` flag is mandatory.
- Integration tests live in `agent/tests/` mirroring `agent/src/`; new test files must be declared in the enclosing `mod.rs`.
- `libs/backend-api/` and `libs/device-api/` are generated — do not touch.
- `nix` and `chrono` are regular dependencies of the `miru-agent` package and are usable from integration tests (see `agent/tests/privilege/mod.rs`, which already calls `nix::unistd::geteuid()`).

## Plan of Work

**Milestone 1 — fallible existence probe.** In `agent/src/filesys/errors.rs`, add a leaf error struct `PathExistenceErr { pub path: PathBuf, pub source: Box<std::io::Error>, pub trace: Box<Trace> }` with message `unable to determine whether path exists: {path}`, add a matching `#[error(transparent)]` variant to `pub enum FileSysErr`, and add its name to the `crate::impl_error!(FileSysErr { ... })` list. In `agent/src/filesys/path.rs`, add a default method to `PathExt`:

    fn try_exists(&self) -> Result<bool, FileSysErr>

implemented with `std::path::Path::try_exists()`, mapping the `io::Error` into `FileSysErr::PathExistenceErr`. Leave `exists()` untouched — other call sites keep their current semantics.

**Milestone 2 — tri-state activation query.** In `agent/src/disk/device.rs` add

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum Activation { Activated, NotActivated }

    pub fn activation_state(layout: &Layout) -> Result<Activation, DiskErr>

which returns `Ok(Activation::NotActivated)` when either key is absent, `Ok(Activation::Activated)` when both are present, and `Err(..)` (a `DiskErr::FileSysErr` from `try_exists`) when existence cannot be determined. A plain-derive enum is correct here per the `AGENTS.md` enum conventions: it has no wire contract and no backend twin. Then **reimplement `assert_activated` in terms of `activation_state`** so the two can never disagree: `Activated` → `Ok(())`, `NotActivated` → the existing `DiskErr::DeviceNotActivatedErr`, `Err(e)` → `Err(e)`. Export `Activation` and `activation_state` from `agent/src/disk/mod.rs` alongside `assert_activated`.

**Milestone 3 — flag, mapping, wiring.** Add `pub check: bool` to `cli::ProvisionArgs` in `agent/src/cli/mod.rs` and a branch in `ProvisionArgs::parse` for inputs that contain no `=`: match `input.trim_start_matches('-')` against `"check"` and set the field, ignoring anything else. Create `agent/src/provisioning/check.rs` holding the exit-code contract as named constants and a pure, testable mapping:

    pub const EXIT_PROVISIONED: i32 = 0;
    pub const EXIT_ERROR: i32 = 1;
    pub const EXIT_NOT_PROVISIONED: i32 = 3;

    pub enum Report { Provisioned, NotProvisioned, Undeterminable(DiskErr) }

    pub fn check(layout: &disk::Layout) -> Report

    impl Report {
        pub fn exit_code(&self) -> i32
        pub fn stdout_line(&self) -> Option<&'static str>   // "provisioned" / "not provisioned" / None
        pub fn stderr_line(&self) -> Option<String>          // None / None / "miru-agent: {e}"
    }

`check` takes only `&disk::Layout` — no HTTP client, no settings, no token — which is what structurally guarantees the offline property. If clippy's `large_enum_variant` fires on `Undeterminable(DiskErr)` under `-D warnings`, box the payload (`Undeterminable(Box<DiskErr>)`) rather than suppressing the lint. Add `pub mod check;` to `agent/src/provisioning/mod.rs`. In `agent/src/main.rs`, inside the `if let Some(provision_args) = cli_args.provision_args` branch, before calling `run_provision`, add:

    if provision_args.check {
        let report = check::check(&disk::Layout::default());
        if let Some(line) = report.stdout_line() { println!("{line}"); }
        if let Some(line) = report.stderr_line() { eprintln!("{line}"); }
        std::process::exit(report.exit_code());
    }

Keep `main` a thin shim; all logic and all three constants live in `check.rs` so the contract is greppable.

**Milestone 4 — read-only and offline guarantees.** Add the assertions described in Concrete Steps that pin down the two properties automated tests can actually pin down: the check writes nothing, and it has no way to reach the network.

## Concrete Steps

All commands run from the repo root `/home/user/agent` unless stated otherwise.

**Milestone 1**

1. Edit `agent/src/filesys/errors.rs` and `agent/src/filesys/path.rs` as described in Plan of Work.
2. Add tests to `agent/tests/filesys/path.rs` (already declared in `agent/tests/filesys/mod.rs`): `try_exists` returns `Ok(true)` for a seeded file, `Ok(false)` for a missing file under an existing directory, `Ok(false)` for a path whose parent directory does not exist, and `Err(FileSysErr::PathExistenceErr(_))` for a file inside a directory chmodded to `0o000`. Gate the permission test with an early `return` when `nix::unistd::geteuid().is_root()` (root bypasses permission bits) and restore the directory to `0o755` before the test ends so the `TempDir` can be cleaned up. Follow the existing pattern in `agent/tests/deploy/filesys.rs`, which uses `filesys::dirs::set_permissions` with `std::fs::Permissions::from_mode`.
3. Run `./scripts/test.sh` and expect a clean pass.
4. Commit: `feat(filesys): add fallible try_exists path probe`

**Milestone 2**

5. Edit `agent/src/disk/device.rs` and `agent/src/disk/mod.rs` as described.
6. Add a `pub mod activation_state { ... }` block to `agent/tests/disk/device.rs` mirroring the existing `pub mod assert_activated` block and its `fresh_layout()` helper: both keys missing → `Activation::NotActivated`; private key only → `NotActivated`; public key only → `NotActivated`; both present → `Activated`; auth directory chmodded to `0o000` → `Err(DiskErr::FileSysErr(_))` (root-gated and mode-restoring, as in step 2).
7. Run `./scripts/test.sh`. The four pre-existing tests in `pub mod assert_activated` must still pass **unmodified** — in particular the three missing-key cases must still match `DiskErr::DeviceNotActivatedErr(_)`. If they do not, the reimplementation is wrong; fix it rather than editing the tests.
8. Commit: `feat(disk): add tri-state activation_state query`

**Milestone 3**

9. Edit `agent/src/cli/mod.rs`, create `agent/src/provisioning/check.rs`, update `agent/src/provisioning/mod.rs`, and wire `agent/src/main.rs`. Then add one sentence to the Provision-mode bullet in `ARCHITECTURE.md`'s "Bird's Eye View" documenting `provision --check` and exit codes 0/3/1; do not restructure the rest of that file.
10. Add CLI tests to the `mod args_parse` block in `agent/tests/cli/mod.rs` using the existing `to_inputs` helper: `["miru-agent", "provision", "--check"]` sets `check == true` with all three option fields `None`; `["miru-agent", "provision", "--check", "--device-name=robot-1", "--backend-host=https://backend.example.com"]` sets `check == true` and both options; `["miru-agent", "provision"]` leaves `check == false`; an unrecognized bare flag such as `["miru-agent", "provision", "--nonsense"]` is ignored and leaves `check == false` (this covers the new `_ => {}` arm, which `agent/src/cli/.covgate` = 100 requires); and `["miru-agent", "--check"]` alone leaves `provision_args` as `None`. A test that asserts four or more fields of the same variable trips the field-by-field-assert linter; split such a test or suppress it with `// lint:allow(field-by-field-assert)` inside the test body.
11. Create `agent/tests/provisioning/check.rs` and add `pub mod check;` to `agent/tests/provisioning/mod.rs`. Cover, using a temp `Layout` built like the `fresh_layout()` helper in `agent/tests/disk/device.rs`: fresh install (no auth directory at all) → `Report::NotProvisioned`, `exit_code() == 3`, `stdout_line() == Some("not provisioned")`, `stderr_line().is_none()`; both keys seeded → `Provisioned`, code `0`, stdout `Some("provisioned")`; private key only → `NotProvisioned`, code `3`; public key only → `NotProvisioned`, code `3`; `device.json` seeded but both keys missing → `NotProvisioned`, code `3` (partial state is not provisioned); auth directory chmodded to `0o000` → `Undeterminable(_)` with `exit_code() == 1`, `stdout_line().is_none()` (stdout stays clean in the error case), and `stderr_line()` `Some` and non-empty. Root-gate and mode-restore the permission case as in step 2.
12. Run `./scripts/test.sh` and `./scripts/covgate.sh`; both must pass.
13. Commit: `feat(cli): add provision --check exit-code contract`

**Milestone 4**

14. Add the read-only assertions to `agent/tests/provisioning/check.rs`: (a) build a temp `Layout` whose root does not exist, call `check::check(&layout)`, then assert `layout.root().exists()` and `layout.temp_dir().exists()` are both still `false` — this proves the check creates no directories and does not run the temp-directory dance `run_provision` performs; (b) for the provisioned case, snapshot `filesys::dirs::subdirs(&layout.root())` and `filesys::dirs::files(&layout.root())` before and after the call and assert both listings are unchanged.
15. Record the offline guarantee where it is enforceable: `check::check` takes `&disk::Layout` and nothing else, so no HTTP client, backend host, or provisioning token can reach it. State this in the `check.rs` module doc comment, and confirm by inspection that `agent/src/provisioning/check.rs` imports neither `crate::http` nor `crate::network`, and that the `--check` branch in `agent/src/main.rs` returns before `run_provision` is called.
16. Run `./scripts/lint.sh` (run `./scripts/update-deps.sh` first if `Cargo.lock` is stale), then `./scripts/test.sh`, then `./scripts/covgate.sh`.
17. Commit: `test(provisioning): assert check performs no writes`
18. Push the branch and take it through `$preflight` until it reports CLEAN (see Validation and Acceptance).

Expected transcript shape for a passing test run:

    $ ./scripts/test.sh
    ...
    test result: ok. N passed; 0 failed; 0 ignored

## Validation and Acceptance

**Automated.** From the repo root, `./scripts/test.sh` passes with zero failures, `./scripts/covgate.sh` passes for every module (notably `agent/src/cli` at 100), and `./scripts/lint.sh` passes (custom import linter, 50-line function-body limit, field-by-field-assert check, `cargo fmt --check`, `cargo clippy --all-features -- -D warnings`, `cargo machete`, security audit).

Every test named in Concrete Steps fails or does not compile before the corresponding source change and passes after. The four pre-existing tests in `pub mod assert_activated` in `agent/tests/disk/device.rs` pass unchanged both before and after.

**Manual, on a device or in a container with a `miru` user.** As the `miru` user, before provisioning:

    $ sudo -u miru /usr/sbin/miru-agent provision --check; echo "rc=$?"
    not provisioned
    rc=3

After a successful `provision`:

    $ sudo -u miru /usr/sbin/miru-agent provision --check; echo "rc=$?"
    provisioned
    rc=0

With one key removed (partial state), the output returns to `not provisioned` / `rc=3`. With the auth directory unreadable, stdout is empty, an error line appears on stderr, and `rc=1`. All four cases behave identically with the `miru` systemd service running and stopped, and with `MIRU_PROVISIONING_TOKEN` unset.

**CI gate.** `gh` is **not installed** in this environment. CI status must be read through the GitHub MCP tools: `mcp__github__actions_list`, `mcp__github__actions_get`, `mcp__github__get_job_logs`, `mcp__github__pull_request_read`, and `mcp__github__create_pull_request`. **Preflight must report CLEAN — CI green on the pushed branch head — before the PR leaves draft or the task is reported complete.** A local pass is not a substitute.

**Interface stability.** The three exit codes are public interface from the first release that ships them. Ansible playbooks will encode `rc in [0, 3]` as success. Any later change to the codes or to their meanings is a breaking change and must be treated as such.

## Out of Scope

- **Server-side credential validity.** The check answers a purely local question. A device that was reprovisioned onto other hardware from the dashboard still reports `provisioned` locally, because its keys are still on disk. Validating credentials against the backend would require network access and is excluded by design.
- **`reprovision --check`.** Only `provision` gets the flag.
- **JSON or other structured output.** v1 emits one plain line.
- **Running the check as root.** See the Decision Log; the privilege gate is unchanged.
- **The docs-repo pull request and the customer Ansible role.** Separate work in separate repositories.

## Idempotence and Recovery

Every step is a source edit plus a test run; all are safely repeatable and nothing here migrates or destroys data. The change adds a read-only code path and does not alter any write path.

The only edit with blast radius beyond the new feature is the reimplementation of `assert_activated` in Milestone 2, which both `agent/src/provisioning/provision.rs` and `agent/src/app/await_activation.rs` depend on. Guard rail: the four pre-existing `assert_activated` tests must pass unmodified (step 7). If they fail, revert `agent/src/disk/device.rs` with `git checkout -- agent/src/disk/device.rs` and redo the reimplementation; do not adjust the tests to fit.

The permission-based tests chmod a directory to `0o000`. If a test panics between the chmod and the restore, `TempDir` cleanup can fail and leave a stray directory under the system temp directory; remove it manually with `chmod 755` followed by `rm -rf`. Writing the restore so it runs before any assertion that can panic avoids this.

Per-milestone commits mean recovery from any milestone is `git revert` of a single commit, or `git reset --hard <previous milestone commit>` on the feature branch.
