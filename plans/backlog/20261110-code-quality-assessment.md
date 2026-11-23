# Code quality assessment of the agent repo

An audit of `mirurobotics/agent` at `1bf5955` (2026-11-10): where the repo's code, CI and docs fall short, and what to do about it, in priority order. It changes no code. Each finding is meant to become its own ExecPlan in `plans/active/` when picked up.

## How to read this

Each finding gives what is wrong, why it matters, and a suggested fix, with a rough effort (S: under a day, M: a few days, L: a week or more).

- **P1: fix first.** The agent can misbehave on a device, or what CI and the docs say is not what the code does.
- **P2: fix soon.** Structure and gaps that make every change slower or riskier.
- **P3: tidy up.** Lint, docs and hygiene gaps that are cheap to close.

| # | Finding | Priority | Effort |
|---|---------|----------|--------|
| 1 | Atomic writes block the async runtime | P1 | S |
| 2 | `build.rs` reruns on every build | P1 | S |
| 3 | Windows-only code is never linted or coverage-gated | P1 | S |
| 4 | The docs agents read first are out of date | P1 | S |
| 5 | The CLI silently ignores unknown arguments | P2 | S |
| 6 | `main.rs` holds untested startup logic | P2 | M |
| 7 | Two `RetryPolicy` types, and backoff without jitter | P2 | M |
| 8 | Adding a worker means editing `ShutdownManager` in five places | P2 | M |
| 9 | Coverage gates can pass without measuring anything | P2 | S |
| 10 | Three TLS and crypto stacks, and 31 duplicated crates | P2 | M |
| 11 | The error trait mixes HTTP into every module, with heavy boilerplate | P2 | L |
| 12 | Toolchain, MSRV and workspace manifest drift | P3 | S |
| 13 | Lint gaps and blanket suppressions | P3 | S |
| 14 | Little rustdoc, and no `cargo doc` check | P3 | M |
| 15 | Legacy install scripts are unchecked | P3 | S |
| 16 | CI hygiene | P3 | S |
| 17 | Plans and tech debt tracking are stale | P3 | S |
| 18 | Release profile is not tuned for a device binary | P3 | S |

## What is already good

These are worth keeping as the repo changes:

- Production code is careful about panics. Outside tests there are fewer than ten `unwrap()`/`expect()` calls in roughly 26k lines, all on constants or at startup.
- Testing is deep: about 2,270 tests, with more test code (about 59k lines) than production code. Time is controlled with paused clocks and sleep controllers rather than real sleeps, and tests that share OS resources are `#[serial]`.
- Every module has a coverage gate (`.covgate`), and the gates sit between 81% and 100%.
- A custom linter enforces import order, a 50-line function limit (4 suppressions in the whole crate) and field-by-field assertions.
- Every third-party action is pinned to a commit SHA, Dependabot covers Cargo and Actions, and CI reaches AWS and GCP through OIDC.
- `Cargo.toml` explains *why* each security-driven dependency choice was made, with the advisory IDs.
- The actor pattern (`*_impl` modules behind a handle) is used consistently, and the old event-store mutex problems are gone because the event hub is now an actor.
- Design decisions are written down as ExecPlans, and `AGENTS.md` captures review rules with links to the PR discussions they came from.

---

## P1: fix first

### 1. Atomic writes block the async runtime

**What.** `filesys::files::write_bytes` is `async`, but when `opts.atomic == Atomic::Yes` (the default since the atomic-writes plan) it calls `write_bytes_atomic` directly (`agent/src/filesys/files.rs:236`). That function uses the `atomicwrites` crate, which writes a temp file, calls `fsync`, renames, and syncs the directory, all as blocking syscalls on a Tokio worker thread. Every state file goes through this path: queue snapshots, caches, settings, the token file, the calibrations index.

`files::sha256` is a related, smaller case. The scanner and the retention deleter hash candidate files on the runtime (`data_uploads/scan/rule.rs:278`, `retention/deleter_impl.rs:176`), and data-upload files can be large.

**Why it matters.** Robots often store data on SD cards or eMMC, where one `fsync` can take tens to hundreds of milliseconds. While it runs, that worker thread can't serve the device API, the MQTT loop or other actors. This is the same kind of problem TD-2 described for the event store, and the default write path has it now.

