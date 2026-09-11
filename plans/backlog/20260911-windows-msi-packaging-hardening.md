# Harden and validate the Windows MSI packaging

This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective current as implementation proceeds. Continue through every milestone without waiting for further instructions, and make one Conventional Commit at the end of each milestone.

## Scope

| Repository | Access | Description |
|---|---|---|
| `mirurobotics/agent` | read-write | Harden PR #236's WiX MSI and PowerShell tools, add native Windows validation, and correct the agent Windows roadmap. |

This plan lives in `plans/backlog/` in the agent repository because every implementation and validation change belongs to that repository. The workbench and other Miru repositories are out of scope and must not be modified. The base branch is `main`; implementation continues the draft branch `feat/windows-msi-packaging` and PR #236.

This work deliberately does not add Windows Service Control Manager integration, service install/start/stop/recovery behavior, a release or GoReleaser build lane, artifact publication, PDB handling, or Authenticode signing. Those are separate roadmap changes. In particular, do not make the current console-capable executable pretend to be a Windows service.

## Purpose / Big Picture

After this work, a developer can build a real x64 `miru-agent.exe`, produce a validated x64 MSI with pinned tooling, and safely exercise install, maintenance, upgrade, downgrade rejection, rollback, and uninstall on Windows. The MSI installs the console-capable binary under 64-bit Program Files, protects `%ProgramData%\Miru` so only SYSTEM and built-in Administrators can access inherited customer state, and retains that state across upgrades and ordinary uninstall. It does not register a `MiruAgent` service.

The PowerShell installer refuses an unexpected or malformed MSI, verifies an exact checksum record for downloads, handles Windows PowerShell 5.1 safely, and reports a reboot-required result distinctly. The provisioning wrapper accepts its secret only through `MIRU_PROVISIONING_TOKEN`, never places it on the command line or in output, directly invokes the installed executable, and preserves the `provision --check` exit contract. CI proves the package and scripts on a native Windows runner. PR #236 remains draft until preflight reports **CLEAN** for the exact pushed branch head, where CLEAN means all required GitHub CI checks are green on that commit.

## Progress

- [ ] Confirm `feat/windows-msi-packaging` contains current `origin/main` and record the starting SHA.
- [ ] Remove every production service declaration and commit one stable UpgradeCode.
- [ ] Add the pinned x64 WiX project, required-input checks, and stable-only MSI version validation.
- [ ] Implement protected ProgramData ACLs, transactional upgrades, and state-preserving uninstall behavior.
- [ ] Build and inspect valid MSIs and reject invalid build inputs.
- [ ] Commit Milestone 1.
- [ ] Harden `scripts/install/install.ps1` and its tests.
- [ ] Harden `scripts/install/provision.ps1` and its tests.
- [ ] Parse and exercise both scripts under Windows PowerShell 5.1.
- [ ] Commit Milestone 2.
- [ ] Add deterministic install, ACL, upgrade, rollback, and uninstall fixtures.
- [ ] Run the native integration matrix and wire the same entry point into Windows CI.
- [ ] Commit Milestone 3.
- [ ] Update Windows packaging docs and the umbrella plan, then run repository-wide validation.
- [ ] Commit Milestone 4 and push the exact branch head.
- [ ] Run the clean Windows 10/11 VM smoke pass on that pushed commit and update PR #236's body with the evidence.
- [ ] Run preflight until it reports CLEAN for the pushed SHA; only then undraft or report completion.

## Surprises & Discoveries

Add short observations and supporting command or test evidence as work proceeds.

## Decision Log

Add dated implementation decisions here as they are made. Record any revision to this plan and its reason.

## Outcomes & Retrospective

Complete this section after all acceptance criteria pass. Summarize delivered behavior, the exact tested commit, and deferred roadmap work.

## Context and Orientation

The repository root is `/home/ben/miru/workbench5/repos/agent`. The branch `feat/windows-msi-packaging` contains draft PR #236. At plan creation it is based on the merge of native Windows compile CI from PR #234 and differs from `main` by four new files: `build/windows/README.md`, `build/windows/miru-agent.wxs`, `scripts/install/install.ps1`, and `scripts/install/provision.ps1`. The remote feature branch is behind this local branch, so do not assess PR readiness until the final local head is pushed.

