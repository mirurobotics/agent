# Harden and validate the Windows MSI packaging

This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective current as implementation proceeds. Continue through every milestone without waiting for further instructions, and make one Conventional Commit at the end of each milestone.

## Scope

| Repository | Access | Description |
|---|---|---|
| `mirurobotics/agent` | read-write | Harden PR #236's WiX MSI and PowerShell tools, add native Windows validation, and correct the agent Windows roadmap. |

This plan follows the repository lifecycle from `plans/backlog/` to `plans/active/` during implementation and `plans/completed/` at closure. The workbench and other Miru repositories are out of scope and must not be modified. The base branch is `main`; implementation continues the draft branch `feat/windows-msi-packaging` and PR #236.

This work deliberately does not add Windows Service Control Manager integration, service install/start/stop/recovery behavior, a release or GoReleaser build lane, artifact publication, PDB handling, or Authenticode signing. Those are separate roadmap changes. In particular, do not make the current console-capable executable pretend to be a Windows service.

## Purpose / Big Picture

After this work, a developer can build a real x64 `miru-agent.exe`, produce a validated x64 MSI with pinned tooling, and safely exercise install, maintenance, upgrade, downgrade rejection, rollback, and uninstall on Windows. The MSI installs the console-capable binary under 64-bit Program Files, protects `%ProgramData%\Miru` so only SYSTEM and built-in Administrators can access inherited customer state, and retains that state across upgrades and ordinary uninstall. It does not register a `MiruAgent` service.

The PowerShell installer refuses an unexpected or malformed MSI, verifies an exact checksum record for downloads, handles Windows PowerShell 5.1 safely, and reports a reboot-required result distinctly. The provisioning wrapper accepts its secret only through `MIRU_PROVISIONING_TOKEN`, never places it on the command line or in output, directly invokes the installed executable, and preserves the `provision --check` exit contract. CI proves the package and scripts on a native Windows runner. PR #236 remains draft until preflight reports **CLEAN** for the exact pushed branch head, where CLEAN means all required GitHub CI checks are green on that commit.

## Progress

Implementation started at `92cb9254fe4ee14e455d5041dc07d1b94da2be04`. `git merge-base --is-ancestor origin/main 92cb9254fe4ee14e455d5041dc07d1b94da2be04` exits 0, verifying that the current `origin/main` is an ancestor of that implementation start.

- [x] Remove every production service declaration and commit one stable UpgradeCode.
- [x] Add the pinned x64 WiX project, required-input checks, and stable-only MSI version validation.
- [x] Implement protected ProgramData ACLs, transactional upgrades, and state-preserving uninstall behavior.
- [ ] Build and inspect normal and boundary-valid MSIs and reject invalid build inputs.
- [ ] Commit Milestone 1.
- [x] Harden `scripts/install/install.ps1`.
- [x] Harden `scripts/install/provision.ps1`.
- [ ] Parse and exercise both scripts under Windows PowerShell 5.1.
- [ ] Commit Milestone 2.
- [x] Add deterministic install, ACL, upgrade, rollback, and uninstall fixtures.
- [ ] Run the native integration matrix.
- [x] Wire the native integration entry point into Windows CI.
- [ ] Commit Milestone 3.
- [ ] Update Windows packaging docs and the umbrella plan, then run repository-wide validation.
- [ ] Commit Milestone 4 and push the exact branch head from a clean working tree.
- [ ] Run the clean Windows 10/11 VM smoke pass on that pushed commit and update PR #236's body with the evidence.
- [ ] Run preflight until it reports CLEAN, then prepare Progress and Outcomes & Retrospective with the smoke-tested and first-CLEAN implementation SHA and evidence for the focused plan-only closure commit, which is the final repository mutation.

After that checklist is complete and committed, post-closure push, CLEAN verification, exact SHA/check comparison, PR-body update, and undrafting are external closure actions. Record them only in the verified PR body and final task result, not by another plan edit or checklist update.

## Surprises & Discoveries

- Source implementation was committed as `bcc152ec168b46734cac13929c6f03eb96cfda6f` (`feat(windows): harden MSI packaging tools`), `d276bebf7e8d9929f3955670fb14fe83403f200e` (`fix(windows): repair MSI maintenance behavior`), and `1ac94f73a7672538e38ff4fb3874f9b02b04c696` (`test(windows): validate MSI package lifecycle`). The planned milestone checkpoints were omitted, so the checked Progress items above record source inspection only, not Windows execution or current-head CI evidence.

## Decision Log

- Decision: Refined the authored plan to make package and boundary validation, PowerShell parsing and TLS restoration, ACL reapplication, the rollback fixture, branch review, and closure-head verification deterministic and directly executable.
  Rationale: The prior draft omitted valid MSI boundary builds and crossed TLS cases, under-specified maintenance/upgrade ACL repair, contained an unsupported rollback action and shared parser error-array bug, and allowed plan mutations after closure or readiness checks against a superseded head.
  Date/Author: 2026-09-11, Codex.
- Decision: This plan was activated at implementation start and remains active until the final evidence closure, when it moves to completed as part of the plan-only evidence closure commit that is the final repository mutation.
  Rationale: The active plan records milestone checkpoints, smoke evidence, and the first-CLEAN implementation SHA; the verified PR body and final task result are authoritative for `Preflight: CLEAN on <immutable closure SHA>`, so no later plan-only commit records post-closure CLEAN.
  Date/Author: 2026-09-11, Codex.

## Outcomes & Retrospective

Complete this section after all acceptance criteria pass. Summarize delivered behavior, the exact tested commit, and deferred roadmap work.

## Context and Orientation

The repository root is `/home/ben/miru/workbench5/repos/agent`. The branch `feat/windows-msi-packaging` contains draft PR #236. At plan creation it is based on the merge of native Windows compile CI from PR #234 and differs from `main` by four new files: `build/windows/README.md`, `build/windows/miru-agent.wxs`, `scripts/install/install.ps1`, and `scripts/install/provision.ps1`. The remote feature branch is behind this local branch, so do not assess PR readiness until the final local head is pushed.