**Fix.** Run `write_bytes_atomic` inside `tokio::task::spawn_blocking` (copy the buffer, or take ownership of it). Do the same for hashing files above a size threshold, such as 8 MiB. Add a test that a write doesn't stall a concurrent task on a current-thread runtime.

### 2. `build.rs` reruns on every build

**What.** `agent/build.rs` prints `cargo:rerun-if-changed=.git/HEAD` and `.git/refs/`. Cargo resolves those paths against the package directory (`agent/`), where there is no `.git`. Cargo treats a missing path as changed, so the build script runs on every `cargo` command. This was checked with a small crate on cargo 1.97: the second build reports "Dirty ... the file `.git/HEAD` is missing". Each run also stamps a new `MIRU_AGENT_BUILD_DATE`, so `miru-agent` and every test target recompile even when nothing changed.

The script also shells out to `date` and `git` and calls `expect` on both. A build without `git` on the PATH or outside a checkout (a source tarball, some CI containers) panics. In a git worktree, `.git` is a file, not a directory.

**Why it matters.** Every local `cargo build`, `cargo test` and `cargo clippy` pays for a recompile of the largest crate, and `scripts/lint.sh` pays it twice, since it runs clippy twice by default. The binary's build date also changes on every build, so builds are never reproducible.

**Fix.** Find the git directory with `git rev-parse --git-dir` and emit absolute paths for `HEAD`, the ref it points to, and `packed-refs`. Fall back to `"unknown"` when `git` is missing instead of panicking. Take the build date from `SOURCE_DATE_EPOCH` when set, or drop it from debug builds. Use `chrono` (already a dependency) instead of the `date` binary, which doesn't exist on a stock Windows machine.

### 3. Windows-only code is never linted or coverage-gated

**What.** Clippy runs only in the Linux `lint` job (`scripts/lib/lint.sh`). The `windows-check` job runs `cargo test` on Windows but not clippy, so `#[cfg(windows)]` code is compiled but never linted: `windows/scm.rs` (456 lines), the Windows halves of `platform`, `filesys` and `server`, and the service entry in `main.rs`. Coverage runs only on Linux too, so `windows/.covgate` (90%) measures only `windows/errors.rs`, the part that compiles on Linux.

**Why it matters.** Windows support is new and shipping (`plans/active/20260910-windows-support.md`). The code most likely to have platform bugs is the code with the fewest checks, and `-D warnings` doesn't protect it.

**Fix.** Add `cargo clippy --package miru-agent --all-targets --locked -- -D warnings` to the `windows-check` job. It runs on Windows already, so this adds only a few minutes. Then either run `cargo llvm-cov` there with a Windows-only gate for `windows/`, or set `windows/.covgate` to `0` with a comment saying the Windows suite covers it. Today's gate claims coverage it doesn't measure.

### 4. The docs agents read first are out of date

**What.** `AGENTS.md` tells agents to read `ARCHITECTURE.md` first, but both have drifted from the code:

- `AGENTS.md` says `lib.rs` lists "all 23 public modules". There are 32.
- The `ARCHITECTURE.md` codemap has no entry for `app`, `events`, `network`, `privilege` or `s3`. It calls the provisioning module `provision` with helpers in `provision/display`, but the module is `provisioning`.
- It says `telemetry` is "OpenTelemetry integration". It is a `sysinfo` wrapper, and there is no OpenTelemetry dependency.
- It says `workers/` holds "three long-running tasks". There are six (`mqtt`, `poller`, `token_refresh`, `scan`, `sync_scan_bridge`, `delete`).
- `README.md` says releases build with GoReleaser for Linux only. `release.yml` has no GoReleaser step, and releases also build a Windows MSI.
- `TECH_DEBT.md` lists TD-1 to TD-4 as open, but they are fixed. `agent/tests/events/` exists, the event store is async, the hub is an actor with no mutex, and `sse.rs` uses `e.code()`. TD-5 points to `agent/.ai/exec-plans/...`, which doesn't exist. Only TD-6 (the log file is opened on every append) is still true.

**Why it matters.** Most changes here are written by agents that trust these files. A wrong codemap leads to code in the wrong layer, and a stale debt list sends work at problems that are already solved.