`agent/src/main.rs` is important negative context: the Windows path waits for console control events, but it does not call `StartServiceCtrlDispatcher` or handle Windows service control messages. Therefore the current `ServiceInstall`, `ServiceControl`, and `util:ServiceConfig` declarations in `build/windows/miru-agent.wxs`, and the service stop/start logic in `scripts/install/provision.ps1`, cannot work. Remove them. The absence of a `MiruAgent` service is an acceptance condition for this PR; service lifecycle belongs to a later roadmap PR.

`build/windows/miru-agent.wxs` is the WiX source. WiX turns this XML into an MSI. `UpgradeCode` is the permanent GUID that relates major-upgrade packages and must be generated once, committed, asserted by tests, and never changed in later releases. `ProductCode` may change between package versions. MSI `ProductVersion` is not general SemVer: it must have exactly three numeric fields for this project, its first two fields must be at most 255, its third must be at most 65535, a fourth field is ignored by Windows Installer, and prerelease labels are invalid. For PR #236, support only stable release versions of the form `MAJOR.MINOR.PATCH` within those bounds and reject prerelease/build labels or extra fields. Keep the release tag/display/asset version separate from this MSI-only numeric value. This explicit restriction avoids collisions such as two beta tags mapping to the same MSI version; prerelease MSI sequencing remains deferred until a monotonic, tested mapping is designed.

An MSI major upgrade installs a new ProductCode with the same UpgradeCode. Schedule `RemoveExistingProducts` transactionally at `afterInstallInitialize`: removal and replacement then occur in the installer transaction, so a later failure can roll back to the previously installed product. Configure downgrade blocking. Reinstalling the identical MSI is maintenance mode, not a new product. Customer state under `%ProgramData%\Miru` must remain on maintenance, upgrade, rollback, and ordinary uninstall, while the executable, Add/Remove Programs entry, and installer-owned registry metadata are removed when uninstall succeeds.

The current WiX command defaults to x86 unless architecture is explicit, while `ProgramFiles64Folder` and the Rust target are x64. Add a pinned `WixToolset.Sdk` 5.0.2 project under `build/windows/` with an explicit x64 platform, Windows Installer 5.0 (`InstallerVersion=500`), validation enabled, and warnings treated as errors where the tool supports it. Make both `Version` and `BinDir` required build inputs with clear errors; never fall back to `0.0.0` or an implicit binary path. The production package must not need the WiX Util extension after service configuration is removed.

An ACL is an access-control list. WiX Util's existing `PermissionEx` defaults to appending permissions, so it does not replace inherited access on a pre-existing directory. That can expose future secrets such as `auth/private_key.pem` and `token.json`. Use the core WiX MSI-5 `PermissionEx` SDDL form instead. SDDL is the string form of a Windows security descriptor. Apply a protected DACL (disabled inheritance) with inheritable full-control ACEs for only `SY` (Local System) and `BA` (built-in Administrators), for example the semantic descriptor `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`. Confirm the exact WiX syntax by compiling it; do not weaken the descriptor to silence validation. The installer must correct a deliberately permissive pre-existing tree as well as create a secure new one.

`.github/workflows/ci.yml` already has a `windows-latest` job from PR #234 that installs Rust for `x86_64-pc-windows-msvc`, installs NASM, and runs `cargo check`. Extend or complement that native job for packaging. Build the real executable with `cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked`. Do not upload it as a release artifact and do not add signing. `scripts/test.sh`, `scripts/covgate.sh`, `scripts/update-deps.sh`, and `scripts/lint.sh` are the repository's local validation commands. `scripts/update-deps.sh` intentionally refreshes `Cargo.lock`; inspect and reject unrelated lockfile drift.