`agent/src/main.rs` is important negative context: the Windows path waits for console control events, but it does not call `StartServiceCtrlDispatcher` or handle Windows service control messages. Therefore the current `ServiceInstall`, `ServiceControl`, and `util:ServiceConfig` declarations in `build/windows/miru-agent.wxs`, and the service stop/start logic in `scripts/install/provision.ps1`, cannot work. Remove them. The absence of a `MiruAgent` service is an acceptance condition for this PR; service lifecycle belongs to a later roadmap PR.

`build/windows/miru-agent.wxs` is the WiX source. WiX turns this XML into an MSI. `UpgradeCode` is the permanent GUID that relates major-upgrade packages and must be generated once, committed, asserted by tests, and never changed in later releases. `ProductCode` may change between package versions. MSI `ProductVersion` is not general SemVer: it must have exactly three numeric fields for this project, its first two fields must be at most 255, its third must be at most 65535, a fourth field is ignored by Windows Installer, and prerelease labels are invalid. For PR #236, support only stable release versions of the form `MAJOR.MINOR.PATCH` within those bounds and reject prerelease/build labels or extra fields. Keep the release tag/display/asset version separate from this MSI-only numeric value. This explicit restriction avoids collisions such as two beta tags mapping to the same MSI version; prerelease MSI sequencing remains deferred until a monotonic, tested mapping is designed.

An MSI major upgrade installs a new ProductCode with the same UpgradeCode. Schedule `RemoveExistingProducts` transactionally at `afterInstallInitialize`: removal and replacement then occur in the installer transaction, so a later failure can roll back to the previously installed product. Configure downgrade blocking. Reinstalling the identical MSI is maintenance mode, not a new product. Customer state under `%ProgramData%\Miru` must remain on maintenance, upgrade, rollback, and ordinary uninstall, while the executable, Add/Remove Programs entry, and installer-owned registry metadata are removed when uninstall succeeds.

The current WiX command defaults to x86 unless architecture is explicit, while `ProgramFiles64Folder` and the Rust target are x64. Add a pinned `WixToolset.Sdk` 5.0.2 project under `build/windows/` with an explicit x64 platform, Windows Installer 5.0 (`InstallerVersion=500`), validation enabled, and warnings treated as errors where the tool supports it. Make both `Version` and `BinDir` required build inputs with clear errors; never fall back to `0.0.0` or an implicit binary path. The production package must not need the WiX Util extension after service configuration is removed.

An ACL is an access-control list. WiX Util's existing `PermissionEx` defaults to appending permissions, so it does not replace inherited access on a pre-existing directory. That can expose future secrets such as `auth/private_key.pem` and `token.json`. Use the core WiX MSI-5 `PermissionEx` SDDL form instead. SDDL is the string form of a Windows security descriptor. Apply a protected DACL (disabled inheritance) with inheritable full-control ACEs for only `SY` (Local System) and `BA` (built-in Administrators), for example the semantic descriptor `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`. Confirm the exact WiX syntax by compiling it; do not weaken the descriptor to silence validation. The installer must correct a deliberately permissive pre-existing tree as well as create a secure new one.

`.github/workflows/ci.yml` already has a `windows-latest` job from PR #234 that installs Rust for `x86_64-pc-windows-msvc`, installs NASM, and runs `cargo check`. Extend or complement that native job for packaging. Build the real executable with `cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked --release`. Do not upload it as a release artifact and do not add signing. `scripts/test.sh`, `scripts/covgate.sh`, `scripts/update-deps.sh`, and `scripts/lint.sh` are the repository's local validation commands. `scripts/update-deps.sh` intentionally refreshes `Cargo.lock`; inspect and reject unrelated lockfile drift.

PowerShell tests must target Windows PowerShell 5.1 first because customer machines may use it; `pwsh` can be a secondary parse/run check. Use `[System.Management.Automation.Language.Parser]` to reject syntax errors. Prefer a repository-owned, dependency-free test harness under `build/windows/tests/` so CI does not depend on whatever Pester version happens to be preinstalled. Test-only fake executables, local HTTP responses, MSI fixtures, and a deliberately failing upgrade fixture may be generated in the runner's temporary directory and must never enter the release package.

## Plan of Work

### Milestone 1: package contract, security, and upgrades

In `build/windows/miru-agent.wxs`, replace the placeholder UpgradeCode with a newly generated stable GUID and put a warning next to it that changing it breaks upgrades. Remove the WiX Util namespace and all service installation, control, and recovery elements. Mark the executable component and package as x64, require build-provided `Version` and `BinDir`, set `InstallerVersion=500`, and configure `MajorUpgrade` with downgrade blocking and transactional `afterInstallInitialize` scheduling. Give installer-owned binary and registry components normal uninstall behavior, but model the ProgramData directory and its ACL so customer state is not removed on uninstall or upgrade. Do not leave installer registry values behind merely to keep data directories permanent; separate key paths/components or use a registry-free directory key path supported by WiX.

Replace appended Util permissions with core WiX `PermissionEx` using a protected, inheritable SYSTEM-and-Administrators-only DACL on `%ProgramData%\Miru` and every separately authored sensitive child directory. Ensure a repair or upgrade reapplies the descriptor to a pre-existing tree. No generic users, authenticated users, current user, or inherited parent ACE may retain access.

Add `build/windows/miru-agent.wixproj` using `WixToolset.Sdk` version 5.0.2. Make x64, warnings-as-errors, MSI/ICE validation, and required `Version` and `BinDir` inputs explicit. Add a small version-validation build target or equivalent source-generation check that accepts only stable numeric three-part versions within Windows Installer bounds. Use the SemVer string only for release-facing display or asset names; do not silently strip a `v`, prerelease label, build label, or fourth field at the MSI boundary.