**Fix.** Correct the items above. Move TD-6 into `plans/backlog/` and delete `TECH_DEBT.md`, so there is one place for debt (see #17). Add a short check to the lint job that every directory under `agent/src/` is named in the `ARCHITECTURE.md` codemap. It is about ten lines of shell and would catch the next new module.

---

## P2: fix soon

### 5. The CLI silently ignores unknown arguments

**What.** `cli::Args::parse` (`agent/src/cli/mod.rs:12`) matches known flags and drops everything else (`_ => {}`). `ProvisionArgs::parse` does the same for `key=value` pairs. A typo such as `miru-agent --provison` or `--device_name=foo` doesn't fail. With no recognised subcommand, `main` starts the full runtime agent. The legacy install scripts still call `miru-agent --install`, a flag the current CLI doesn't know.

**Why it matters.** A mistyped provisioning command starts a long-running agent under whatever user ran it, with no error. A mistyped option is dropped and the default is used. Both are hard to debug on a remote device.

**Fix.** Reject unknown flags and stray positional arguments with a usage message and a non-zero exit. Parse subcommand options only after their subcommand. Either keep the hand-written parser with a strict `_ => error` arm and tests, or move to `clap` (derive). Both are small; `clap` also gives `--help` for free.

### 6. `main.rs` holds untested startup logic

**What.** `agent/src/main.rs` is 374 lines in the binary target: provisioning and reprovisioning result handling, `reconcile_agent_version`, `read_settings`, `build_app_options`, bootstrap host selection, signal handling and Windows service dispatch. Integration tests can't reach a binary, and the coverage gates only cover directories under `agent/src/`, so none of this is tested or gated.

**Why it matters.** This is where settings become runtime options and where upgrades are reconciled, so a mistake here affects every device on startup. `build_app_options` is the kind of mapping that quietly drops a new setting.

**Fix.** Move everything except `fn main` and the Windows service macro into the library, for example an `app::entry` module, and test it there. Keep `main.rs` under about 50 lines.

### 7. Two `RetryPolicy` types, and backoff without jitter

**What.** `deploy::fsm::RetryPolicy` (`max_attempts` of 2^31-1, 15 s doubling to 24 h) and `queue::RetryPolicy` (`attempts`, `max_age`, network errors not counted) share a name but have different fields and meanings. `AGENTS.md` asks for one name per concept. Also, `cooldown::Backoff` has no jitter. Only `http::retry` adds any, and it does so with `SystemTime` nanoseconds.

**Why it matters.** The name clash makes it easy to import the wrong type and confuses the docs. Without jitter, a fleet that loses the control plane at the same moment retries in lockstep: every device waits 10 s, 20 s, 40 s and so on together, and the burst arrives just as the control plane recovers.

**Fix.** Rename the deploy one (for example `DeployRetry`) or fold it into `queue::RetryPolicy` if the semantics can be unified. Add jitter to `cooldown::calc`, either full or ±20%, behind a source of randomness that tests can control. Have `http::retry` use it too.

### 8. Adding a worker means editing `ShutdownManager` in five places

**What.** `app/run.rs` is the largest production file (761 lines before its tests). `ShutdownManager` (`run.rs:486`) has one named `Option<JoinHandle>` field per worker. A new worker needs a field, a `None` in `new`, a `register_handle` call, a numbered step in `shutdown_impl`, and a test, and the shutdown order lives only in that function's body and its comments.

**Why it matters.** The order is an architectural invariant (`ARCHITECTURE.md`: "Shutdown ordering matters"). Today it is enforced by hand-numbered comments, and the file mixes startup wiring with the shutdown machinery.

**Fix.** Replace the per-worker fields with an ordered list of named handles, each tagged with a shutdown phase (workers, servers, drivers, state). `shutdown_impl` then joins phase by phase, and adding a worker is one `register` call. Move `ShutdownManager` into `app/shutdown.rs`.

### 9. Coverage gates can pass without measuring anything

**What.** In `scripts/lib/covgate.sh`, a `.covgate` whose directory has no files in the coverage report prints a warning and passes (`no_files`). A module that is renamed, moved, or compiled out on Linux (see #3) keeps a green gate. Thresholds have two decimals (for example `cache` at 92.78), so a refactor that moves coverage by 0.01% fails. The gate measures regions only.

**Why it matters.** A gate that silently passes is worse than no gate, because it looks like coverage. Thresholds that tight push people to run `update-covgates.sh` rather than look at what dropped.

**Fix.** Make `no_files` a failure unless the threshold is `0`. Round thresholds down to whole numbers and allow a 0.5-point tolerance, or keep the ratchet but print the diff against `main`. Consider gating line coverage as well as regions, since that is what reviewers read in the HTML report.

### 10. Three TLS and crypto stacks, and 31 duplicated crates

**What.** `Cargo.lock` has `ring`, `aws-lc-rs`/`aws-lc-sys` and vendored OpenSSL (`openssl-src` 3.6). The HTTP and S3 clients use rustls on aws-lc, and MQTT uses native-tls on OpenSSL (on Linux) to avoid a rustls-webpki advisory in `rumqttc`. On Windows, native-tls means SChannel, so MQTT TLS behaves differently per OS. 31 crates appear in two versions, including `axum`, `hyper`, `http`, `rand`, `thiserror`, `sha2` and `syn`.

**Why it matters.** Three C crypto builds dominate clean build time and binary size, and each is its own patch stream. Two MQTT TLS backends double what has to be tested.

**Fix.** Track the `rumqttc`/`rumqttd` release that moves to rustls-webpki 0.103 and switch MQTT back to rustls on aws-lc, which drops OpenSSL and native-tls. Add `cargo-deny` with a `bans` section (warn on duplicates, deny specific heavy ones), plus licence and source checks. Its advisory check can replace `cargo audit`.

### 11. The error trait mixes HTTP into every module, with heavy boilerplate

**What.** `crate::errors::Error` has `http_status()` returning `axum::http::StatusCode`, so every module's errors (`filesys`, `s3`, `crypt`, `queue`) know about HTTP. There are about 120 hand-written leaf error structs across 3,600 lines of `errors.rs`, each with a boxed `trace` that captures a `Backtrace`.

**Why it matters.** Each new failure costs a struct, an enum variant, an `impl_error!` entry and a trait impl. The HTTP mapping is spread over every module rather than kept in `server/`, which `AGENTS.md` already treats as the place for wire conversions.

**Fix.** This is large, so do it module by module when a module is touched anyway. Move the `code()`/`http_status()` mapping into `server/response.rs` for the errors the device API actually returns, and let other modules default. Consider a small derive or a declarative macro for leaf errors (`#[error(...)]` plus the trace field) to cut the boilerplate. Decide first whether the per-error `Backtrace` is used in practice; if not, drop it.

---

## P3: tidy up

### 12. Toolchain, MSRV and workspace manifest drift

**What.**

- `Cargo.toml` declares `rust-version = "1.93.0"`, but `rust-toolchain.toml` pins 1.97.0 and nothing checks 1.93. The MSRV is probably already wrong.
- The Linux CI jobs install `stable` with `dtolnay/rust-toolchain`, and then rustup switches to the pinned 1.97 on first use. The `stable` install is unused, as the Windows job's comment already says.
- `[workspace.dependencies]` has `config-agent = { path = "apps/agent" }`, and `apps/` doesn't exist.
- `[workspace.metadata] git_commit = "arglebargle"` is a placeholder nothing reads.
- `.gitmodules` is empty.

**Fix.** Set `rust-version` to the pinned toolchain (the agent ships as a binary, so a lower MSRV buys nothing), or add an MSRV job. Use `rustup show` instead of `dtolnay/rust-toolchain` on Linux. Delete the dead manifest entries and `.gitmodules`.

### 13. Lint gaps and blanket suppressions

**What.**

- Seven files have `#[allow(unused_imports)] use tracing::{debug, error, info, trace, warn};`, which hides unused imports on that line forever.
- There is no workspace `[lints]` table. The crate allows `new_without_default` and nothing stricter.
- Linux lint and test jobs don't pass `--locked`, while the Windows jobs do.

**Fix.** Import only the macros each file uses, and remove the `allow`s. Add a `[workspace.lints]` table with `clippy::unwrap_used`, `clippy::expect_used` and `clippy::panic` set to `warn` (allowed in tests via `clippy.toml` `allow-unwrap-in-tests`). With fewer than ten call sites today, this locks in the current discipline cheaply. Add `--locked` to every cargo call in CI.

### 14. Little rustdoc, and no `cargo doc` check

**What.** 14 of 214 source files have a `//!` module doc. Behaviour is described in `ARCHITECTURE.md` instead. The calibrations entry is one paragraph of about 1,000 words that restates crash ordering, retry timing and on-disk format. Nothing runs `cargo doc`, so broken intra-doc links aren't caught.

**Why it matters.** Prose far from the code drifts (see #4). Detail like "the queue entry is written before the report file" belongs next to the code that does it.

**Fix.** Keep `ARCHITECTURE.md` to a short paragraph per module and move the detail into `//!` docs in each module's `mod.rs`. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p miru-agent` to the lint job.

### 15. Legacy install scripts are unchecked

**What.** The six scripts in `scripts/install/` are generated from `scripts/jinja/` and refuse v0.9.0+ (`plans/completed/20260429-install-script-deprecate-v0.9.0.md`). CI doesn't check that they match their templates. Re-rendering them today gives no diff apart from the timestamp, so they are in sync for now. Shellcheck runs only on `build/debian/`. On these scripts it flags unquoted `$args` in the final `miru-agent --install $args` call, so a device name with a space is split into separate arguments.

**Fix.** Decide when v0.6 to v0.8 installs stop being supported, then delete the scripts and the Jinja pipeline. Until then, drop the build timestamp from the header and add a CI step that re-renders and runs `git diff --exit-code`, plus shellcheck on `scripts/` and `build/`.

### 16. CI hygiene

**What.**

- CI runs on pull requests, `release/*` pushes and nightly, but not on pushes to `main`. A merge that breaks `main` is found up to a day later, unless branch protection requires branches to be up to date.
- The `lint` and `tools` jobs reinstall `cargo-machete`, `cargo-audit` and `cargo-diet` on every run.
- Linux jobs have no `timeout-minutes` on `lint`.

**Fix.** Add `push: branches: [main]`, or confirm in `mirurobotics/infra` that "require up to date" is on. Use `taiki-e/install-action` for the cargo tools (prebuilt, cached), as the job already does for `cargo-llvm-cov`. Add a timeout to `lint`.

### 17. Plans and tech debt tracking are stale

**What.**

- Five of the nine plans in `plans/active/` have every progress item checked (`revendor-backend-spec-v05-beta2`, `platform-paths`, `portable-path-fixtures`, `windows-cfg-gates`, `windows-test-suite`).
- `20260812-pr3a-retention-delete-worker.md` has four unchecked items, but `workers/delete.rs` and `retention/deleter.rs` exist.
- Three completed plans have no date prefix.
- 283 lines across the plans name a contributor's local path (`/home/ben/...`).
- `TECH_DEBT.md` is a second tracker beside `plans/`.

**Fix.** Move finished plans to `plans/completed/`, rename the three undated ones, and replace local paths with repo-relative ones in the plan template. Keep debt in `plans/backlog/` (this file is the first one there) and retire `TECH_DEBT.md` (#4).

### 18. Release profile is not tuned for a device binary

**What.** `[profile.release]` sets only `opt-level = 3` and `debug = false`. There is no `lto`, `codegen-units = 1` or `strip`.

**Why it matters.** The agent ships to robots, often ARM, over constrained links. Fat LTO and stripping usually cut a Rust binary's size noticeably and can speed it up a little.

**Fix.** Measure first. Build release with `lto = "thin"` or `"fat"`, `codegen-units = 1` and `strip = "symbols"`, and compare size and build time. Keep separate debug symbols if crash symbolication needs them; the Windows release already ships a PDB.

---

## Suggested order

1. **One small PR for quick wins:** #2, #3, #12, #13, and the doc fixes in #4.
2. **Runtime correctness:** #1, then #5 and #7.
3. **Startup and shutdown structure:** #6 and #8 together, since both touch `main.rs` and `app/run.rs`.
4. **Gates that mean what they say:** #9 and #14.
5. **Dependencies:** #10, when `rumqttc` allows it.
6. **Hygiene, any time:** #15, #16, #17, #18.
7. **Opportunistically:** #11, one module at a time.

## How this was checked

The repo was read at `1bf5955`, with a focus on `agent/src/`, CI, scripts and docs. Counts come from `grep` and `wc` over the tree; production line counts exclude inline `#[cfg(test)]` modules. The `build.rs` behaviour in #2 was reproduced with a small crate on cargo 1.97. The install scripts were re-rendered and shellchecked. `cargo build`, tests, clippy and coverage were not run, because the repo asks agents to leave those to CI.