PowerShell tests must target Windows PowerShell 5.1 first because customer machines may use it; `pwsh` can be a secondary parse/run check. Use `[System.Management.Automation.Language.Parser]` to reject syntax errors. Prefer a repository-owned, dependency-free test harness under `build/windows/tests/` so CI does not depend on whatever Pester version happens to be preinstalled. Test-only fake executables, local HTTP responses, MSI fixtures, and a deliberately failing upgrade fixture may be generated in the runner's temporary directory and must never enter the release package.

## Plan of Work

### Milestone 1: package contract, security, and upgrades

In `build/windows/miru-agent.wxs`, replace the placeholder UpgradeCode with a newly generated stable GUID and put a warning next to it that changing it breaks upgrades. Remove the WiX Util namespace and all service installation, control, and recovery elements. Mark the executable component and package as x64, require build-provided `Version` and `BinDir`, set `InstallerVersion=500`, and configure `MajorUpgrade` with downgrade blocking and transactional `afterInstallInitialize` scheduling. Give installer-owned binary and registry components normal uninstall behavior, but model the ProgramData directory and its ACL so customer state is not removed on uninstall or upgrade. Do not leave installer registry values behind merely to keep data directories permanent; separate key paths/components or use a registry-free directory key path supported by WiX.

Replace appended Util permissions with core WiX `PermissionEx` using a protected, inheritable SYSTEM-and-Administrators-only DACL on `%ProgramData%\Miru` and every separately authored sensitive child directory. Ensure a repair or upgrade reapplies the descriptor to a pre-existing tree. No generic users, authenticated users, current user, or inherited parent ACE may retain access.

Add `build/windows/miru-agent.wixproj` using `WixToolset.Sdk` version 5.0.2. Make x64, warnings-as-errors, MSI/ICE validation, and required `Version` and `BinDir` inputs explicit. Add a small version-validation build target or equivalent source-generation check that accepts only stable numeric three-part versions within Windows Installer bounds. Use the SemVer string only for release-facing display or asset names; do not silently strip a `v`, prerelease label, build label, or fourth field at the MSI boundary. Add package metadata inspection to the Windows test harness so it verifies ProductName, Manufacturer, ProductVersion, UpgradeCode, ProductCode changes between versions, and x64 Summary Information.

Build at least two valid versions and invalid-version cases on Windows. Run WiX's MSI/ICE validation with warnings as errors where possible; if a platform ICE produces a documented false positive, narrowly suppress that ICE in the project and explain the evidence in `build/windows/README.md`, never suppress all validation.

### Milestone 2: safe PowerShell tools

Refactor `scripts/install/install.ps1` into small functions that can be exercised without performing a network download or installation when dot-sourced by the test harness. Check for an elevated Administrator token before any network, temporary-file, or installation work, and reject a non-x64 OS or 32-bit PowerShell host with an actionable message. For Windows PowerShell 5.1, enable TLS 1.2 for GitHub requests while preserving the process's previous protocol setting, and pass behavior equivalent to `-UseBasicParsing` where required.

Remove the `-Prerelease` switch because this PR intentionally does not support prerelease MSIs. Continue accepting a leading `v` in stable release input for download naming, but validate the resulting value against the strict MSI version contract rather than stripping any other suffix.

For downloads, create a cryptographically unique directory below the system temporary directory. Clean it in `finally` on success or handled failure. Parse the checksum file as records: exactly one line must contain exactly 64 hexadecimal digest characters and the exact MSI filename, with normal checksum-file whitespace/optional binary marker syntax but no substring filename matches. Reject zero matches, duplicate matches, malformed digests, and case variants of the filename; compare normalized digests with an invariant ordinal comparison. Document that checksums detect corruption but do not authenticate the publisher because Authenticode signing is deferred.

Before invoking `msiexec`, query the MSI through the Windows Installer API. Require Miru's exact ProductName and Manufacturer, the committed UpgradeCode, x64 platform metadata, and a valid three-part MSI ProductVersion. If `-Version` accompanies `-FromMsi`, require an exact version match after only the explicitly documented leading `v` normalization at the release-input boundary; otherwise report the metadata version being installed. Reject an unknown MSI before any state change. Invoke `msiexec` with an argument array that preserves paths containing spaces. Exit 0 for success, clearly report and propagate 3010 for success requiring reboot, and retain a verbose log only for installation failure. The error must name the retained log path.