Create `build/windows/tests/package-tests.ps1` as the single package-contract harness with required `-ProjectPath`, `-BinDir`, and `-ArtifactsDirectory` parameters. It may delete and recreate only its deterministic `v1`, `v2`, `boundaries`, `invalid`, and `logs` children below the supplied artifacts directory. It builds versions 1.0.0 and 1.1.0, copies the results to `build/windows/artifacts/package-tests/v1/miru-agent-1.0.0.msi` and `build/windows/artifacts/package-tests/v2/miru-agent-1.1.0.msi`, runs MSI/ICE validation, and inspects both packages for ProductName, Manufacturer, ProductVersion, UpgradeCode, ProductCode, and x64 Summary Information. It must assert that the two ProductCodes differ and their UpgradeCode is identical. It also builds, validates, and inspects the expected-success boundary packages at `build/windows/artifacts/package-tests/boundaries/miru-agent-0.0.0.msi` and `build/windows/artifacts/package-tests/boundaries/miru-agent-255.255.65535.msi`, and emits exactly `PASS version boundaries (0.0.0, 255.255.65535)`. Finally, it invokes expected-failure builds for omitted and empty `Version`, omitted and empty `BinDir`, `1.2.3-beta.1`, `1.2.3.4`, `256.0.0`, `1.256.0`, and `1.2.65536`, failing the harness if any invalid build succeeds or lacks the expected input/version diagnostic.

Build at least two valid versions and invalid-version cases on Windows. Run WiX's MSI/ICE validation with warnings as errors where possible; if a platform ICE produces a documented false positive, narrowly suppress that ICE in the project and explain the evidence in `build/windows/README.md`, never suppress all validation.

### Milestone 2: safe PowerShell tools

Refactor `scripts/install/install.ps1` into small functions that can be exercised without performing a network download or installation when dot-sourced by the test harness. Check for an elevated Administrator token before any network, temporary-file, or installation work, and reject a non-x64 OS or 32-bit PowerShell host with an actionable message. For Windows PowerShell 5.1, enable TLS 1.2 for GitHub requests while preserving the process's previous protocol setting, and pass behavior equivalent to `-UseBasicParsing` where required.

Remove the `-Prerelease` switch because this PR intentionally does not support prerelease MSIs. Continue accepting a leading `v` in stable release input for download naming, but validate the resulting value against the strict MSI version contract rather than stripping any other suffix.

For downloads, create a cryptographically unique directory below the system temporary directory. Clean it in `finally` on success or handled failure. Parse the checksum file as records: exactly one line must contain exactly 64 hexadecimal digest characters and the exact MSI filename, with normal checksum-file whitespace/optional binary marker syntax but no substring filename matches. Reject zero matches, duplicate matches, malformed digests, and case variants of the filename; compare normalized digests with an invariant ordinal comparison. Exercise TLS compatibility with four focused cases: TLS 1.2 initially absent and initially present, each crossed with an injected successful request and an injected throwing request. Every case must restore the exact original `[System.Net.ServicePointManager]::SecurityProtocol` value, and the test harness must restore its own original value in an outer `finally`. Document that checksums detect corruption but do not authenticate the publisher because Authenticode signing is deferred.

Before invoking `msiexec`, query the MSI through the Windows Installer API. Require Miru's exact ProductName and Manufacturer, the committed UpgradeCode, x64 platform metadata, and a valid three-part MSI ProductVersion. If `-Version` accompanies `-FromMsi`, require an exact version match after only the explicitly documented leading `v` normalization at the release-input boundary; otherwise report the metadata version being installed. Reject an unknown MSI before any state change. Invoke `msiexec` with an argument array that preserves paths containing spaces. Exit 0 for success, clearly report and propagate 3010 for success requiring reboot, and retain a verbose log only for installation failure. The error must name the retained log path.

In `scripts/install/provision.ps1`, remove the public `-Token` parameter. Normal provisioning must require a non-empty `MIRU_PROVISIONING_TOKEN` process environment variable, check elevation before changing state, reject a 32-bit host on 64-bit Windows (or relaunch into 64-bit PowerShell only if that behavior is fully tested), and resolve the executable from 64-bit Program Files. Capture whether the token variable existed and its exact value and restore that state in `finally`; never print the value. Invoke `miru-agent.exe provision` directly with only non-secret backend and MQTT arguments. Remove every service query, stop, start, and service-related message.

Keep `-Check` a direct read-only probe. It must return the executable's exact 0 (provisioned), 3 (not provisioned), or 1 (undetermined/error) status without translating 3 into a generic failure, and must preserve useful non-secret output. Add dependency-free script tests for elevation failing before side effects, 32-bit rejection, exact checksum parsing, local MSI metadata rejection, temp cleanup, 3010 handling, argument quoting, `-Check` passthrough, absence of token arguments/output, environment restoration on success and failure, and direct invocation without service operations. Use injectable functions or test-only command shims rather than weakening production checks.

Add `build/windows/tests/parse-scripts.ps1` in this milestone. It parses `scripts/install/install.ps1` and `scripts/install/provision.ps1` separately, with a distinct token array and error array for each call to `[System.Management.Automation.Language.Parser]::ParseFile`. It aggregates errors only after both parses, includes the source path in every diagnostic, prints both source paths and the aggregate error count, and exits 1 when that count is nonzero. Keeping the arrays separate prevents the second parse from overwriting or obscuring errors from the first.

### Milestone 3: native Windows integration

Add an integration script and a test-only WiX fragment under `build/windows/tests/`, and wire the script into `.github/workflows/ci.yml` on `windows-latest`. Reuse PR #234's Rust target and NASM setup, restore the pinned WiX SDK, build the actual x64 executable, and build MSI versions 1.0.0, 1.1.0, and a test-only 1.2.0 package whose deferred test action fails after replacement has begun. Give these fixtures a fixed allowlist of three test ProductCodes. Before doing anything, the harness must refuse to run if it finds an installed Miru product whose ProductCode is not on that allowlist. This makes cleanup deterministic on an ephemeral runner while the packages retain the production UpgradeCode and metadata needed to test the real upgrade relationship. Never run automated cleanup against an arbitrary workstation or customer machine.

