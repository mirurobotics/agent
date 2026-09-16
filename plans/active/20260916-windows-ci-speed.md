# Cut the wall-clock time of the `windows-check` CI job


This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective up to date during implementation.

## Scope


| Repository | Access | Work |
| --- | --- | --- |
| `/home/ben/miru/workbench4/repos/agent` | Read-write | `.github/workflows/ci.yml` (the `windows-check` job only) and this plan. |
| `/home/ben/miru/workbench4` | Read-only | Workspace instructions and shared skill policies. Independent Git checkout, not a submodule. |

This plan lives in the agent repository because it owns the workflow. Work happens on branch `perf/windows-ci-speed` (based on `main` at `4b3ac66`). No production code, no test code, no `.cargo/config.toml` and no `Cargo.toml` change is expected; every lever is expressed as job-scoped YAML so the Linux jobs (`lint`, `test`, `tools`) are untouched. Do not modify `libs/`, `.agents/`, or coverage thresholds.

## Purpose / Big Picture


The `windows-check` job runs the full agent test suite natively on `windows-latest` and is the slowest job in CI at about 4m50s, so it sets the wall-clock time of every PR. After this plan, a PR run of `windows-check` finishes materially faster while still executing exactly the same tests (448 library tests, 1538 tests in the `mod` integration binary, one test in each `logs_init_*` binary), the job id stays `windows-check` (branch protection), and every change is proven by the job's own timings recorded in this plan.

Baseline (run 35129974134, job 104908267641, `rustc 1.98.1`, rust-cache full match, ~461 MB cache):

- Whole job: 4m49s.
- "Run Windows Tests" step: about 3m50s, made of a compile phase of 1m36s (`Finished test profile ... in 1m 36s`; only `miru-agent`, `libs/backend-api`, `libs/device-api` compile because rust-cache does not cache workspace crates) and a test phase where the library unit tests take 12.1s and `tests/mod.rs` takes 108.5s (Linux: 2.5s and 5.5s).
- The per-test distribution in the integration binary is flat (largest 4s, median well under 0.1s). The slow suites are the filesystem-heavy ones (sync 23.5s, cache 18.6s, deploy 11.8s, server 11.8s, app 7.0s, provisioning 6.3s, crypt 5.5s, authn 5.4s): temp dirs, many small files, atomic rename writes, local HTTP servers. The ~20x slowdown versus Linux is per-file-operation overhead on the runner, not a few slow tests.

Target (a target, not a guarantee): test step under about 1m30s and whole job under about 2m30s.

## Progress


- [x] M0: measurement scaffolding (runner probe, cargo timings artifact, baseline recorded below; build/run split dropped, see Surprises).
- [ ] M1: libtest parallelism (`RUST_TEST_THREADS`).
- [ ] M2: ReFS Dev Drive for `target/` and the temp directory (Defender exclusion as fallback).
- [ ] M3: `rust-lld` as the MSVC linker.
- [ ] M4: cache workspace crates (`backend-api`, `device-api`) with a source-keyed cache.
- [ ] Final: measurement table complete, losers reverted, preflight `CLEAN`, PR out of draft.

## Surprises & Discoveries