In `scripts/install/provision.ps1`, remove the public `-Token` parameter. Normal provisioning must require a non-empty `MIRU_PROVISIONING_TOKEN` process environment variable, check elevation before changing state, reject a 32-bit host on 64-bit Windows (or relaunch into 64-bit PowerShell only if that behavior is fully tested), and resolve the executable from 64-bit Program Files. Capture whether the token variable existed and its exact value and restore that state in `finally`; never print the value. Invoke `miru-agent.exe provision` directly with only non-secret backend and MQTT arguments. Remove every service query, stop, start, and service-related message.

Keep `-Check` a direct read-only probe. It must return the executable's exact 0 (provisioned), 3 (not provisioned), or 1 (undetermined/error) status without translating 3 into a generic failure, and must preserve useful non-secret output. Add dependency-free script tests for elevation failing before side effects, 32-bit rejection, exact checksum parsing, local MSI metadata rejection, temp cleanup, 3010 handling, argument quoting, `-Check` passthrough, absence of token arguments/output, environment restoration on success and failure, and direct invocation without service operations. Use injectable functions or test-only command shims rather than weakening production checks.

### Milestone 3: native Windows integration

Add an integration script and a test-only WiX fragment under `build/windows/tests/`, and wire the script into `.github/workflows/ci.yml` on `windows-latest`. Reuse PR #234's Rust target and NASM setup, restore the pinned WiX SDK, build the actual x64 executable, and build MSI versions 1.0.0, 1.1.0, and a test-only 1.2.0 package whose deferred test action fails after replacement has begun. Give these fixtures a fixed allowlist of three test ProductCodes. Before doing anything, the harness must refuse to run if it finds an installed Miru product whose ProductCode is not on that allowlist. This makes cleanup deterministic on an ephemeral runner while the packages retain the production UpgradeCode and metadata needed to test the real upgrade relationship. Never run automated cleanup against an arbitrary workstation or customer machine.

Make the v3 failure executable and isolated: compile a test-only WiX fragment that defines a type-19 error custom action named `FailUpgradeForTest`, schedules it after `InstallFiles` and before `InstallFinalize`, and conditions it on a test-only public property such as `FAIL_UPGRADE_FOR_TEST=1`. Include that fragment only in integration packages; the production project build must prove it is absent. Invoke the v3 MSI with that property so failure happens after the old product has entered the transaction and replacement files have been processed. Do not use file locking or an early launch-condition failure, because neither reliably proves rollback.

On the elevated Windows runner, first create a permissive pre-existing `%ProgramData%\Miru` tree with a sentinel and a representative secret file. Install v1 through `scripts/install/install.ps1 -FromMsi`. Assert the executable path and file version/metadata, x64 MSI and installed-product metadata, the stable UpgradeCode, exactly one Add/Remove Programs product, and no `MiruAgent` service. Check the effective DACL with Windows security APIs and `icacls`; create a temporary local non-admin account and run read/create probes under that identity to prove it cannot read the representative secret or create a child. Also prove SYSTEM and Administrators have inheritable full control and the permissive pre-existing ACE was removed.

Run `scripts/install/provision.ps1 -Check` before provisioning. Expect exit 3 and the agent's not-provisioned output. A real successful provision requires a backend and remains a staging/manual test, but use a fake executable in the focused tests to prove argument and token handling without exposing the token.

Upgrade v1 to v2 and assert the sentinel survives, v2 is installed, only one product is registered, and the ACL remains protected. Attempt v1 over v2 and expect the configured downgrade message with v2 intact. Re-run the v2 MSI and assert maintenance succeeds without duplicate products or state loss. Attempt the deliberately failing v3 test package and assert a nonzero MSI result, then prove v2's executable/product registration and the sentinel were restored. Finally uninstall v2 and assert the executable, product registration, and installer-owned registry metadata are gone while `%ProgramData%\Miru`, its sentinel, and protected ACL remain. Retain verbose MSI logs as CI artifacts only when a packaging test fails. Always delete the temporary local user and test-generated files in `finally`.