Make the v3 failure executable and isolated: compile a test-only WiX fragment that defines a Type 34 executable custom action named `FailUpgradeForTest`. Its executable is `[SystemFolder]cmd.exe`, its arguments are `/d /c exit /b 1`, and it uses `Execute=deferred`, `Return=check`, and `Impersonate=no`. Schedule it after `InstallFiles` and before `InstallFinalize`, conditioned on `FAIL_UPGRADE_FOR_TEST=1`. Include the fragment only in integration packages, and make package inspection prove the production MSI contains no `FailUpgradeForTest` custom action, sequence row, or `FAIL_UPGRADE_FOR_TEST` condition. Invoke the v3 MSI with that property so failure happens after the old product has entered the transaction and replacement files have been processed. Do not use file locking or an early launch-condition failure, because neither reliably proves rollback.

On the elevated Windows runner, first create a permissive pre-existing `%ProgramData%\Miru` tree with a sentinel and a representative secret file. Add a test-only `%ProgramData%\Miru\rollback-payload.txt` component whose deterministic contents are `fixture-v1`, `fixture-v2`, and `fixture-v3` in the three integration MSIs; it must not enter the production package. Install v1 through `scripts/install/install.ps1 -FromMsi`. Assert the executable exists and record its SHA-256 hash, assert `rollback-payload.txt` contains `fixture-v1`, and verify x64 MSI and installed-product metadata, the stable UpgradeCode, exactly one Add/Remove Programs product, and no `MiruAgent` service. Do not claim a Windows executable file-version resource unless the build actually provides one. Check the effective DACL with Windows security APIs and `icacls`; create a temporary local non-admin account and run read/create probes under that identity to prove it cannot read the representative secret or create a child. Also prove SYSTEM and Administrators have inheritable full control and the permissive pre-existing ACE was removed. Deliberately add the same permissive test ACE again, run same-MSI v1 maintenance, and reuse the account and probes to prove maintenance restores the protected SYSTEM/Administrators-only DACL and denies the non-admin. Add the ACE a third time immediately before the v1-to-v2 upgrade and leave it in place for that upgrade to repair.

Run `scripts/install/provision.ps1 -Check` before provisioning. Expect exit 3 and the agent's not-provisioned output. A real successful provision requires a backend and remains a staging/manual test, but use a fake executable in the focused tests to prove argument and token handling without exposing the token.

Upgrade v1 to v2 and reuse the account and probes to prove the upgrade removes the newly added permissive ACE, restores only the protected SYSTEM/Administrators permissions, and denies non-admin read/create access. Assert the sentinel survives, `rollback-payload.txt` contains `fixture-v2`, the executable exists with the captured v2 SHA-256 hash, and only one product is registered. Attempt v1 over v2 and expect the configured downgrade message with the v2 payload and executable hash intact. Attempt the deliberately failing v3 test package and assert a nonzero MSI result, then prove the v2 executable hash, product registration, sentinel, and `fixture-v2` payload were restored rather than `fixture-v3` remaining. Finally uninstall v2 and assert the executable, product registration, installer-owned registry metadata, and test-only `rollback-payload.txt` are gone while `%ProgramData%\Miru`, its sentinel, and protected ACL remain. Retain verbose MSI logs as CI artifacts only when a packaging test fails. Always delete the temporary local user and any leftover test marker safely in `finally` after retention assertions.

Add a `-ManualProductionSmoke` mode to the integration script and document a clean Windows 10 or 11 x64 VM smoke pass in `build/windows/README.md`. This mode refuses to run unless the operator confirms the machine is a disposable clean snapshot with no installed Miru product; records Windows product name, version, build number, MSI hashes and versions, timestamps, and every assertion in a transcript path supplied by the operator; uses the production package identity and `%ProgramData%\Miru`; and covers install, maintenance, upgrade, uninstall, and reboot handling when 3010 is returned. It must repeat the no-service assertion and leave the retained ProgramData sentinel for human inspection before the VM snapshot is discarded. Full provisioning, service behavior, release download/publication, signing, and Windows Server certification remain outside this PR.

### Milestone 4: documentation, roadmap, and readiness

Rewrite `build/windows/README.md` from “scaffolding” to the buildable and validated package contract. Give exact pinned build commands, required inputs, stable-only MSI version rules, install/provision examples, state-retention behavior, security descriptor intent, checksum limitations, and the native/manual test matrix. Clearly defer service lifecycle, the GoReleaser/release-artifact/PDB build lane, and Authenticode signing.

Update `plans/active/20260910-windows-support.md` to record that PR #234's native Windows compile gate has merged and to describe PR #236 accurately as an x64 package plus safe PowerShell tooling, without service registration. Preserve later roadmap ownership for real service lifecycle/recovery/account handling, release build/publication, and signing. Update PR #236's body with the same scope and validation evidence after the branch is pushed.

Run all repository checks, inspect `Cargo.lock` and the full diff, and push the exact head only after `git status --short` is empty. Invoke the repository's `$preflight` workflow against PR #236. If preflight finds anything, fix it, commit the fix with a focused Conventional Commit, require a clean working tree before pushing the new head, and rerun preflight. After final smoke evidence and a first CLEAN result exist on the implementation SHA, update this active plan with that exact SHA and evidence, then move it to `plans/completed/` and make and push a focused plan-only closure commit as the final repository mutation. Rerun preflight on that immutable closure head; record its CLEAN result in the verified PR body and final task result, never in a further plan-only commit. Do not perform final SHA comparisons, claim CLEAN in the PR body, remove draft status, or report implementation complete until preflight returns **CLEAN** and GitHub CI is green on the exact closure head.

## Concrete Steps

Repository editing, Git, and repository-wide commands in this section run from `/home/ben/miru/workbench5/repos/agent`. Native packaging commands run from an x64 Windows checkout of the same commit at `C:\src\agent`; in GitHub Actions, use `$env:GITHUB_WORKSPACE` as that checkout root instead. Do not try to validate the `x86_64-pc-windows-msvc` binary or MSI behavior by cross-compiling from Linux.

