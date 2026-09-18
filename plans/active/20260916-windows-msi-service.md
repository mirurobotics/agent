# Install the Miru Agent as a Windows service from the MSI

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench2/repos/agent) | read-write | Rust workspace for the Miru device agent (crate `miru-agent` in `agent/`). All edits are WiX/MSI/PowerShell/Markdown under `build/windows/`; no Rust changes. Validation and commits happen here. |

Branch `feat/windows-msi-service` (already created and checked out; based on `origin/main` at `e29f2256`, which includes PR #242 service lifecycle, PR #244 the MSVC release lane, and PR #236 the MSI foundation). PR base `main`. This is **PR 9 — "service-aware MSI follow-up"** of the roadmap `plans/active/20260910-windows-support.md`.

This plan lives in `agent/plans/active/` because all code changes are in the `agent` repo.

## Purpose / Big Picture

Today the MSI (PR #236) installs `miru-agent.exe` to `Program Files\Miru\Agent` but does **not** register it with the Windows Service Control Manager (SCM). PR #242 made the executable service-capable: launched by the SCM with no subcommand and without `--console`, the bare exe runs as service `miru-agent` (`const SERVICE_NAME: &str = "miru-agent"` in `agent/src/windows/scm.rs`).

After this change, installing the MSI creates, starts, and (on uninstall) removes a Windows service:

- A user installs the MSI and, without any extra step, a service named `miru-agent` (display name "Miru Agent", description "Miru Config Agent") exists, is set to **Automatic** start, runs as **LocalSystem**, and has been started.
- The service is configured to **restart on failure** (bounded delay, daily reset period), matching the always-running intent of the Debian/systemd deployment.
- Upgrading stops and removes the old service before replacing files, then installs and starts the new one.
- Uninstalling stops and deletes the service; customer state under `%ProgramData%\Miru` is retained exactly as before.

Observable acceptance is the CI `windows-package` job: it builds the real MSVC binary, builds the MSI, runs the static package contract (`package-tests.ps1`) and the elevated install→maintenance→upgrade→downgrade→rollback→uninstall matrix (`integration-tests.ps1 -ConfirmDisposableTestMachine`) on a disposable `windows-latest` runner, and now asserts the service is present/configured after install/maintenance/upgrade and gone after uninstall.

Non-goals (roadmap Phase 2, explicitly deferred here): the `Miru Clients` local group and device-API discovery-directory permissions. Do not add them.

## Progress

- [x] M0 Activate plan (`docs(plans):` commit; roadmap PR 9 in-progress marker)
- [x] M1 Install the agent as a service in the MSI (`feat(windows):` — `miru-agent.wixproj` + `miru-agent.wxs`)
- [ ] M2 Assert the service in the static package contract (`feat(windows):` — `package-tests.ps1`)
- [ ] M3 Assert the service across the install lifecycle (`feat(windows):` — `integration-lib.ps1`)
- [ ] M4 Document the service behavior (`docs(windows):` — `build/windows/README.md`)
- [ ] M5 Preflight CLEAN; CI `windows-package` green on the pushed head; draft PR opened and (only then) left as draft resolved

## Surprises & Discoveries

- M1: the doc comment above `ServiceInstall` cannot describe the console flag as `--console`: `--` is illegal inside an XML comment and the offline well-formedness check rejected it. Reworded to "console flag".

## Decision Log

Design decisions resolved during authoring (2026-09-18, Benjamin Smidt):

- **Service account = LocalSystem.** The `#236` `DataDirs` components already own `%ProgramData%\Miru` (and its `logs`/`auth`/`tmp` children) with a protected DACL granting full control to `SYSTEM` (`S-1-5-18`) and built-in Administrators (SDDL `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`). LocalSystem *is* `S-1-5-18`, so the service already has full access to its runtime state with **no password to manage** and no ACL rework. The alternative — a dedicated per-service virtual account (`NT SERVICE\miru-agent`, LocalService, or a created local user like the Debian `miru` user) — would require adding that SID to every protected DACL and reconciling with the existing exact-two-ACE assertions in the harness; it is deferred. LocalSystem is authored as an explicit `Account="LocalSystem"` (it is also WiX's default) so the emitted `ServiceInstall.StartName` column is a stable, testable value.
- **Recovery mechanism = `util:ServiceConfig` (WiX Util extension), not native `ServiceConfig`.** Only the Util extension's `util:ServiceConfig` reliably configures SCM failure actions (`SC_ACTION_RESTART`). The native `wxs` `ServiceConfig` element configures `DelayedAutoStart`/`PreShutdownDelay`/`ServiceSid` and its `FailureActionsWhen` path depends on `MsiServiceConfigFailureActions`, which Microsoft documents as "not working as expected." Chosen recovery: `FirstFailureActionType`/`SecondFailureActionType`/`ThirdFailureActionType = "restart"`, `RestartServiceDelayInSeconds="10"`, `ResetPeriodInDays="1"`. In WiX v4/v5 `RestartServiceDelayInSeconds` and `ResetPeriodInDays` are **required** attributes (build error if omitted), which also keeps `TreatWarningsAsErrors=true` green. Using the extension requires a `<PackageReference Include="WixToolset.Util.wixext" Version="7.0.0" />` in the wixproj (matching the pinned `WixToolset.Sdk/7.0.0`) and the namespace `xmlns:util="http://wixtoolset.org/schemas/v4/wxs/util"` on the `<Wix>` root.
- **Delay value is a Windows-side choice, not a mirror.** The systemd unit `build/debian/miru.service` is `Type=simple` with **no** `Restart=`/`RestartSec=` — Linux resilience comes from `miru.socket` activation + `systemctl restart`, which Windows has no analogue for. The 10 s delay / 1 day reset are a sensible SCM default for the roadmap's restart-on-failure requirement; only the `Description="Miru Config Agent"` is literal parity with the unit.
- **Upgrade ordering relies on the existing `MajorUpgrade Schedule="afterInstallInitialize"`.** `RemoveExistingProducts` runs early (right after `InstallInitialize`), so the old product's `ServiceControl Stop="both" Remove="uninstall"` runs `StopServices`/`DeleteServices` for the old service **before** the new files/component install. The new component then installs the new exe, `ServiceInstall` registers the service, and `ServiceControl Start="install"` starts it. Net order: stop+delete old → replace files → install+start new. No extra scheduling is required; the static test already asserts `RemoveExistingProducts` is transactional (between `InstallInitialize` and `InstallFinalize`).
- **Service state in CI: install success is the authority for "started"; config is asserted directly; runtime `Running` is not hard-asserted.** `ServiceControl Start="install" Wait="yes"` makes `StartServices` block until the service leaves `StartPending`; per PR #242 the SCM lifecycle reports `Running` *before* the agent body runs, so a successful MSI install (exit 0/3010, already asserted) proves the service reached `Running` during install. An **unprovisioned** agent (the CI box is never provisioned) then exits shortly after (`RunOutcome::Failed → ServiceSpecific(1) → Stopped`), so a direct `State='Running'` probe would race. The harness therefore asserts the **SCM configuration** (service exists; StartMode Automatic; binary path = `Program Files\Miru\Agent\miru-agent.exe`; StartName LocalSystem; recovery = restart via `sc.exe qfailure`) — all independent of the agent's post-start runtime — and treats install success as proof of reaching Running. This is honest and non-flaky.
- **Static vs runtime division for recovery.** `package-tests.ps1` (static, offline MSI-table read) pins the deterministic install/start/stop/remove contract from the `ServiceInstall` and `ServiceControl` MSI tables and asserts the Util recovery table is *present*; the exact recovery **action values** are pinned at runtime by `sc.exe qfailure` in `integration-lib.ps1` against the actually-installed service. The Util extension emits a custom table (`Wix4ServiceConfig`) + scheduled custom actions rather than the standard MSI `ServiceConfig` table, so the static test discovers/pins that table name from the first CI build rather than guessing offline.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Read nothing outside this plan is required, but these are the load-bearing files (all paths relative to `/home/ben/miru/workbench2/repos/agent`).

**The service the exe already implements.** `agent/src/windows/scm.rs` line 32: `const SERVICE_NAME: &str = "miru-agent";` with the doc comment "must match the installer's `ServiceInstall Name`." The service is `ServiceType::OWN_PROCESS`. The bare exe (no subcommand, no `--console`) runs as a service when launched by the SCM; `--console` runs it in the foreground. Therefore the `ServiceInstall` must use `Name="miru-agent"` and **no `Arguments`**. No Rust file changes in this plan.

**The MSI today.** `build/windows/miru-agent.wxs` is a WiX v4/v5 source (`xmlns="http://wixtoolset.org/schemas/v4/wxs"`). Key structure:

- `<Package ... UpgradeCode="B5ED0336-5F14-4308-A667-3CE8CDEF7D48" Scope="perMachine" InstallerVersion="500">` with `<MajorUpgrade Schedule="afterInstallInitialize" DowngradeErrorMessage="A newer version of Miru Agent is already installed." />`.
- `ProgramFiles64Folder → Miru (INSTALLFOLDER) → Agent (AGENTFOLDER)`; `CommonAppDataFolder → Miru (MIRUDATA) → logs/auth/tmp`.
- `<ComponentGroup Id="AgentBinary" Directory="AGENTFOLDER">` contains `<Component Id="MiruAgentExe" Guid="*" Bitness="always64">` with `<File Id="miru_agent.exe" Name="miru-agent.exe" Source="$(BinDir)\miru-agent.exe" KeyPath="yes" />`. **The service elements go inside this `MiruAgentExe` component**, after the `<File>`.
- `DataDirs` components own `%ProgramData%\Miru` subtree with SDDL `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)` (SYSTEM + Administrators full control, protected+inheritable).

`build/windows/miru-agent.wixproj` (`Sdk="WixToolset.Sdk/7.0.0"`) has `<TreatWarningsAsErrors>true</TreatWarningsAsErrors>` and `<SuppressValidation>false</SuppressValidation>` — any WiX warning or ICE validation failure fails the build. It compiles `miru-agent.wxs` (and, in test builds, a fixture `.wxs`). MSBuild input-validation errors use stable `MIRUMSI1001`–`MIRUMSI1009` codes; those are for build-input validation only and this plan adds none.

**MSI service-table encodings** (the static test pins these; use bit masks to be robust):

- `ServiceInstall.ServiceType`: `ownProcess = 0x10` (16); interactive adds `0x100`.
- `ServiceInstall.StartType`: auto = `2`, demand = `3`, disabled = `4`.
- `ServiceInstall.ErrorControl`: normal = `1`, critical = `3`; the **Vital** flag folds in the high bit `msidbServiceInstallErrorControlVital = 0x8000`. So `ErrorControl="normal" Vital="yes"` emits `1 | 0x8000 = 0x8001` (32769).
- `ServiceControl.Event` bits: Start=`0x1`, Stop=`0x2`, Delete=`0x8`, UninstallStart=`0x10`, UninstallStop=`0x20`, UninstallDelete=`0x80`. `Start="install" Stop="both" Remove="uninstall"` emits `0x1 | 0x2 | 0x20 | 0x80 = 0xA3` (163). `Wait="yes"` → `Wait` column `1`.

**The test harness** (`build/windows/tests/`, all PowerShell 5.1, `Set-StrictMode -Version Latest`; small single-purpose functions with `[Parameter(Mandatory = $true)]` params, callers defined before callees, `Assert-True`/`Assert-Equal` from `MsiTest.psm1`):

- `MsiTest.psm1` — shared module: constants (`$MsiProductName="Miru Agent"`, `$MsiUpgradeCode`, `$MsiExpectedSddl`, `$MsiExpectedDirectories`, `$MsiFixtureProductCodes`), MSI-database readers (`Open-MsiDatabase`, `Get-MsiRows`, `Test-MsiTable`, `Get-MsiContract`), `Invoke-DotNetBuild`, and `Export-ModuleMember`. Add any new shared assertion here only if used by both scripts; otherwise keep it script-local.
- `package-tests.ps1` — **static, offline** contract. Builds production MSIs and reads their tables via COM (`WindowsInstaller.Installer`). `Assert-ProductionTables` (line 57) calls, among others, `Assert-NoServiceTables` (line 220) which today asserts the `ServiceInstall` and `ServiceControl` tables are **absent**. This is the function to replace. Runs in CI `windows-package`, step "Run package tests".
- `integration-lib.ps1` — **runtime, elevated** lifecycle. `Invoke-IntegrationLifecycle` (line 55) runs install → maintenance → upgrade → downgrade → rollback → uninstall against real `msiexec`. `Assert-NoService` (lines 412–415) today asserts `Get-Service -Name "MiruAgent"` is `$null` and is called in `Invoke-InstallStage` (line 150), `Invoke-MaintenanceStage` (line 426), `Invoke-UpgradeStage` (line 441), and `Invoke-UninstallStage` (line 473). These four call sites are what change. Note the current name string `"MiruAgent"` is wrong for the real service; the real name is `miru-agent`.
- `integration-tests.ps1` — the elevated entry point (`-ConfirmDisposableTestMachine`); imports the module, dot-sources `integration-lib.ps1`, runs the matrix. No change needed beyond what `integration-lib.ps1` exposes.
- `non-admin-probe.ps1`, `integration-test.wxs` — unaffected. The fixture `.wxs` only adds a rollback-payload component; fixture packages compile the real `miru-agent.wxs`, so **fixture installs also install the `miru-agent` service** (this is what the matrix exercises).

**CI** (`.github/workflows/ci.yml`):

- `windows-check` (blacksmith windows-2025, line 41): `cargo test --package miru-agent --locked` only — Rust, no WiX. Unaffected (no Rust changes).
- `windows_package_scope` (line 71) + `windows-package` (line 91): `windows-package` runs on `windows-latest`, gated by a paths filter on `build/windows/**`, `.github/workflows/ci.yml`, `.github/workflows/release.yml` for pull requests (always runs on push to `main`/`release/*`). It builds the MSVC release binary, `dotnet restore`s the pinned WiX SDK, then runs **both** `package-tests.ps1` (static) **and** `integration-tests.ps1 -ConfirmDisposableTestMachine` (the elevated matrix). GitHub `windows-latest` runners are elevated and disposable, so the destructive matrix runs there. **This job is the authority for this change.** On failure it uploads `build/windows/artifacts/package-tests/logs`.

**Local validation limits.** There is no Windows host. `./scripts/test.sh` runs `cargo test --package miru-agent` (Rust only via `scripts/lib/test.sh`) and does not compile WiX or run any `.ps1`; it stays green because no Rust changes are made, but it does **not** exercise this feature. WiX cannot be built and MSIs cannot be installed on Linux. The elevated install→upgrade→uninstall matrix — and the reboot dimension (auto-start survival across a reboot) — cannot run locally at all; the reboot dimension is not even exercised in CI (runners cannot reboot mid-job) and is asserted only indirectly via `StartType=Automatic`. **The CI `windows-package` job is the sole authority** that the WiX compiles, the MSI validates under `TreatWarningsAsErrors=true`, and the service installs/starts/stops/recovers/removes as specified.

## Plan of Work

The change is: author the three service elements into the one binary component, then update the two test layers (static contract, runtime lifecycle) to assert the service instead of its absence, then update the README. Because the `windows-package` job runs both test layers against the same MSI, the `.wxs` change (M1) and the two test updates (M2, M3) are one CI-coherent unit — the job passes only at the tip after M3 (and M4). Commit them as separate milestones for review/bisect clarity, but expect an intermediate commit's `windows-package` run to be red if bisected in isolation.

### M1 — Install the agent as a service (WiX)

`build/windows/miru-agent.wixproj`: add an `<ItemGroup>` (adjacent to the existing `<ItemGroup>` with the `<Compile>` items) with the Util extension reference:

    <ItemGroup>
      <PackageReference Include="WixToolset.Util.wixext" Version="7.0.0" />
    </ItemGroup>

`build/windows/miru-agent.wxs`:

1. Add the Util namespace to the root element so it reads:

        <Wix xmlns="http://wixtoolset.org/schemas/v4/wxs"
             xmlns:util="http://wixtoolset.org/schemas/v4/wxs/util">

2. Inside `<Component Id="MiruAgentExe" ...>`, **after** the `<File ... />` element (still inside the component), add the service install (with recovery) and the service control:

        <ServiceInstall Id="MiruAgentService"
                        Name="miru-agent"
                        DisplayName="Miru Agent"
                        Description="Miru Config Agent"
                        Type="ownProcess"
                        Start="auto"
                        ErrorControl="normal"
                        Account="LocalSystem"
                        Vital="yes"
                        Interactive="no">
          <util:ServiceConfig FirstFailureActionType="restart"
                              SecondFailureActionType="restart"
                              ThirdFailureActionType="restart"
                              RestartServiceDelayInSeconds="10"
                              ResetPeriodInDays="1" />
        </ServiceInstall>
        <ServiceControl Id="MiruAgentServiceControl"
                        Name="miru-agent"
                        Start="install"
                        Stop="both"
                        Remove="uninstall"
                        Wait="yes" />

   `Name="miru-agent"` on both elements must equal `SERVICE_NAME` in `scm.rs`. No `Arguments` on `ServiceInstall` (the bare exe runs as the service). `util:ServiceConfig` is a child of `ServiceInstall` so it configures the service being installed.

There is no way to compile this on Linux; correctness is confirmed by the M5 CI run. Watch specifically for: a WiX warning promoted to error under `TreatWarningsAsErrors=true` (e.g. an informational about the Util extension or `Vital`), an ICE validation failure, and the required-attribute error if either `RestartServiceDelayInSeconds`/`ResetPeriodInDays` is dropped.

### M2 — Assert the service in the static package contract

`build/windows/tests/package-tests.ps1`: replace `Assert-NoServiceTables` (lines 220–226) with assertions that the service tables exist and carry the authored values, and update the caller in `Assert-ProductionTables` (line 66) from `Assert-NoServiceTables $handle.Database` to the new function name. Keep every other assertion in `Assert-ProductionTables` unchanged. Follow the file's style: small functions, callers before callees, `Assert-Equal`/`Assert-True`, `Get-MsiRows`/`Test-MsiTable` from the module.

New `Assert-ServiceTables $Database` (place where `Assert-NoServiceTables` was, so it stays after its caller ordering-wise) does three checks via helper functions:

- `Assert-ServiceInstallRow`: `Test-MsiTable ServiceInstall` is true; `Get-MsiRows` over `SELECT ServiceInstall, Name, DisplayName, ServiceType, StartType, ErrorControl, StartName, Arguments, Component_, Description FROM ServiceInstall` returns exactly one row with `Name='miru-agent'`, `DisplayName='Miru Agent'`, `Description='Miru Config Agent'`, `Component_='MiruAgentExe'`, `StartName='LocalSystem'`, `Arguments` null/empty, `([int]ServiceType -band 16) -ne 0` (own-process), `[int]StartType -eq 2` (auto), `([int]ErrorControl -band 1) -ne 0` (normal) and `([int]ErrorControl -band 0x8000) -ne 0` (vital).
- `Assert-ServiceControlRow`: `Test-MsiTable ServiceControl` is true; one row over `SELECT Name, Event, Wait, Component_ FROM ServiceControl` with `Name='miru-agent'`, `Component_='MiruAgentExe'`, `([int]Wait) -eq 1`, and every required `Event` bit set: `-band 0x1` (start on install), `-band 0x2` (stop on install), `-band 0x20` (stop on uninstall), `-band 0x80` (delete on uninstall).
- `Assert-ServiceRecoveryTable`: the Util extension emits a custom table for failure actions rather than the standard MSI `ServiceConfig` table. Assert that table is present: query `_Tables` (as `Test-MsiTable` does) for a name matching `*ServiceConfig` that is **not** the empty standard `ServiceConfig`, expecting the WiX Util table (pin the exact name — expected `Wix4ServiceConfig` — from the first CI `windows-package` build log; if the offline name is uncertain, assert presence of a row whose data contains `miru-agent`). Keep this assertion loose on values: the exact restart action values are pinned at runtime in M3.

Rationale comments stay concise and present-tense (what the assertion checks now), no history.

### M3 — Assert the service across the install lifecycle

`build/windows/tests/integration-lib.ps1`: replace `Assert-NoService` (lines 412–415) with two functions and update its four call sites.

Add (callers-first ordering: define these where `Assert-NoService` was, after their callers):

- `Get-AgentService`: returns `Get-CimInstance Win32_Service -Filter "Name='miru-agent'" -ErrorAction SilentlyContinue` (gives `Name`, `StartMode`, `State`, `PathName`, `StartName`).
- `Assert-ServiceInstalled $Stage`: assert the CIM object is not null; `StartMode -eq 'Auto'` (Automatic); `StartName -eq 'LocalSystem'`; and `PathName` (trimmed of surrounding quotes) equals `$agentPath` (`Program Files\Miru\Agent\miru-agent.exe`, already computed in `Initialize-IntegrationPaths`). Then call `Assert-ServiceRecovery $Stage`.
- `Assert-ServiceRecovery $Stage`: run `$out = & sc.exe qfailure miru-agent 2>&1 | Out-String`; `Assert-Equal 0 $LASTEXITCODE`; assert `$out -match 'RESET_PERIOD'` and `$out -match 'RESTART'` (SC restart action configured). Delay/reset specifics may be matched loosely (presence of `RESTART` and a reset period is sufficient and stable across `sc.exe` output formats).
- `Assert-ServiceAbsent`: assert `Get-AgentService` is `$null`, message "miru-agent service removed".

Do **not** hard-assert `State='Running'` — an unprovisioned agent exits shortly after the SCM reports Running (see Decision Log); MSI install success with `Start="install" Wait="yes"` is the authoritative proof the service reached Running.

Update the call sites:

- `Invoke-InstallStage` (line 150): `Assert-NoService` → `Assert-ServiceInstalled "install"`; update the following `Write-Host` from "...and no service" to "...and service installed".
- `Invoke-MaintenanceStage` (line 426): `Assert-NoService` → `Assert-ServiceInstalled "maintenance"`.
- `Invoke-UpgradeStage` (line 441): `Assert-NoService` → `Assert-ServiceInstalled "upgrade"`.
- `Invoke-UninstallStage` (line 473): `Assert-NoService` → `Assert-ServiceAbsent`; update its `Write-Host` accordingly.

Leave the downgrade and rollback stages unchanged (they already assert v2 is intact and never called `Assert-NoService`; the `miru-agent` service simply survives them because those products keep v2 installed). Keep all existing coverage (install, maintenance, upgrade, downgrade rejection, rollback, uninstall, ProgramData ACLs, retained state, non-admin denial) intact.

### M4 — Document the service behavior

`build/windows/README.md`:

- Replace the paragraph at lines 6–10 ("The current executable is console-capable but is not a Windows Service Control Manager executable. Accordingly, this MSI does not create, start, stop, or remove a `MiruAgent` service...") with text stating the MSI installs `miru-agent.exe` as a Windows service named `miru-agent` (display name "Miru Agent", description "Miru Config Agent"), running as **LocalSystem**, start type **Automatic**, started on install and stopped+removed on uninstall, that major upgrades stop and delete the old service before file replacement and then install and start the new one, and that the service is configured to **restart on failure** (10 s delay, 1 day reset period).
- Update the "Package behavior" bullet list to include the service install/start/stop/remove and recovery, and the LocalSystem account.
- Update the "Validation" section: the matrix line "and the absence of a `MiruAgent` service" becomes an assertion that the `miru-agent` service is installed (Automatic, LocalSystem, correct binary path, recovery configured) after install/maintenance/upgrade and removed after uninstall.
- In the final "deferred" paragraph, remove "Windows service lifecycle, account and recovery handling" (now done). Keep deferred: Authenticode signing, WinGet publication, full live-backend provisioning, Windows Server certification, and the Phase 2 `Miru Clients` local group + device-API discovery-directory permissions.

### M5 — Preflight and CI (the authority)

Run preflight, push, open the draft PR, and iterate on the `windows-package` job until green. Details in Concrete Steps.

## Concrete Steps

Working directory for every command: `/home/ben/miru/workbench2/repos/agent`. One commit per milestone; every commit message ends with the trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

### M0 — Activate plan

1. This plan file exists at `plans/active/20260916-windows-msi-service.md`.
2. Append an in-progress marker to the PR 9 paragraph of `plans/active/20260910-windows-support.md` (starts `**PR 9 — service-aware MSI follow-up.**`, line 128): add `(in progress — \`plans/active/20260916-windows-msi-service.md\`)` to that paragraph. No other roadmap edits.
3. Commit:

        git add plans/active/20260916-windows-msi-service.md plans/active/20260910-windows-support.md
        git commit -m "docs(plans): add windows MSI service install plan" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M1 — WiX service authoring

1. Edit `build/windows/miru-agent.wixproj` and `build/windows/miru-agent.wxs` as in Plan of Work M1.
2. Sanity check (Linux, no build): the XML is well-formed and the intended elements/attributes are present.

        python3 -c "import xml.dom.minidom,sys; xml.dom.minidom.parse('build/windows/miru-agent.wxs'); print('wxs well-formed')"
        grep -n 'ServiceInstall\|ServiceControl\|util:ServiceConfig\|xmlns:util' build/windows/miru-agent.wxs
        grep -n 'WixToolset.Util.wixext' build/windows/miru-agent.wixproj

   Expect `wxs well-formed`, one `ServiceInstall`/`ServiceControl`/`util:ServiceConfig`/`xmlns:util` match set, and the `PackageReference` line. (WiX itself cannot be compiled here — that is M5/CI.)
3. `./scripts/test.sh` still ends with `test result: ok.` (Rust unchanged).
4. Commit:

        git add build/windows/miru-agent.wxs build/windows/miru-agent.wixproj
        git commit -m "feat(windows): install the agent as a service from the MSI" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M2 — Static package contract

1. Edit `build/windows/tests/package-tests.ps1` as in Plan of Work M2.
2. Syntax-parse the script without executing it (works on Linux if PowerShell is available; otherwise this is confirmed in CI):

        pwsh -NoProfile -Command "\$null = [System.Management.Automation.Language.Parser]::ParseFile('build/windows/tests/package-tests.ps1',[ref]\$null,[ref]\$errs); if (\$errs){\$errs; exit 1} else {'parse ok'}" 2>/dev/null || echo "pwsh unavailable on Linux — parse verified in CI"
        grep -n 'Assert-ServiceTables\|Assert-NoServiceTables' build/windows/tests/package-tests.ps1

   Expect no remaining `Assert-NoServiceTables` reference and the new `Assert-ServiceTables` caller + definition.
3. Commit:

        git add build/windows/tests/package-tests.ps1
        git commit -m "feat(windows): assert MSI service tables in package contract" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M3 — Runtime lifecycle harness

1. Edit `build/windows/tests/integration-lib.ps1` as in Plan of Work M3.
2. Verify the four call sites and the removed function:

        grep -n 'Assert-NoService\|Assert-ServiceInstalled\|Assert-ServiceAbsent\|Assert-ServiceRecovery\|Get-AgentService' build/windows/tests/integration-lib.ps1

   Expect zero `Assert-NoService` matches; `Assert-ServiceInstalled` at the install/maintenance/upgrade stages; `Assert-ServiceAbsent` at uninstall.
3. Commit:

        git add build/windows/tests/integration-lib.ps1
        git commit -m "feat(windows): assert the miru-agent service across the install matrix" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M4 — README

1. Edit `build/windows/README.md` as in Plan of Work M4.
2. Commit:

        git add build/windows/README.md
        git commit -m "docs(windows): document the MSI service install and LocalSystem account" \
            -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M5 — Preflight, PR, and CI (authority)

1. Preflight must be CLEAN (Rust preflight is unaffected but is the repo's gate before delivery):

        ./scripts/preflight.sh

   Expect exit 0 and the final line `Preflight clean`.
2. Push and open the draft PR (CI runs on `pull_request`, so the PR is what compiles the WiX for the first time). Write the PR body to `/tmp/windows-msi-service-pr.md` in the repo's PR style, ending with the line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`, then:

        git push -u origin feat/windows-msi-service
        gh pr create --draft --base main \
            --title "feat(windows): install the agent as a service from the MSI" \
            --body-file /tmp/windows-msi-service-pr.md
        gh pr checks --watch

3. Iterate on the `windows-package` job from its logs (download the failure-log artifact if an installer step fails). Fix WiX/PowerShell and re-push until **every** CI job — `lint`, `test`, `windows-check`, `windows-package` — is green on the pushed head. Pin the exact Util recovery table name (M2) from the first successful build if it differed from `Wix4ServiceConfig`.
4. Only once CI is green does the PR leave draft (leaving draft is the orchestrator's call).

## Validation and Acceptance

Required before the PR leaves draft or the task is reported complete:

1. **Preflight CLEAN.** `./scripts/preflight.sh` exits 0 with final line `Preflight clean`. (This gate is Rust-only and unaffected by the WiX/PowerShell changes, but must be run — never skip preflight.)
2. **`./scripts/test.sh` green.** Ends with `test result: ok.`; no Rust changed, so this only confirms no accidental breakage. It does **not** exercise the MSI/service.
3. **CI green on the pushed head — the authority.** All jobs green: `lint`, `test`, `windows-check`, and especially `windows-package`. The `windows-package` job must show:
   - the MSVC release binary builds, the WiX SDK restores, and the MSI builds and **validates** under `TreatWarningsAsErrors=true` and `SuppressValidation=false` (no promoted warning, no ICE failure) with the new `ServiceInstall`/`ServiceControl`/`util:ServiceConfig`;
   - `package-tests.ps1` prints its `PASS` lines with the new `Assert-ServiceTables` (ServiceInstall row: name/displayname/description/own-process/auto/normal+vital/LocalSystem/no-arguments; ServiceControl row: start-on-install, stop-on-both, delete-on-uninstall, wait; recovery table present);
   - `integration-tests.ps1 -ConfirmDisposableTestMachine` runs the full matrix and its stage `PASS` lines show `Assert-ServiceInstalled` after install, maintenance, and upgrade (service exists, StartMode Automatic, binary path `Program Files\Miru\Agent\miru-agent.exe`, StartName LocalSystem, `sc.exe qfailure` shows a RESTART action + reset period) and `Assert-ServiceAbsent` after uninstall (service gone), with all prior coverage — downgrade rejection, failed-upgrade rollback, ProgramData ACL repair/ownership, customer-state retention, non-admin denial — still passing.

Because there is no local Windows host, the elevated install→upgrade→uninstall matrix cannot be run locally at all, and the reboot dimension (auto-start survival across a reboot) is not exercised even in CI — it is asserted only indirectly via `StartType=Automatic`. **The CI `windows-package` job is therefore the sole authority** that the service installs, starts, recovers, upgrades, and uninstalls as specified; the task is not complete until that job (and all other CI jobs) report CLEAN/green on the pushed head.

## Idempotence and Recovery

All edits are additive/replacements and re-runnable; re-applying a milestone over a partially applied tree converges. One commit per milestone, so `git revert <sha>` unwinds one in isolation. Risky point: the `.wxs` service authoring — its correctness is unknowable until the CI `windows-package` build. If it regresses the package, reverting the M1 commit restores the #236 MSI (no service) and reverting M2/M3 restores the absence-asserting tests; the three form one CI-coherent unit, so a bisect landing between M1 and M3 will show a red `windows-package` run (expected). The runtime `sc.exe qfailure` and CIM assertions are read-only. No customer state or Rust behavior is touched.