Add a `-ManualProductionSmoke` mode to the integration script and document a clean Windows 10 or 11 x64 VM smoke pass in `build/windows/README.md`. This mode refuses to run unless the operator confirms the machine is a disposable clean snapshot with no installed Miru product; records Windows product name, version, build number, MSI hashes and versions, timestamps, and every assertion in a transcript path supplied by the operator; uses the production package identity and `%ProgramData%\Miru`; and covers install, maintenance, upgrade, uninstall, and reboot handling when 3010 is returned. It must repeat the no-service assertion and leave the retained ProgramData sentinel for human inspection before the VM snapshot is discarded. Full provisioning, service behavior, release download/publication, signing, and Windows Server certification remain outside this PR.

### Milestone 4: documentation, roadmap, and readiness

Rewrite `build/windows/README.md` from “scaffolding” to the buildable and validated package contract. Give exact pinned build commands, required inputs, stable-only MSI version rules, install/provision examples, state-retention behavior, security descriptor intent, checksum limitations, and the native/manual test matrix. Clearly defer service lifecycle, the GoReleaser/release-artifact/PDB build lane, and Authenticode signing.

Update `plans/active/20260910-windows-support.md` to record that PR #234's native Windows compile gate has merged and to describe PR #236 accurately as an x64 package plus safe PowerShell tooling, without service registration. Preserve later roadmap ownership for real service lifecycle/recovery/account handling, release build/publication, and signing. Update PR #236's body with the same scope and validation evidence after the branch is pushed.

Run all repository checks, inspect `Cargo.lock` and the full diff, push the exact head, and invoke the repository's `$preflight` workflow against PR #236. If preflight finds anything, fix it, commit the fix with a focused Conventional Commit, push the new head, and rerun preflight. Do not remove draft status and do not report implementation complete until preflight returns **CLEAN** for the current pushed SHA and GitHub CI is green on that same SHA.

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
    cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked
    dotnet restore build/windows/miru-agent.wixproj
    dotnet build build/windows/miru-agent.wixproj -c Release -p:Platform=x64 -p:Version=1.0.0 -p:BinDir="$PWD/target/x86_64-pc-windows-msvc/debug" -warnaserror
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\package-tests.ps1 -MsiPath build\windows\bin\x64\Release\miru-agent.msi -ExpectedVersion 1.0.0

Expected: the Rust build and WiX build succeed; package validation reports no unsuppressed warnings or ICE errors; metadata tests report x64, version 1.0.0, and the committed UpgradeCode. Builds omitting either property, and builds using `1.2.3-beta.1`, `1.2.3.4`, `256.0.0`, `1.256.0`, or `1.2.65536`, must fail with a clear version/input error. Adjust output paths in the test invocation to the actual deterministic project output path and document it.

Review and commit only Milestone 1 files.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --check
    git diff -- build/windows/miru-agent.wxs build/windows/miru-agent.wixproj build/windows/tests
    git add build/windows/miru-agent.wxs build/windows/miru-agent.wixproj build/windows/tests
    git commit -m "feat(windows): harden MSI package contract"

For Milestone 2, parse both scripts with Windows PowerShell 5.1 and run the dependency-free focused harness. A secondary `pwsh` parse is useful when available, but it does not replace 5.1. From an x64 Windows PowerShell 5.1 session in `C:\src\agent`, run:

    Set-Location C:\src\agent
    powershell.exe -NoProfile -Command "$e=$null; [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path 'scripts/install/install.ps1'),[ref]$null,[ref]$e) > $null; [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path 'scripts/install/provision.ps1'),[ref]$null,[ref]$e) > $null; if ($e.Count) { $e | Format-List; exit 1 }"
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\script-tests.ps1

Expected: parsing exits 0 with no errors and the harness reports every checksum, metadata, elevation, cleanup, reboot, check-exit, environment, argument, and secret-hygiene case passed. A canary token must not appear in captured process arguments, stdout, stderr, or retained logs.