Before editing, synchronize refs, confirm the branch, and ensure `origin/main` is an ancestor. Preserve unrelated work if the tree is unexpectedly dirty.

    cd /home/ben/miru/workbench5/repos/agent
    git fetch origin
    git switch feat/windows-msi-packaging
    git status --short --branch
    git merge-base --is-ancestor origin/main HEAD

The last command should exit 0. If it does not, merge current `origin/main` normally, resolve only in-scope conflicts, and rerun it. Do not force-push or rewrite the signed merge already on the branch.

For Milestone 1, generate the UpgradeCode once on Windows, record it in source and tests, then build and inspect real packages. The exact project property names may be adjusted to match the pinned SDK, but the public inputs remain `Version` and `BinDir`. From an x64 Windows PowerShell 5.1 session in `C:\src\agent`, run:

    Set-Location C:\src\agent
    [guid]::NewGuid().ToString().ToUpperInvariant()
    cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked --release
    dotnet restore build/windows/miru-agent.wixproj
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\package-tests.ps1 -ProjectPath build\windows\miru-agent.wixproj -BinDir target\x86_64-pc-windows-msvc\release -ArtifactsDirectory build\windows\artifacts\package-tests

Expected: the Rust build and restore succeed. The harness exits 0; leaves the valid packages exactly at `build/windows/artifacts/package-tests/v1/miru-agent-1.0.0.msi`, `build/windows/artifacts/package-tests/v2/miru-agent-1.1.0.msi`, `build/windows/artifacts/package-tests/boundaries/miru-agent-0.0.0.msi`, and `build/windows/artifacts/package-tests/boundaries/miru-agent-255.255.65535.msi`; builds, validates, and inspects all four; and reports `PASS package 1.0.0`, `PASS package 1.1.0`, `PASS ProductCodes differ`, `PASS UpgradeCode stable`, `PASS version boundaries (0.0.0, 255.255.65535)`, and `PASS invalid inputs rejected (9 cases)`. Package validation has no unsuppressed warning or ICE error. The nine expected failures are omitted `Version`, empty `Version`, omitted `BinDir`, empty `BinDir`, and versions `1.2.3-beta.1`, `1.2.3.4`, `256.0.0`, `1.256.0`, and `1.2.65536`.

Before reviewing or committing Milestone 1, update this plan's Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective as applicable with the milestone's exact test evidence and decisions. Review and commit only Milestone 1 files plus this plan checkpoint.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --check
    git diff -- build/windows/miru-agent.wxs build/windows/miru-agent.wixproj build/windows/tests plans/active/20260911-windows-msi-packaging-hardening.md
    git add build/windows/miru-agent.wxs build/windows/miru-agent.wixproj build/windows/tests plans/active/20260911-windows-msi-packaging-hardening.md
    git commit -m "feat(windows): harden MSI package contract"

For Milestone 2, parse both scripts with Windows PowerShell 5.1 and run the dependency-free focused harness. A secondary `pwsh` parse is useful when available, but it does not replace 5.1. From an x64 Windows PowerShell 5.1 session in `C:\src\agent`, run:

    Set-Location C:\src\agent
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\parse-scripts.ps1
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\script-tests.ps1

Expected: the parser names `scripts/install/install.ps1` and `scripts/install/provision.ps1`, reports `Aggregate parse errors: 0`, and exits 0. The focused harness reports every checksum, metadata, elevation, cleanup, reboot, check-exit, environment, argument, and secret-hygiene case passed, including the exact line `PASS TLS 1.2 protocol restoration (4 cases)`. Those four cases cross TLS 1.2 initially absent/present with an injected successful/throwing request, assert the exact original protocol value after each call, and use an outer `finally` to restore the harness's own original value. A canary token must not appear in captured process arguments, stdout, stderr, or retained logs.

Before reviewing or committing Milestone 2, update this plan's Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective as applicable with the milestone's exact test evidence and decisions. Review and commit only Milestone 2 files plus this plan checkpoint.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --check
    git diff -- scripts/install/install.ps1 scripts/install/provision.ps1 build/windows/tests plans/active/20260911-windows-msi-packaging-hardening.md
    git add scripts/install/install.ps1 scripts/install/provision.ps1 build/windows/tests plans/active/20260911-windows-msi-packaging-hardening.md
    git commit -m "feat(windows): harden install and provision scripts"

For Milestone 3, run the destructive integration script only on a disposable test machine from an elevated x64 Windows PowerShell session in `C:\src\agent`, and acknowledge that environment with `-ConfirmDisposableTestMachine`. CI must invoke the same entry point after building the real executable and MSIs. Manual production smoke continues to require its distinct `-ConfirmDisposableCleanVm` acknowledgement.

    Set-Location C:\src\agent
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ConfirmDisposableTestMachine

Expected: the script reports PASS for initial install, pre-existing root and `logs` ACL correction, non-admin denial, check exit 3, same-MSI v1 maintenance ACL restoration after permissive ACEs are re-added, v1-to-v2 upgrade ACL restoration after they are re-added again, downgrade rejection, failed-v3 rollback, uninstall state retention, and absence of a `MiruAgent` service. Every stage retains the representative customer-owned log file and its contents. Both repair paths leave only inheritable full-control ACEs for SYSTEM and built-in Administrators on both protected directories and make the reused non-admin read/create probes fail. The script exits 0 and verifies that no test account or installed Miru product remains.

Before reviewing or committing Milestone 3, update this plan's Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective as applicable with the milestone's exact test evidence and decisions. Review the workflow diff and commit Milestone 3 files plus this plan checkpoint.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --check
    git diff -- .github/workflows/ci.yml build/windows/tests plans/active/20260911-windows-msi-packaging-hardening.md
    git add .github/workflows/ci.yml build/windows/tests plans/active/20260911-windows-msi-packaging-hardening.md
    git commit -m "ci(windows): validate MSI install and upgrades"

For Milestone 4, update docs and the umbrella plan, run repository-wide validation, then update this plan's Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective as applicable with the milestone's exact test evidence and decisions before reviewing or committing. Stage and commit the documentation plus this plan checkpoint before reviewing the complete committed branch.

    cd /home/ben/miru/workbench5/repos/agent
    ./scripts/test.sh
    ./scripts/covgate.sh
    ./scripts/update-deps.sh
    git diff -- Cargo.lock
    ./scripts/lint.sh
    git diff --check