- 2026-09-16 (M0, run 35132246902): splitting into `--no-run` build + run steps made the run step recompile `miru-agent` for 1m18s (job 6m6s vs 4m49s baseline). Cause: `agent/build.rs` emits `rerun-if-changed=.git/HEAD` and `.git/refs/`, which Cargo resolves relative to the package dir `agent/`, where no `.git` exists; a missing rerun-if-changed path is permanently stale, so the build script re-runs on every invocation, emits a fresh `MIRU_AGENT_BUILD_DATE`, and dirties the crate. Verified locally on Linux with `CARGO_LOG=cargo::core::compiler::fingerprint=info` (`stale: missing ".../agent/.git/HEAD"`). Fixing `build.rs` is out of this plan's scope (production build metadata semantics), so M0 keeps a single `cargo test --timings` step and derives compile time from the `Finished` timestamp. Filed as a follow-up task.
- 2026-09-16 (M0 probe): `RealTimeProtectionEnabled: False`, `AntivirusEnabled: True`, 4 CPUs, `TEMP=C:\Users\RUNNER~1\AppData\Local\Temp`, `RUNNER_TEMP=D:\a\_temp`. The `Get-MpPreference` and `Get-PSDrive` tables printed empty because pwsh defers formatting across object types; the probe now pipes each through `Out-String`. Real-time protection is already off, so the Defender-exclusion fallback in M2 is moot. Corrected probe (run 35133302125): `DisableRealtimeMonitoring: True`, `ExclusionPath: {C:\\, D:\\}`, `C:` 33 GB free, `D:` 156 GB free (dev drive fits on `D:`).
- 2026-09-16 (M1, run 35133866291): `RUST_TEST_THREADS=16` failed `workers::poller::run::ignored_syncer_events` (`agent/tests/workers/poller.rs:374`), a wall-clock assertion with a 1s drift tolerance; oversubscribing 4 cores 4x stretched the gap between loop iterations past that. `mod.rs` was not faster either (79.62s vs 73.67s). Trying 8 threads as the second and last round for this lever.
- 2026-09-16 (M0, run 35133302125): with no lever applied, `mod.rs` took 73.67s versus 108.5s at baseline and 108.70s one run earlier; run-to-run noise on `windows-latest` is on the order of 30s for the test phase. Judge levers on repeated runs, not single deltas.

## Decision Log


- 2026-09-16 (authoring): Rejected `cargo-nextest`. It runs each test in its own process; 1538 tiny tests would pay Windows process spawn (tens of ms each) plus a `taiki-e/install-action` download, and the tests already run in parallel threads inside one process. Its benefits (per-test timeouts, retries) are not the bottleneck.
- 2026-09-16 (authoring): Rejected test-side edits. The distribution is flat, so no single test is worth a Windows-specific change. The only pattern worth noting: tests that connect to `127.0.0.1:1` / `localhost:1` (`agent/tests/http/client.rs`, `agent/tests/http/errors.rs`, the SSE fixture in `agent/tests/server/sse.rs`) take 1-2s each on Windows because the TCP stack retries SYN to a closed port before reporting refusal; the SSE cursor tests additionally wait their 200ms stream timeout. These run in parallel and disappear into the noise once M1 lands.
- 2026-09-16 (authoring): Kept the pinned `Swatinem/rust-cache@6323deb1...`. That SHA is tag `v2.9.2` (and what `v2` resolves to), and its `action.yml` already has `cache-workspace-crates`, so no pin bump is needed.
- 2026-09-16 (authoring): Ranked the Dev Drive above Defender exclusions. The `windows-latest` image (Windows Server 2025) is built with `Set-MpPreference -DisableRealtimeMonitoring $true` and `ExclusionPath C:\, D:\` (runner-images `images/windows/scripts/build/Configure-WindowsDefender.ps1`), so exclusions likely add nothing; M0 probes the live state so the implementer knows rather than guesses.

## Outcomes & Retrospective


(Summarize at completion. Include the final measurement table.)

## Context and Orientation


`.github/workflows/ci.yml` defines four jobs. `windows-check` runs on `windows-latest` (4 vCPU, Windows Server 2025, NTFS; the workspace is `D:\a\agent\agent`, the runner temp dir `RUNNER_TEMP` is `D:\a\_temp`, and the process temp dir `TEMP`/`TMP` is on `C:`). Its steps: checkout, `dtolnay/rust-toolchain@...stable`, `Swatinem/rust-cache@6323deb1... # v2` with `key: test-suite` and `cache-on-failure: true`, `choco install nasm` (needed by `aws-lc-sys`), then one step `cargo test --package miru-agent --locked` with `RUST_LOG: off` and job env `CARGO_PROFILE_DEV_DEBUG: 0`.

Facts that shape the levers:

- rust-cache exports `CARGO_INCREMENTAL=0` in its restore step, so the `incremental = true` in `.cargo/config.toml` has no effect in CI, and rust-cache's deletion of `target/debug/incremental` on save costs nothing. Incremental state does not need caching.
- rust-cache's cache key is `v0-rust-<key input>-<job id>-<OS>-<arch>-<hash of rustc version + every env var whose name starts with CARGO, CC, CFLAGS, CXX, CMAKE or RUST>-<lockfile hash>`, and restore falls back to the prefix without the lockfile hash. Consequence: any job-level env var starting with `CARGO`/`RUST` creates a fresh cache namespace, so the first run after adding one compiles from a cold cache and is not a valid measurement; measure the second run. Step-level env on the test step is invisible to rust-cache and does not change the key.
- Cargo decides whether a path crate (like `libs/backend-api`) is fresh by comparing the modification time of every source file against the modification time of the dep-info file in `target/debug/.fingerprint/`. A fresh `actions/checkout` gives sources the checkout time, which is newer than anything restored from cache, so cached workspace-crate artifacts are always rebuilt unless source mtimes are pushed back. rust-cache never adjusts mtimes (upstream issues Swatinem/rust-cache#297 and #348 describe this).
- `rust-lld.exe` ships in every MSVC toolchain at `<sysroot>\lib\rustlib\x86_64-pc-windows-msvc\bin\`. rustc adds that directory to the linker child's `PATH` (`get_linker` in `compiler/rustc_codegen_ssa/src/back/linker.rs`) and infers the `lld-link` flavor from the file stem `rust-lld`, so `linker = rust-lld.exe` works on stable with no PATH change. Cargo reads that setting from the env var `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER`.
- libtest defaults to one thread per logical CPU (4 here). No test reads `RUST_TEST_THREADS` or `available_parallelism`; the 11 `#[serial]` tests (`agent/tests/app/run.rs`, `agent/tests/logs/mod.rs`, `agent/tests/logs_init_smoke.rs`) use `serial_test`'s global lock, which is independent of thread count. Mock servers bind port 0 except `run_rejecting_broker(18832, ...)`, used by exactly one test.
- Test temp directories come from `tempfile::Builder::tempdir()` (`agent/tests/test_utils/filesys/dirs.rs`), i.e. `std::env::temp_dir()`, which on Windows reads `TMP` then `TEMP`.
- `cargo test --package miru-agent` builds six link targets: lib test harness, bin test harness (`src/main.rs`, zero tests), `tests/mod.rs`, `tests/logs_init_smoke.rs`, `tests/logs_init_locked.rs`, plus a doctest pass (zero doctests). This plan keeps all of them so Windows still type-checks the binary.

Terms: "Dev Drive" is a Windows 11 / Server 2025 volume type backed by ReFS inside a VHDX file, designed for developer workloads; it skips most filesystem filter drivers and is markedly faster for many-small-files workloads. `samypr100/setup-dev-drive` is a GitHub Action that creates and mounts one (post step dismounts it). "libtest" is the built-in Rust test harness.

## Plan of Work


All edits are inside the `windows-check` job of `.github/workflows/ci.yml`. Land each milestone as its own commit so CI runs can be bisected, and record measurements in the table under Validation after every CI round. Keep a lever only if its numbers beat the previous round by more than run-to-run noise (take two warm runs when in doubt); otherwise `git revert` that commit.

M0, measurement scaffolding (no speed change expected). Replace the single test step with two steps so the job UI shows compile and run time separately: "Build Windows Tests" runs `cargo test --package miru-agent --locked --no-run --timings` and "Run Windows Tests" runs `cargo test --package miru-agent --locked` with `RUST_LOG: off` (the second reuses the build). Add a probe step before the build, `shell: pwsh`, that prints `Get-MpComputerStatus | Select-Object RealTimeProtectionEnabled, AntivirusEnabled`, `Get-MpPreference | Select-Object DisableRealtimeMonitoring, ExclusionPath`, `$env:NUMBER_OF_PROCESSORS`, `$env:TEMP`, `$env:RUNNER_TEMP`, and `Get-PSDrive C, D | Select-Object Name, Used, Free`. Optionally upload `target/cargo-timings/cargo-timing.html` with `actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1` (`if: always()`) to see per-crate compile durations. Record the baseline row.

M1, libtest parallelism. On the "Run Windows Tests" step add step-level env `RUST_TEST_THREADS: "16"` (step-level, so the cache key is unchanged). The tests are I/O-bound, so more threads than cores overlap waits. If 16 helps, try 32 in a second round; keep the best. Expected: the 108s `mod.rs` phase drops by roughly 2-3x; this is the likeliest largest win.

M2, ReFS Dev Drive for `target/` and temp files. Insert, after checkout and before rust-cache (so rust-cache's post-step save runs before the drive is dismounted), a step:

    - name: Create Dev Drive (ReFS) for target and temp
      uses: samypr100/setup-dev-drive@562171fe8df7401fdf582d766cd3b6ab80d5b1a0 # v4.1.0
      with:
        drive-size: 12GB
        drive-format: ReFS
        drive-type: Dynamic
        drive-path: D:\dev_drive.vhdx
        mount-path: ${{ github.workspace }}\target
        env-mapping: |
          TEMP,{{ DEV_DRIVE }}\tmp
          TMP,{{ DEV_DRIVE }}\tmp

followed by a `pwsh` step `New-Item -ItemType Directory -Force $env:TEMP | Out-Null`. Mounting at `<workspace>\target` keeps rust-cache's default `workspaces: . -> target` and every cargo command unchanged; `TEMP`/`TMP` on the drive moves every `tempfile` fixture there. Tag `v4.1.0` resolves to commit `562171fe8df7401fdf582d766cd3b6ab80d5b1a0` (`gh api repos/samypr100/setup-dev-drive/git/ref/tags/v4.1.0`; `v4` points at the same commit). If the M0 probe printed free space on `D:` below ~15 GB, use `drive-path: C:\dev_drive.vhdx` instead. Fallback if the Dev Drive fails or does not help and the M0 probe showed `RealTimeProtectionEnabled: True`: a `pwsh` step `Add-MpPreference -ExclusionPath "$env:GITHUB_WORKSPACE","$env:RUNNER_TEMP","$env:TEMP","$env:USERPROFILE\.cargo"` plus redirecting `TEMP`/`TMP` to `$env:RUNNER_TEMP\tmp` via `$env:GITHUB_ENV`. If the probe showed real-time protection already off, skip the fallback entirely.

M3, `rust-lld`. Add job-level env `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER: rust-lld.exe`. Job-level is deliberate: it starts with `CARGO`, so rust-cache keys a separate cache namespace and never mixes artifacts from the two linkers. First run is cold; measure the second. Expected: each of the five link steps gets faster (lld is typically 2-4x faster than `link.exe` on large binaries); combined with `CARGO_PROFILE_DEV_DEBUG: 0` already present, expect 10-30s off the compile phase. Revert if any link error mentions `lld-link` or `rust-lld`.

M4, cache the generated API crates. Change the rust-cache step to:

    with:
      key: test-suite-${{ hashFiles('libs/**') }}
      cache-on-failure: true
      cache-workspace-crates: true

The `hashFiles('libs/**')` in `key` puts the generated sources into both the full key and the fallback prefix, so a restored cache was always built from identical `libs/` content. That makes it safe to add a `pwsh` step after checkout (before the build) that pins mtimes:

    # Cargo treats a path crate as stale when any source is newer than the
    # cached dep-info. Checkout stamps sources with "now"; pin the generated
    # crates to a constant past time so unchanged libs are reused. Safe only
    # because hashFiles('libs/**') is part of the rust-cache key.
    $stamp = Get-Date "2000-01-01T00:00:00Z"
    Get-ChildItem -Path libs -Recurse -File | ForEach-Object { $_.LastWriteTime = $stamp }

Do not use `git log` commit times for the stamp: checkout is shallow (`fetch-depth` default 1), so the only commit is HEAD and its time is "now". Because `cache-workspace-crates` would also save `miru-agent`'s own artifacts (five large test binaries that are always rebuilt), add a final step `if: always()` running `cargo clean --package miru-agent --locked` so the saved cache stays close to the baseline ~461 MB; compare the "Cache Size" lines. Trade-off to record: a change to `libs/` (API regeneration) now yields one cold run because the fallback prefix changed too.

## Concrete Steps


All commands run from `/home/ben/miru/workbench4/repos/agent`. Heavy validation runs only in GitHub Actions; locally only edit, `git diff --check`, and commit.

Setup once:

    git fetch origin main
    git checkout perf/windows-ci-speed
    git log --oneline origin/main..HEAD
    gh pr create --draft --base main --title "perf(ci): speed up windows-check" --body "See plans/backlog/20260916-windows-ci-speed.md"

Then per milestone (M0 through M4): edit `.github/workflows/ci.yml` as described in Plan of Work, run `git diff --check`, commit, push, wait for CI, and record the row:

    git add .github/workflows/ci.yml plans/backlog/20260916-windows-ci-speed.md
    git commit -m "ci(windows): <lever>"        # e.g. "ci(windows): split build and run steps"
    git push
    gh run list --branch perf/windows-ci-speed --workflow CI --limit 1 --json databaseId,url,status
    gh run watch <RUN_ID> --exit-status
    gh run view <RUN_ID> --json jobs -q '.jobs[] | "\(.name) \(.conclusion) \(.startedAt) \(.completedAt)"'
    gh run view <RUN_ID> --job <WINDOWS_JOB_ID> --log | sed 's/\x1b\[[0-9;]*m//g' \
      | grep -E 'Finished|test result|Cache Size|Restored from cache|Compiling|RealTimeProtection|Free'

Expected greps: one `Finished \`test\` profile ... in Xs` (compile), a `test result: ok. 448 passed ... finished in Xs` (lib), `test result: ok. 1538 passed ... finished in Xs` (`mod`), two `1 passed`, and `Restored from cache key "..." full match: true`. If the line says `full match: false` or `No cache found`, the run is cold (expected right after M3 and M4, and after any `libs/` change): push an empty commit (`git commit --allow-empty -m "ci: warm cache"`) or re-run the job and measure that run instead. Step durations come from the job page or `gh api repos/mirurobotics/agent/actions/jobs/<WINDOWS_JOB_ID> -q '.steps[] | "\(.name) \(.started_at) \(.completed_at)"'`.

Suggested commit messages: M0 `ci(windows): split build and run steps and probe the runner`; M1 `ci(windows): run libtest with 16 threads`; M2 `ci(windows): put target and temp on a ReFS dev drive`; M3 `ci(windows): link with rust-lld`; M4 `ci(windows): cache generated api crates`. A rejected lever is removed with `git revert <sha>` and a Decision Log entry citing its numbers.

## Validation and Acceptance


The Windows job itself is the test. Acceptance for every round: `windows-check` succeeds, its log shows `448 passed`, `1538 passed`, `1 passed` twice and `0 failed` everywhere (same counts as baseline), the job id is still `windows-check`, and `lint`, `test` and `tools` are green and unchanged in duration (they share no edited YAML).

Fill this table after each CI round (warm-cache runs only):

| Round | Commit | Job total | Build step | Run step | lib finished in | mod finished in | Cache size | Keep? |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Baseline | 4b3ac66 | 4m49s | (in 3m50s step) 1m36s | (in 3m50s step) | 12.1s | 108.5s | ~461 MB | n/a |
| M0 (split) | bb104ce | 6m6s | 1m32s | 3m26s (incl. 1m18s recompile) | 16.07s | 108.70s | ~461 MB | no (split dropped) |
| M0 | 246838d | 4m20s | 1m38s | 86s | 10.81s | 73.67s | ~461 MB | yes (scaffolding) |
| M1 (16 threads) | 0a27e19 | 4m04s (red) | 1m20s | 91s | 10.85s | 79.62s, 1 failed | ~461 MB | no |
| M1 | | | | | | | | |
| M2 | | | | | | | | |
| M3 | | | | | | | | |
| M4 | | | | | | | | |

Final acceptance: the run on the pushed branch head with all kept levers is green, the table is complete with a Decision Log entry per rejected lever, and preflight (the `$preflight` workflow: CI on the pushed branch head, draft PR) reports exactly `CLEAN`. Only then does the PR leave draft and this task count as complete; a later commit needs its own green run. Report the final job total against the 2m30s target honestly, whether or not it is met.

## Idempotence and Recovery


Every milestone is a single-commit YAML change: repeating a push re-runs the same measurement; reverting is `git revert <sha>`. M3 and M4 change the rust-cache key, so their first run is cold; that is expected, not a failure. If M2's action fails to create or mount the drive (`New-VHD`/`Mount-VHD` errors), revert M2 and try the Defender-exclusion fallback only if the M0 probe showed real-time protection enabled. If M4 ever produces a compile error that mentions a symbol from `backend-api` or `device-api` that exists in the current sources, the key/hash invariant was broken: revert M4 immediately and re-establish that `hashFiles('libs/**')` is part of `key`. `cargo clean --package miru-agent` only removes that package's artifacts and can be re-run safely. Nothing in this plan touches production code, so no rollback beyond the workflow file is ever needed.