Review and commit only Milestone 2 files.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --check
    git diff -- scripts/install/install.ps1 scripts/install/provision.ps1 build/windows/tests
    git add scripts/install/install.ps1 scripts/install/provision.ps1 build/windows/tests
    git commit -m "feat(windows): harden install and provision scripts"

For Milestone 3, run the complete integration script from an elevated x64 Windows PowerShell session in `C:\src\agent`. CI must invoke the same entry point after building the real executable and MSIs.

    Set-Location C:\src\agent
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release

Expected: the script reports PASS for initial install, pre-existing ACL correction, non-admin denial, check exit 3, v1-to-v2 upgrade, downgrade rejection, same-MSI maintenance, failed-v3 rollback, uninstall state retention, and absence of a `MiruAgent` service. It exits 0 and leaves no test account or installed Miru product behind.

Review the workflow diff and commit Milestone 3.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --check
    git diff -- .github/workflows/ci.yml build/windows/tests
    git add .github/workflows/ci.yml build/windows/tests
    git commit -m "ci(windows): validate MSI install and upgrades"

For Milestone 4, update docs and the umbrella plan, then run repository-wide validation in the required order.

    cd /home/ben/miru/workbench5/repos/agent
    ./scripts/test.sh
    ./scripts/covgate.sh
    ./scripts/update-deps.sh
    git diff -- Cargo.lock
    ./scripts/lint.sh
    git diff --check
    git status --short

Expected: tests, coverage gates, dependency refresh, and lint all exit 0. `Cargo.lock` has no unexplained drift; restore no file destructively—if it changes, determine why and include only required changes. Review the entire branch against current main and confirm no service, release-upload, GoReleaser/PDB, or signing implementation slipped in.

    cd /home/ben/miru/workbench5/repos/agent
    git diff --stat origin/main...HEAD
    git diff origin/main...HEAD
    git add build/windows/README.md plans/active/20260910-windows-support.md
    git commit -m "docs(windows): document validated package scope"

Push the completed branch without force and verify that GitHub sees the exact local SHA.

    cd /home/ben/miru/workbench5/repos/agent
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

Keep the PR draft. Then, from the Codex task rooted at `/home/ben/miru/workbench5/repos/agent`, invoke the skill with the exact request `$preflight PR #236 on feat/windows-msi-packaging; do not stop until the exact pushed head is CLEAN.` Preflight must publish any fixes, watch GitHub Actions for the pushed head, and return `CLEAN`. If it does not, diagnose the reported CI job, make and commit a focused fix, push, rerun the manual smoke when the fix can affect MSI or script behavior, update the body SHA/evidence, and rerun `$preflight` against the new SHA. Only after CLEAN may the PR leave draft:

    cd /home/ben/miru/workbench5/repos/agent
    gh pr ready 236

Do not run the final command before the CLEAN result. The task is not complete merely because local commands and the manual smoke pass.

## Validation and Acceptance

Accept the implementation only when all of the following are true on the same pushed commit.