Expected: tests, coverage gates, dependency refresh, lint, and the whitespace check all exit 0. `Cargo.lock` has no unexplained drift; restore no file destructively—if it changes, determine why and include only required changes.

    cd /home/ben/miru/workbench5/repos/agent
    git diff -- build/windows/README.md plans/active/20260910-windows-support.md plans/active/20260911-windows-msi-packaging-hardening.md
    git add build/windows/README.md plans/active/20260910-windows-support.md plans/active/20260911-windows-msi-packaging-hardening.md
    git commit -m "docs(windows): document validated package scope"
    git status --short
    git diff --check origin/main...HEAD
    git diff --stat origin/main...HEAD
    git diff origin/main...HEAD

Expected: `git status --short` prints nothing and `git diff --check origin/main...HEAD` exits 0. The displayed stat and full diff contain all four milestone commits and their package, script, integration/CI, and documentation changes. Confirm from that displayed diff that no service, release-upload, GoReleaser/PDB, or signing implementation slipped in before pushing.

Push the completed branch without force and verify that GitHub sees the exact local SHA. Immediately before every push in this plan—including preflight fixes and the later plan-only closure commit—run `git status --short` and require it to print nothing; never push from a dirty working tree.

    cd /home/ben/miru/workbench5/repos/agent
    git status --short
    git push origin feat/windows-msi-packaging
    git rev-parse HEAD
    git ls-remote origin refs/heads/feat/windows-msi-packaging
    gh pr view 236 --json isDraft,headRefOid,mergeStateStatus,statusCheckRollup,url

The local SHA, remote branch SHA, and `headRefOid` must match. Next perform the required manual pass. Start from a clean Windows 10 or 11 x64 VM snapshot with no Miru product installed. Check out that exact pushed commit at `C:\src\agent`, build the production 1.0.0 and 1.1.0 packages as in Milestone 1, open an elevated x64 Windows PowerShell 5.1 session, and run from `C:\src\agent`:

    Set-Location C:\src\agent
    Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsBuildNumber
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ManualProductionSmoke -ConfirmDisposableCleanVm -TranscriptPath C:\Windows\Temp\miru-msi-smoke.txt
    Get-FileHash C:\Windows\Temp\miru-msi-smoke.txt -Algorithm SHA256

Expected: the transcript identifies Windows 10 or 11 x64 and the exact packages, reports all smoke assertions passed, explicitly reports that no `MiruAgent` service exists, and records whether a reboot was requested. Attach the transcript and its hash to PR #236 or paste its non-secret summary into the PR body. Inspect the retained sentinel, then revert the VM snapshot rather than teaching the script to delete retained customer state.

After the smoke pass, from `/home/ben/miru/workbench5/repos/agent`, update the PR body with this exact scope, replacing the bracketed SHA and test summary with observed values:

    cd /home/ben/miru/workbench5/repos/agent
    gh pr edit 236 --body $'## Summary\n- Build and validate a pinned WiX 5.0.2 x64 MSI for the console-capable Miru Agent.\n- Protect retained ProgramData state and prove maintenance, upgrade, downgrade rejection, rollback, and uninstall behavior.\n- Harden Windows PowerShell 5.1 install/provision tooling without putting provisioning secrets on command lines.\n\n## Scope boundaries\nThis PR does not install or control a Windows service. Service lifecycle/account/recovery, the GoReleaser/PDB release lane and artifact publication, and Authenticode signing remain in later roadmap PRs.\n\n## Validation\nBranch head: [FULL_SHA]\n[LOCAL_WINDOWS_AND_VM_TEST_SUMMARY]\nPreflight: pending; this PR remains draft until CLEAN.'

Keep the PR draft. Then, from the Codex task rooted at `/home/ben/miru/workbench5/repos/agent`, invoke the skill with the exact request `$preflight PR #236 on feat/windows-msi-packaging; do not stop until the exact pushed head is CLEAN.` Preflight must publish any fixes, watch GitHub Actions for the pushed head, and return `CLEAN`. If it does not, diagnose the reported CI job, make and commit a focused fix, require `git status --short` to print nothing immediately before pushing, rerun the manual smoke when the fix can affect MSI or script behavior, update the body SHA/evidence, and rerun `$preflight` against the new SHA.

After the smoke evidence is final and preflight first returns CLEAN, capture the implementation SHA before editing. Record that exact smoke-tested and first-CLEAN implementation SHA and concise evidence in this active plan's Progress and Outcomes & Retrospective. Update Surprises & Discoveries and Decision Log too if that implementation evidence produced a discovery or decision. Review the active-plan evidence edit, then move the plan to `plans/completed/`, review the staged rename and evidence diff across both paths, and commit it. The rename and evidence update together are the plan-only final repository mutation; this closure commit must not include implementation or other documentation changes.

    cd /home/ben/miru/workbench5/repos/agent
    TESTED_SHA=$(git rev-parse HEAD)
    git diff -- plans/active/20260911-windows-msi-packaging-hardening.md
    git mv plans/active/20260911-windows-msi-packaging-hardening.md plans/completed/20260911-windows-msi-packaging-hardening.md
    git diff --staged -- plans/active/20260911-windows-msi-packaging-hardening.md plans/completed/20260911-windows-msi-packaging-hardening.md
    git commit -m "docs(windows): record MSI hardening validation"
    git status --short
    git push origin feat/windows-msi-packaging

Expected: Progress and Outcomes & Retrospective name the full value captured in `TESTED_SHA`, distinguish manual smoke evidence from first-CLEAN implementation preflight evidence, and state that the subsequent closure commit changes only this plan. The final commit changes only this plan and leaves it at `plans/completed/20260911-windows-msi-packaging-hardening.md`. `git status --short` prints nothing immediately before the push.

