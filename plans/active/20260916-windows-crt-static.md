# Statically link the MSVC CRT into the Windows agent binary

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench2/repos/agent) | read-write | Rust workspace for the Miru device agent (crate `miru-agent` in `agent/`). All edits are build configuration and docs: `.cargo/config.toml`, `build/windows/README.md`, and optionally a one-line comment in `.github/workflows/ci.yml` / `.github/workflows/release.yml`. No Rust source and no WiX changes. Validation and commits happen here. |

Branch `feat/windows-crt-static` (already created and checked out; based on `feat/windows-msi-artifact` = PR #252 at head `72b99b13`). **PR base is `feat/windows-msi-artifact`, NOT `main`.** This is deliberate stacking: the base branch is where the `windows-package` CI job both builds the release binary and uploads the `miru-agent-msi-unsigned` artifact, so stacking on it makes the PR's own `windows-package` run build the *statically linked* binary and upload an MSI a human can install on a clean machine to prove the fix.

This plan lives in `agent/plans/active/` because all changes are in the `agent` repo.

## Purpose / Big Picture

Today the Windows agent binary fails to start on a customer machine that does not have the Microsoft Visual C++ 2015–2022 Redistributable installed. The Rust `x86_64-pc-windows-msvc` target links the MSVC C runtime **dynamically** by default, so the produced `miru-agent.exe` depends on `VCRUNTIME140.dll` and `MSVCP140.dll`, which ship only with that redistributable. On a box without it the DLLs are missing and the executable cannot load:

- Installing the MSI and letting the Service Control Manager (SCM) start the service fails: the start step times out and the System event log records **Event 7009** (service did not respond to the start request in time), because the process image cannot be loaded at all.
- Running the binary interactively, `miru-agent.exe --console`, exits immediately with **`0xC0000135` (`STATUS_DLL_NOT_FOUND`)** — the Windows loader could not find a required DLL.

CI never caught this because the GitHub-hosted Windows runners (`windows-latest` / `blacksmith-*-windows-2025`) always have the VC++ redistributable pre-installed, so both the service integration tests and interactive runs succeed there.

After this change, the Windows build sets `-C target-feature=+crt-static`, which statically links the MSVC CRT into `miru-agent.exe`. The binary becomes **self-contained**: it no longer imports `VCRUNTIME140.dll`/`MSVCP140.dll`, so it loads and the service starts on a stock Windows machine with **no VC++ Redistributable prerequisite**.

Observable outcome: on a Windows machine that has never had the VC++ redistributable installed, installing the MSI built by this branch's `windows-package` job (uploaded as the `miru-agent-msi-unsigned` artifact) registers and starts the `miru-agent` service without an Event 7009 timeout, and `miru-agent.exe --console` runs instead of exiting `0xC0000135`. Because CI runners always have the redistributable, this end-to-end proof can only be confirmed by a human on a clean machine (see Validation).

## Progress

- [ ] M0 Activate plan (`docs(plans):` commit)
- [ ] M1 Statically link the MSVC CRT (`build(windows):` — `.cargo/config.toml` + `build/windows/README.md` + optional workflow comment)
- [ ] M2 Preflight CLEAN; CI green on the pushed head (all jobs incl. `windows-check` and `windows-package`); draft PR opened against `feat/windows-msi-artifact` and (only then) the draft state resolved

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

(Add entries as work proceeds. Decisions taken at authoring time, 2026-09-16, Benjamin Smidt:)

- **Static CRT over dynamic.** Statically linking the MSVC CRT (`-C target-feature=+crt-static`) makes `miru-agent.exe` self-contained, so a customer install needs no per-machine Visual C++ Redistributable and the `0xC0000135` / Event 7009 start failure on a clean box is eliminated. The alternative — keep dynamic linking and ship/require the redistributable — adds a customer prerequisite and an installer dependency (bootstrapper or merge module) that the current single-MSI packaging does not have, and leaves the failure mode one missed prerequisite away. Self-contained is the right default for an unattended, auto-starting service.
- **Serviceability tradeoff, accepted.** With the CRT statically linked, Windows Update no longer patches the CRT inside our binary; a CRT security fix reaches customers only when we rebuild and release the agent. This is an accepted tradeoff — the agent already ships as a self-updating, regularly released binary, and self-contained loading is worth more than delegating CRT patching to the OS. Recorded here so it is a conscious choice, not an accident.
- **Config placement: `[target.x86_64-pc-windows-msvc]` in `.cargo/config.toml`.** Putting `rustflags` under the target table (rather than a blanket `[build] rustflags`) scopes the flag to the Windows MSVC target only and leaves Linux/macOS builds untouched. It applies to **both** Windows build paths: the host-target `windows-check` test build (`cargo test --package miru-agent --locked`, which compiles for the runner's native `x86_64-pc-windows-msvc`) **and** the explicit cross/`--target x86_64-pc-windows-msvc --release` build used by `windows-package` (MSI) and `windows-release-build` (release). Both therefore get a statically linked CRT — desirable, since the tested binary and the shipped binary link the CRT the same way.
- **Do not touch the existing `[profile.dev]` / `[profile.test]` `incremental = true` settings.** They are unrelated to CRT linkage; this change only adds a new `[target.*]` table.
- **Risk to watch: C dependencies linking the static CRT consistently.** `aws-lc-sys` (via NASM, already installed in CI) and any other `cc`-compiled C in the dependency tree must compile their C objects against the static CRT (`/MT`) to match. Under `+crt-static`, the `cc` crate is expected to select `/MT` automatically, so no manual `CFLAGS` should be needed. The thing to watch for is a CRT-mismatch linker error in `windows-check` or `windows-package` — e.g. `LNK2038: mismatch detected for 'RuntimeLibrary'` (MT_StaticRelease vs MD_DynamicRelease) or duplicate/undefined CRT symbols. If that appears, it is the concrete thing to fix (typically by ensuring the C build also uses the static CRT). CI is the authority that this links cleanly; it cannot be checked on Linux.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

All paths are relative to `/home/ben/miru/workbench2/repos/agent`.

**Dynamic vs static CRT (the mechanism).** By default the `x86_64-pc-windows-msvc` target links the MSVC CRT dynamically (`/MD`), so `miru-agent.exe` imports `VCRUNTIME140.dll`/`MSVCP140.dll` from the "Microsoft Visual C++ 2015–2022 Redistributable (x64)" — the missing-DLL cause of the `0xC0000135` / Event 7009 failure described in Purpose. The `crt-static` target feature switches to static linkage (`/MT`), baking the CRT into the exe so no external DLL is required.

**The config file today.** `.cargo/config.toml` currently contains only build profiles and no `[target.*]` table:

    [profile.dev]
    incremental = true

    [profile.test]
    incremental = true

Cargo reads `rustflags` from a `[target.<triple>]` table in `.cargo/config.toml` and applies them to every crate compiled for that target. Adding `[target.x86_64-pc-windows-msvc]` with `rustflags = ["-C", "target-feature=+crt-static"]` is the standard, target-scoped way to statically link the MSVC CRT (see the Rust reference on linkage: the flag is `-C target-feature=+crt-static`; targets that cannot switch CRT linkage ignore it, so it is safe to leave in the checked-in config).

**Where the Windows binary is built (all three consume `.cargo/config.toml`).**

- `.github/workflows/ci.yml` → `windows-check` (line 41, `blacksmith-4vcpu-windows-2025`): `cargo test --package miru-agent --locked`. This builds and tests for the runner's **host** `x86_64-pc-windows-msvc` target, so the new `rustflags` apply here too. This is where a CRT-mismatch link error would first surface.
- `.github/workflows/ci.yml` → `windows-package` (line 91, `windows-latest`, gated by the `windows_package_scope` paths filter on `build/windows/**`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`): builds `cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked --release`, restores the pinned WiX SDK, runs `package-tests.ps1` and the elevated `integration-tests.ps1 -ConfirmDisposableTestMachine` matrix, then (on the PR #252 base branch this PR stacks on) builds and **uploads the `miru-agent-msi-unsigned` artifact** (`.github/workflows/ci.yml` "Upload unsigned installer MSI" step). This job is the CI authority for this change and produces the MSI a human uses for the true end-to-end proof.
- `.github/workflows/release.yml` → `windows-release-build` (line 54): `cargo auditable build --release --target x86_64-pc-windows-msvc -p miru-agent --locked`. Also picks up the flag, so shipped release binaries are statically linked too.

**Triggering `windows-package`.** The `windows_package_scope` paths filter runs `windows-package` on any change to `.github/workflows/ci.yml`, but `.cargo/config.toml` is **not** in the filter. So to guarantee this PR's `windows-package` runs, the plan requires the one-line comment edit to `.github/workflows/ci.yml` (Plan of Work M1, step 3); editing the paths-filter list itself is unnecessary. Do not rely on `.cargo/config.toml` alone to trigger the job.

**Documentation to update.** `build/windows/README.md` documents the MSI's build/install/validation story. It currently states the build "requires … the Rust MSVC toolchain" and lists install prerequisites, but says nothing about the runtime CRT dependency. It needs a short note that the installed agent is self-contained and needs no VC++ Redistributable because the binary statically links the CRT.

**Local validation limits.** There is no Windows host here. `./scripts/test.sh` runs `RUST_LOG=off cargo test --package miru-agent` on Linux (the host target is `x86_64-unknown-linux-gnu`); the new `[target.x86_64-pc-windows-msvc]` table does not apply to that target, so `test.sh` stays green but **does not exercise the Windows build at all**. WiX cannot be built and MSIs cannot be installed on Linux. **The CI Windows jobs (`windows-check`, `windows-package`) are the sole authority** that the flag compiles and links cleanly and that the MSI still builds; the true end-to-end proof (a clean machine with no redistributable) is a human step (Validation).

## Plan of Work

The change is a target-scoped build-config flag plus documentation. It is small and additive.

### M1 — Statically link the MSVC CRT

1. **`.cargo/config.toml`** — append a new `[target.x86_64-pc-windows-msvc]` table with the crt-static rustflag and an explanatory comment. Leave the existing `[profile.dev]` and `[profile.test]` `incremental = true` settings exactly as they are. Add:

        # Statically link the MSVC C runtime into the Windows binary so it is
        # self-contained: customer machines need no Visual C++ Redistributable
        # (VCRUNTIME140.dll / MSVCP140.dll) for the service to start. Tradeoff:
        # CRT security fixes now ship via agent rebuilds/releases rather than
        # through Windows Update of the shared redistributable.
        [target.x86_64-pc-windows-msvc]
        rustflags = ["-C", "target-feature=+crt-static"]

2. **`build/windows/README.md`** — add a brief note, fitting the existing structure, that the installed agent is self-contained and needs no Visual C++ Redistributable prerequisite because the Windows binary statically links the MSVC CRT. Natural home: a sentence in the "Install and provision" section next to the existing prerequisites (the paragraph that today discusses installing the trusted MSI), or a short note under "Build" that the produced `miru-agent.exe` statically links the CRT. Keep it to one or two sentences; do not restructure the file.

3. **`.github/workflows/ci.yml` (required, one line)** — add a one-line comment on the `windows-package` "Build Windows Target" step (line 115–116) noting the MSVC build is statically linked (self-contained; no VC++ redistributable). This doubles as the guaranteed trigger for the `windows_package_scope` paths filter so `windows-package` runs on this PR. Example, placed above the `run:` of the "Build Windows Target" step:

        # The MSVC build statically links the CRT (see .cargo/config.toml) so the
        # resulting exe/MSI is self-contained and needs no VC++ redistributable.

4. **`.github/workflows/release.yml` (optional, only if it fits naturally)** — a matching one-line comment on the `windows-release-build` "Build Windows Release" step (line 87–88) that the release binary is statically linked. Do not over-edit; skip if it does not read cleanly.

There is no way to compile or link the Windows target on Linux; correctness is confirmed by the M2 CI run, which is also where the CRT-mismatch linker error flagged in the Decision Log would surface (`windows-check` or `windows-package`).

### M2 — Preflight and CI (the authority)

Run preflight, push, open the draft PR against `feat/windows-msi-artifact`, and iterate on the `windows-check` and `windows-package` jobs until green. Details in Concrete Steps.

## Concrete Steps

Working directory for every command: `/home/ben/miru/workbench2/repos/agent`. One commit per milestone; every commit message ends with the trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

### M0 — Activate plan

1. This plan file exists at `plans/active/20260916-windows-crt-static.md`.
2. Commit:

        git add plans/active/20260916-windows-crt-static.md
        git commit -m "docs(plans): add windows CRT static-link plan" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M1 — Static CRT build config + docs

1. Edit `.cargo/config.toml`, `build/windows/README.md`, and `.github/workflows/ci.yml` (and optionally `.github/workflows/release.yml`) as in Plan of Work M1.
2. Sanity checks (Linux, no Windows build):

        # config parses and has the target table + flag, profiles untouched
        python3 -c "import tomllib; d=tomllib.load(open('.cargo/config.toml','rb')); assert d['target']['x86_64-pc-windows-msvc']['rustflags']==['-C','target-feature=+crt-static'], d; assert d['profile']['dev']['incremental'] is True and d['profile']['test']['incremental'] is True; print('config ok')"
        grep -n 'crt-static\|x86_64-pc-windows-msvc' .cargo/config.toml
        grep -in 'redistributab\|self-contained\|statically link' build/windows/README.md

   Expect `config ok`, the target table and flag present, the existing profile settings intact, and the README note present. (The Windows target cannot be built here — that is M2/CI.)
3. `./scripts/test.sh` still ends with `test result: ok.` (Linux host target is unaffected by the Windows-only `rustflags`).
4. Commit:

        git add .cargo/config.toml build/windows/README.md .github/workflows/ci.yml .github/workflows/release.yml
        git commit -m "build(windows): statically link the MSVC CRT" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

   (Drop `.github/workflows/release.yml` from the `git add` if step M1.4 was skipped.)

### M2 — Preflight, PR, and CI (authority)

1. Preflight must be CLEAN before delivery (never skip preflight):

        ./scripts/preflight.sh

   Expect exit 0 and the final "clean" line. Preflight is Linux/Rust-only and does not exercise the Windows target, but is the repo's required gate before a PR leaves draft.
2. Push and open the **draft** PR with base `feat/windows-msi-artifact` (the stacked base, not `main`), so its `windows-package` run builds the statically linked binary and uploads the MSI. Write the PR body to `/tmp/windows-crt-static-pr.md` in the repo's PR style, ending with the line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`, then:

        git push -u origin feat/windows-crt-static
        gh pr create --draft --base feat/windows-msi-artifact \
            --title "build(windows): statically link the MSVC CRT" \
            --body-file /tmp/windows-crt-static-pr.md
        gh pr checks --watch

3. Iterate from CI logs until **every** job is green on the pushed head: `lint`, `test`, `tools`, `windows-check`, and especially `windows-package`. If a CRT-mismatch linker error appears (`LNK2038`, duplicate/undefined CRT symbols), fix it (ensure C dependencies compile against the static CRT) and re-push.
4. Confirm the `windows-package` run uploaded the `miru-agent-msi-unsigned` artifact (it is built from the now-statically-linked binary). This artifact is what the user installs for the true end-to-end proof.
5. Only once all CI jobs are green does the PR leave draft (leaving draft is the orchestrator's call).

## Validation and Acceptance

Required before the PR leaves draft or the task is reported complete:

1. **Preflight CLEAN.** `./scripts/preflight.sh` exits 0 with its final "clean" line. (Linux/Rust-only and unaffected by the Windows-only flag, but must be run — never skip preflight.)
2. **`./scripts/test.sh` green.** Ends with `test result: ok.`; the Linux host target is unaffected, so this only confirms no accidental breakage. It does **not** exercise the Windows build.
3. **CI green on the pushed head — the CI authority.** All jobs green: `lint`, `test`, `tools`, `windows-check`, and especially `windows-package`. Specifically:
   - `windows-check` (`cargo test --package miru-agent --locked`) compiles and links the host MSVC target with `+crt-static` and passes — no `LNK2038` RuntimeLibrary mismatch, no duplicate/undefined CRT symbols.
   - `windows-package` builds `--target x86_64-pc-windows-msvc --release` with the static CRT, restores WiX, passes `package-tests.ps1` and the elevated `integration-tests.ps1 -ConfirmDisposableTestMachine` matrix, and uploads the `miru-agent-msi-unsigned` artifact built from the statically linked binary.

   Because the GitHub Windows runners always have the VC++ redistributable installed, green CI proves the binary **compiles and links** statically but **cannot** prove the original failure is gone — the runners would start the service either way.

4. **True end-to-end proof — user-confirmed on a clean machine (cannot be done in CI or here).** The definitive check that the fix works can only be performed by the user on their own Windows laptop / a VM that has **never had** the Visual C++ Redistributable installed, using the `miru-agent-msi-unsigned` artifact uploaded by this PR's `windows-package` run:
   - Install the MSI on the clean machine. The `miru-agent` service registers and **starts** without an **Event 7009** start-timeout in the System event log.
   - Run `miru-agent.exe --console` from `Program Files\Miru\Agent`. It runs instead of exiting immediately with `0xC0000135` (`STATUS_DLL_NOT_FOUND`).
   - (Optional confirmation) `dumpbin /dependents miru-agent.exe` (or Dependencies/`link /dump /dependents`) shows **no** `VCRUNTIME140.dll` / `MSVCP140.dll` import.

   This step is outside CI's and this environment's reach; report the PR and the artifact and hand this proof to the user.

## Idempotence and Recovery

All edits are additive and re-runnable; re-applying M1 over a partially applied tree converges. One commit per milestone, so `git revert <sha>` unwinds one in isolation. Risky point: the CRT-linkage behavior is unknowable until the CI Windows build. If `+crt-static` triggers a CRT-mismatch link failure that cannot be resolved quickly, reverting the M1 commit restores the dynamic-CRT build (the pre-existing behavior) with no other side effects. No Rust source, WiX, customer state, or non-Windows build behavior is touched.