- A pinned WiX 5.0.2 project builds the real `x86_64-pc-windows-msvc` binary into an x64 MSI when and only when valid `Version` and `BinDir` inputs are supplied. WiX/ICE validation has no unexplained warning or error.
- The MSI's ProductName, Manufacturer, stable UpgradeCode, numeric three-part ProductVersion, x64 platform metadata, and ProductCode behavior match the documented contract. Stable versions within MSI bounds work; prerelease/build labels, fourth fields, and out-of-range parts fail before packaging.
- Installing through `install.ps1 -FromMsi` places the binary under 64-bit Program Files, registers one product, and creates no `MiruAgent` service. An MSI with the wrong identity, UpgradeCode, architecture, or version is rejected before `msiexec` runs.
- A permissive pre-existing `%ProgramData%\Miru` is corrected to a protected DACL with inheritable full control for only SYSTEM and built-in Administrators. A real non-admin logon cannot read the representative secret or create state. No inherited permissive ACE remains.
- Download checksum tests accept exactly one valid 64-hex record for the exact asset filename and reject substring, duplicate, missing, malformed, and wrong-digest cases. Temporary data is cleaned, Windows PowerShell 5.1 uses TLS 1.2 compatibly, failure logs are retained only on failure, and exit 3010 is visibly and programmatically distinct from exit 0.
- `provision.ps1` has no token parameter, never exposes the canary token in arguments or output, restores the prior environment exactly on success and failure, invokes the executable directly, and performs no service operations. `-Check` returns exactly 0, 3, or 1 from controlled fake cases and returns 3 with not-provisioned output against a fresh real install.
- The elevated integration test proves install v1, same-MSI maintenance, upgrade v1 to v2, v1 downgrade rejection with v2 intact, failed v3 rollback to v2, and uninstall. The ProgramData sentinel and protected ACL survive every transition; the executable, product registration, and installer-owned registry metadata disappear on final uninstall. MSI logs are uploaded only for failed CI runs, and test cleanup removes the temporary user and installed test product.
- `build/windows/README.md`, `plans/active/20260910-windows-support.md`, and PR #236's body say the same thing: PR #234 resolved native compile CI; PR #236 supplies a validated x64 package and safe PowerShell tooling; Windows service lifecycle/account/recovery, release/GoReleaser/PDB work, artifact publication, and Authenticode signing remain deferred.
- From `/home/ben/miru/workbench5/repos/agent`, `./scripts/test.sh`, `./scripts/covgate.sh`, `./scripts/update-deps.sh`, and `./scripts/lint.sh` all succeed, `git diff --check` is clean, and no unintended `Cargo.lock` drift remains.
- Most importantly, `$preflight` reports **CLEAN** for PR #236's exact pushed branch head. CLEAN explicitly means every required GitHub CI check is green on the SHA returned by both `git rev-parse HEAD` and `gh pr view 236 --json headRefOid`. Until this is true, PR #236 must remain draft and the implementation task must not be reported complete.

The clean Windows 10 or 11 VM pass is recorded in the transcript and PR evidence with OS build, MSI hashes and versions, reboot result, and install/maintenance/upgrade/uninstall observations. Successful live provisioning may remain a staging/manual follow-up because it requires a backend token; the fake-executable security tests and real `-Check` test are mandatory here.

## Idempotence and Recovery

WiX and Rust builds, parsers, focused tests, repository checks, metadata inspection, and CI runs are safe to repeat. Use unique temporary directories and an account with a test-specific name. Integration setup begins by enumerating installed products with the production UpgradeCode: it may remove only ProductCodes in the three-value fixture allowlist and must abort on every other match. It may remove only the specifically named temporary local test user. It must never remove customer state or an arbitrary Miru installation. Cleanup belongs in `finally`, while failed MSI logs are copied to a known artifact directory before temporary files are removed. The manual production smoke runs only on a disposable clean snapshot and recovers by reverting that snapshot.

Generate the production UpgradeCode exactly once. If it changes before any MSI has shipped, update source, tests, and documentation together and record the reason in Decision Log. After publication it is immutable. Commit the three fixture ProductCodes beside the integration harness and keep them stable across normal reruns. If a ProductCode must change, update its allowlist entry in the same commit and first clean any package built with the old code on the disposable runner or revert its VM snapshot; never leave an unidentifiable test install behind. Never change the production UpgradeCode to make a broken upgrade test pass.

If a normal integration assertion fails after installation, collect product metadata, service absence/presence, `icacls` output, and the verbose MSI log before uninstalling only an allowlisted ProductCode. If the rollback test leaves v3 installed, treat that as a product bug: clean up only the allowlisted v3 ProductCode, correct upgrade scheduling or the fixture, and rerun from v1. Do not mask the failure by weakening assertions.

The four milestone commits provide rollback points. Use `git revert <commit>` for a published bad milestone; do not rewrite or force-push the shared feature branch. If current `main` advances, merge `origin/main` and rerun the entire native matrix. If preflight fails, fix the underlying issue and rerun it on the new pushed SHA; a prior green run never applies to a newer commit. Ordinary MSI uninstall intentionally preserves `%ProgramData%\Miru`; test cleanup may remove only test-created sentinel data after all retention assertions pass.