The closure commit supersedes the implementation SHA for which preflight first returned CLEAN. Keep the PR draft and rerun `$preflight PR #236 on feat/windows-msi-packaging; do not stop until the exact pushed head is CLEAN.` Do not reuse the prior CLEAN result. Preflight must validate the immutable closure head without changing repository contents. Explicitly do not make a further plan-only commit merely to record post-closure CLEAN; the verified PR body and final task result are authoritative for `Preflight: CLEAN on <immutable closure SHA>`. If closure-head preflight instead finds a defect that requires a repository change, keep the PR draft, make the focused fix, rerun affected smoke and preflight validation on the new implementation head, and repeat the evidence-closure process so the eventual plan-only closure commit is again the final repository mutation.

Only after preflight returns CLEAN for that closure head, and immediately before updating the body or changing draft state, capture and compare the final local head, remote feature ref, and PR head, and display the PR check rollup:

    cd /home/ben/miru/workbench5/repos/agent
    FINAL_SHA=$(git rev-parse HEAD)
    REMOTE_SHA=$(git ls-remote origin refs/heads/feat/windows-msi-packaging | cut -f1)
    PR_HEAD_SHA=$(gh pr view 236 --json headRefOid --jq .headRefOid)
    test "$FINAL_SHA" = "$REMOTE_SHA"
    test "$FINAL_SHA" = "$PR_HEAD_SHA"
    gh pr view 236 --json headRefOid,statusCheckRollup,isDraft,url
    gh pr checks 236 --required

Expected: both comparisons exit 0, `headRefOid` equals the full value in `FINAL_SHA`, every required entry in `statusCheckRollup` is successful, `gh pr checks 236 --required` exits 0, and `isDraft` is still true. If any SHA differs or any required check is pending, failing, cancelled, or missing, do not undraft; push or wait/fix as appropriate and rerun preflight for the resulting exact head.

Update the PR body a second time, preserving the Summary, Scope boundaries, and observed smoke evidence already recorded, but replace its validation footer with the final SHA and the exact line `Preflight: CLEAN on [FULL_SHA]`. Re-run the two SHA comparisons if editing the body reveals any head change, then verify the body, head, draft status, and checks together:

    cd /home/ben/miru/workbench5/repos/agent
    gh pr edit 236 --body $'## Summary\n- Build and validate a pinned WiX 5.0.2 x64 MSI for the console-capable Miru Agent.\n- Protect retained ProgramData state and prove maintenance, upgrade, downgrade rejection, rollback, and uninstall behavior.\n- Harden Windows PowerShell 5.1 install/provision tooling without putting provisioning secrets on command lines.\n\n## Scope boundaries\nThis PR does not install or control a Windows service. Service lifecycle/account/recovery, the GoReleaser/PDB release lane and artifact publication, and Authenticode signing remain in later roadmap PRs.\n\n## Validation\nBranch head: [FULL_SHA]\n[LOCAL_WINDOWS_AND_VM_TEST_SUMMARY]\nPreflight: CLEAN on [FULL_SHA]'
    gh pr view 236 --json body,headRefOid,statusCheckRollup,isDraft,url
    gh pr checks 236 --required

Replace both `[FULL_SHA]` values with `FINAL_SHA` and retain the previously observed non-secret `[LOCAL_WINDOWS_AND_VM_TEST_SUMMARY]`; do not substitute an abbreviated SHA. Expected: the body preserves all three sections and smoke evidence, both SHA lines contain the exact full head, `headRefOid` is the same SHA, all required checks remain green, and `isDraft` remains true. Only then may the PR leave draft:

    cd /home/ben/miru/workbench5/repos/agent
    gh pr ready 236

Do not run the final command before preflight reports CLEAN for the exact closure head. The PR stays draft and the task remains incomplete merely because local commands, the manual smoke pass, or preflight on an earlier SHA succeeded.

## Validation and Acceptance

Accept the implementation only when all of the following are true, the behavioral evidence records its exact tested implementation SHA, and the later plan-only closure head contains that evidence and has its own CLEAN preflight result recorded in the verified PR body and final task result.

- Invoking `build/windows/tests/package-tests.ps1` with `-ProjectPath build\windows\miru-agent.wixproj`, `-BinDir target\x86_64-pc-windows-msvc\release`, and `-ArtifactsDirectory build\windows\artifacts\package-tests` builds the real binary into the exact v1/v2 artifact paths, builds/validates/inspects 0.0.0 and 255.255.65535 beneath the `boundaries` child, and prints all six named PASS lines, including exactly `PASS version boundaries (0.0.0, 255.255.65535)`. Omitted or empty `Version`/`BinDir` and all five named invalid versions fail as expected. WiX/ICE validation has no unexplained warning or error.
- The normal and boundary MSIs have the required ProductName, Manufacturer, exact numeric three-part ProductVersion, x64 platform metadata, and committed UpgradeCode; the 1.0.0 and 1.1.0 ProductCodes are distinct. The production MSI contains no test rollback action, sequence entry, condition, or payload.
- Installing through `install.ps1 -FromMsi` places the binary under 64-bit Program Files, registers one product, and creates no `MiruAgent` service. An MSI with the wrong identity, UpgradeCode, architecture, or version is rejected before `msiexec` runs.
- A permissive pre-existing `%ProgramData%\Miru` is corrected to a protected DACL with inheritable full control for only SYSTEM and built-in Administrators. After v1 installation, the harness re-adds the permissive ACE and proves same-MSI v1 maintenance removes it; it adds the ACE again before v1-to-v2 and proves upgrade removes it. The reused real non-admin logon cannot read the representative secret or create state after either repair, and no inherited permissive ACE remains.
- Download checksum tests accept exactly one valid 64-hex record for the exact asset filename and reject substring, duplicate, missing, malformed, and wrong-digest cases. Temporary data is cleaned; Windows PowerShell 5.1 reports `PASS TLS 1.2 protocol restoration (4 cases)` after crossing TLS 1.2 initially absent/present with injected success/throwing requests and restoring the exact original protocol value in every case; the harness restores its own original value in an outer `finally`; failure logs are retained only on failure; and exit 3010 is visibly and programmatically distinct from exit 0.
- `provision.ps1` has no token parameter, never exposes the canary token in arguments or output, restores the prior environment exactly on success and failure, invokes the executable directly, and performs no service operations. `-Check` returns exactly 0, 3, or 1 from controlled fake cases and returns 3 with not-provisioned output against a fresh real install.
- Windows PowerShell 5.1 runs `build/windows/tests/parse-scripts.ps1`, names both installer scripts, reports `Aggregate parse errors: 0`, and exits 0; any parse error from either independently parsed source is path-qualified and exits 1.
- The elevated integration test proves install v1, permissive-ACE repair during same-MSI v1 maintenance, permissive-ACE repair during upgrade v1 to v2, v1 downgrade rejection with v2 intact, failed v3 rollback to v2, and uninstall. The rollback marker reads `fixture-v1` after initial install and v1 maintenance, `fixture-v2` after upgrade, returns to `fixture-v2` after the failing v3 deferred Type 34 action, and is removed on uninstall. The recorded v2 executable SHA-256 hash is restored after rollback. The ProgramData sentinel and protected ACL survive every transition; the executable, product registration, and installer-owned registry metadata disappear on final uninstall. MSI logs are uploaded only for failed CI runs, and test cleanup removes the temporary user, installed test product, and any leftover test marker.
- `build/windows/README.md`, `plans/active/20260910-windows-support.md`, and PR #236's body say the same thing: PR #234 resolved native compile CI; PR #236 supplies a validated x64 package and safe PowerShell tooling; Windows service lifecycle/account/recovery, release/GoReleaser/PDB work, artifact publication, and Authenticode signing remain deferred.
- From `/home/ben/miru/workbench5/repos/agent`, `./scripts/test.sh`, `./scripts/covgate.sh`, `./scripts/update-deps.sh`, and `./scripts/lint.sh` all succeed, `git diff --check` is clean, and no unintended `Cargo.lock` drift remains.
- Progress and Outcomes & Retrospective record the full implementation SHA exercised by the final Windows smoke pass and first CLEAN preflight, along with the exact evidence, in a focused plan-only closure commit that is the final repository mutation. `$preflight` is then rerun and reports **CLEAN** for the immutable closure head; a CLEAN result for the superseded implementation SHA is not sufficient, and no further plan-only commit is made merely to record post-closure CLEAN.
- Most importantly, immediately before undrafting, the full SHA from `git rev-parse HEAD`, `git ls-remote origin refs/heads/feat/windows-msi-packaging`, and `gh pr view 236 --json headRefOid,statusCheckRollup` is identical; every required check is green on that closure head; and the verified PR body preserves summary, scope boundaries, and smoke evidence while recording the closure SHA in both `Branch head: [FULL_SHA]` and `Preflight: CLEAN on [FULL_SHA]`. Until all of this is true, PR #236 must remain draft and the implementation task must not be reported complete.

The clean Windows 10 or 11 VM pass is recorded in the transcript and PR evidence with OS build, MSI hashes and versions, reboot result, and install/maintenance/upgrade/uninstall observations. Successful live provisioning may remain a staging/manual follow-up because it requires a backend token; the fake-executable security tests and real `-Check` test are mandatory here.

## Idempotence and Recovery

WiX and Rust builds, parsers, focused tests, repository checks, metadata inspection, and CI runs are safe to repeat. `build/windows/tests/package-tests.ps1` recreates only the `v1`, `v2`, `boundaries`, `invalid`, and `logs` children of the supplied deterministic `build/windows/artifacts/package-tests` directory; retain those children and their logs when diagnosing a normal or boundary-package failure, then rerun the same command to regenerate them. The focused script harness always restores its original `ServicePointManager.SecurityProtocol` in an outer `finally`, including when an injected request throws. Use unique temporary directories elsewhere and an account with a test-specific name. Integration setup begins by enumerating installed products with the production UpgradeCode: it may remove only ProductCodes in the three-value fixture allowlist and must abort on every other match. It may remove only the specifically named temporary local test user. It must never remove customer state or an arbitrary Miru installation. Cleanup belongs in `finally`, while failed MSI logs are copied to the deterministic artifacts `logs` directory before temporary files are removed. The manual production smoke runs only on a disposable clean snapshot and recovers by reverting that snapshot.

Generate the production UpgradeCode exactly once. If it changes before any MSI has shipped, update source, tests, and documentation together and record the reason in Decision Log. After publication it is immutable. Commit the three fixture ProductCodes beside the integration harness and keep them stable across normal reruns. If a ProductCode must change, update its allowlist entry in the same commit and first clean any package built with the old code on the disposable runner or revert its VM snapshot; never leave an unidentifiable test install behind. Never change the production UpgradeCode to make a broken upgrade test pass.

If a normal integration assertion fails after installation—including either deliberate permissive-ACE repair assertion—collect product metadata, service absence/presence, `icacls` output, the reused non-admin probe results, the rollback marker contents, executable SHA-256 hash, and the verbose MSI log before uninstalling only an allowlisted ProductCode. If the rollback test leaves v3 installed, treat that as a product bug: clean up only the allowlisted v3 ProductCode, remove only the test-created `%ProgramData%\Miru\rollback-payload.txt` after capturing evidence, correct upgrade scheduling or the fixture, and rerun from v1. Do not mask the failure by weakening assertions.

The four milestone commits provide rollback points. Use `git revert <commit>` for a published bad milestone; do not rewrite or force-push the shared feature branch. If current `main` advances, merge `origin/main` and rerun the entire native matrix. Before closure, if preflight fails, fix the underlying issue and rerun it on the new pushed SHA; a prior green run never applies to a newer commit. After closure, do not mutate the plan merely to record CLEAN. If closure-head preflight requires a repository fix, keep the PR draft, apply and validate the fix, then repeat the implementation-evidence and closure sequence so the eventual plan-only closure is the final mutation. Ordinary MSI uninstall intentionally preserves `%ProgramData%\Miru`; test cleanup may remove only test-created sentinel data after all retention assertions pass.
