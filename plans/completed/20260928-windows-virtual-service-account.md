# Run the Windows agent service as virtual account NT SERVICE\miru-agent

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (GitHub `mirurobotics/agent`) | read-write | WiX installer source and project, PowerShell MSI test harness, and Markdown docs under `build/windows/`, plus one sentence in `ARCHITECTURE.md`. No Rust changes. |

This plan lives in `agent/plans/` because every change is in the agent repo. Work on branch `feat/windows-virtual-service-account` (already created from `main`); the PR targets `main`. All paths below are relative to the agent repo root, and all commands run from that root unless stated otherwise.

## Purpose / Big Picture

Today the Windows MSI installs the `miru-agent` service to run as `LocalSystem`, which has full control of the machine. After this change the service runs as the *virtual account* `NT SERVICE\miru-agent`: a per-service identity that Windows creates automatically from the service name, with no password, no admin rights, and access only to what ACLs grant it. The installer grants it full control of `%ProgramData%\Miru` (and its `logs`, `auth`, `tmp` children); customers grant it access to their own config-deploy and file-rule folders with `icacls`. This matches Linux, where the agent runs as the low-privilege `miru` user.

Observable result: after installing the MSI, `Get-CimInstance Win32_Service -Filter "Name='miru-agent'"` shows `StartName = NT SERVICE\miru-agent`, the running process's owner is `NT SERVICE\miru-agent`, the service writes its log under `%ProgramData%\Miru\logs`, and non-admin users are still denied access to `%ProgramData%\Miru`.

## Progress

- [x] Milestone 1: installer account, service SID type, SDDL, static package tests. (WIX1149 suppressed in the wixproj; package tests implemented; CI validation pending in Milestone 4.)
- [x] Milestone 2: integration-test assertions for ACLs and runtime service identity. (Implemented; CI validation pending in Milestone 4.)
- [x] Milestone 3: documentation.
- [x] Milestone 4: push, preflight `CLEAN`, manual test plan in PR description. (CI green on f6da1f1, including `windows-package`; PR #263 open and ready for review.)
- [x] Milestone 5: refine pass (2026-09-29): review findings F1–F8 applied and F10 deferred as a follow-up; later refine iterations are recorded in the Decision Log. (CI green on 7b3e5d0; final head re-verified at delivery.)
- [x] Milestone 6 (2026-09-29): rollback custom actions reapply the service settings after a failed major upgrade; see the Decision Log. (Implemented. The first placement failed the MSI build with ICE63 on cd76442; the actions moved to the removed version's uninstall, see the Decision Log, rollback placement. CI validation pending.)

## Surprises & Discoveries

(Add entries as work proceeds.)

- 2026-09-28: Windows Installer does not propagate a re-applied `MsiLockPermissionsEx` descriptor to files that already exist. In CI the install stage passed, but in the maintenance stage, after `Add-PermissiveAces` loosened the directories and the REINSTALL re-applied the protected descriptor (the directory itself had the correct 3 ACEs), a representative file created by the install stage still had 6 ACEs. Evidence: `ASSERT: C:\ProgramData\Miru\representative-install-<guid>.txt has only trusted inherited ACEs (expected '3', actual '6')`.
- 2026-09-29: WiX 7 compiles one core `ServiceConfig` element into one `MsiServiceConfig` row per setting: `<id>.SS` (ConfigType 5) for `ServiceSid` and `<id>.RP` (ConfigType 6) for `RequiredPrivilege` children, which are joined with `[~]`.
- 2026-09-29: CI (runs 36600097110 and 36601581732) showed Windows Installer's rollback of a failed major upgrade recreates the miru-agent service from `ServiceInstall` alone: `sc.exe qfailure` lists no failure actions, `qsidtype` reports `NONE`, and `qprivs` lists no privileges (so the full default service set, including `SeImpersonatePrivilege`). Runtime identity still holds, because a virtual account's token user is the service SID whatever the SID type. The rollback stage logs these values and asserts only core configuration and runtime identity, then repairs v2 and asserts the full configuration. (Superseded 2026-09-29: rollback custom actions now reapply the settings and restart the service, and the rollback stage asserts the full configuration directly; see the Decision Log, rollback custom action.)
- 2026-09-29: The cause is that `RemoveExistingProducts`' `DeleteServices` records only the basic `CreateService` definition for rollback. `MsiConfigureServices` (SID type and required privileges) and the Util extension's `Wix4ExecServiceConfig` (failure actions) do not run during the old product's uninstall, so rollback has nothing to restore them from. Rollback also restarts the old service (undoing `StopServices`) inside the rollback of `RemoveExistingProducts`, before any action sequenced ahead of it, and the SID type and required privileges apply only to a process started after they change.
- 2026-09-29: CI on cd76442 failed the MSI build (`TreatWarningsAsErrors`) with `miru-agent.wxs(32): error WIX1076: ICE63: Some action falls between InstallInitialize and RemoveExistingProducts.` ICE63 requires `RemoveExistingProducts` to follow `InstallInitialize` directly when it is scheduled in that window, so the `Before="RemoveExistingProducts"` chain, which WiX numbered between the two, is invalid. See the Decision Log, rollback placement.

## Decision Log

(Add entries as work proceeds. Design choices made while authoring are under "Design choices" in Context and Orientation.)

- 2026-09-28: Suppress WiX warning WIX1149 (`ServiceConfigFamilyNotSupported`) in `build/windows/miru-agent.wixproj` via `<SuppressSpecificWarnings>1149</SuppressSpecificWarnings>`. The WiX 7 compiler emits it unconditionally for every core `ServiceConfig` element, and the project sets `TreatWarningsAsErrors=true`, so the build would otherwise fail. The core element is the only way to set the service SID type (`util:ServiceConfig` has no SID-type attribute), and `package-tests.ps1` verifies the compiled `MsiServiceConfig` row. The production, CI package-test, and integration-fixture builds all use this one project, so one suppression covers them.
- 2026-09-28: `Assert-ServiceSidConfig` also asserts that `MsiConfigureServices` is scheduled exactly once in `InstallExecuteSequence`, after `InstallServices` and before `StartServices` (sequence number and condition not pinned). Without that action the `MsiServiceConfig` row is inert. It asserts both `Event` bits, install (0x1) and reinstall (0x4), because the element sets `OnInstall` and `OnReinstall`. (Renamed Assert-MsiServiceConfig on 2026-09-29; now two rows, see F2.)
- 2026-09-28: The primary runtime identity check is `Win32_Process.GetOwnerSid` on the service process, compared with `$MsiServiceSid`; it checks the token user SID directly and does not depend on name lookup. `GetOwner` (`NT SERVICE` / `miru-agent`) is kept as a readable secondary check.
- 2026-09-28: Only the install-stage `waiting for provisioning` log line is decisive on its own: no `miru.log*` exists yet, so the service must create one through its own ACE. In the upgrade stage `Add-PermissiveAces` also grants Everyone on the existing log file, so a new line there proves nothing alone. The upgrade stage was meant to rely on an existing-file inheritance check in `Assert-ProtectedState`; that check was dropped (see the next entry), so the upgrade-stage log line is supporting rather than decisive evidence, and the install stage remains the decisive proof.
- 2026-09-28: Applied the Milestone 2 step 5 fallback after CI showed existing files keep stale ACEs (see Surprises & Discoveries). Removed the representative-file `Assert-InheritedProtection` loop from `Assert-ProtectedState`, restoring its pre-branch shape; fresh representative files are still checked when created. Removed the Validation sentence about files created earlier. Manual test 5 now runs the README reset step before checking `private_key.pem`. Added a README note under "Service account and folder access": after upgrading from a LocalSystem build, reset existing files, not the four directories, to inherit. Superseded in part on 2026-09-29: the README note and the manual-test-5 reset step were removed (see the refine-pass entries); removing the representative-file loop stands.
- 2026-09-28: Superseded 2026-09-29 (README migration steps removed; see the refine-pass entries): Widened the README upgrade reset note. Resetting only files was not enough: the agent creates `resources\` (with `config_instances\` and `contents\`) and `events\`, which inherited only SYSTEM and Administrators from the LocalSystem-era root. On a version change, upgrade reconcile's `disk::setup::reset` deletes both with `remove_dir_all`, which the virtual account cannot do inside them, so reconcile retried forever. The note now resets every existing file and folder except the root and its `logs`, `auth`, and `tmp` children, parents before children, then restarts the service. Because `VERSION` comes from `CARGO_PKG_VERSION`, not the MSI version, manual test 5 now forces reconcile by overwriting `agent_version`.
- 2026-09-28: Superseded 2026-09-29 (README migration steps removed; see the refine-pass entries): The upgraded service panics in `logs::init` (tracing-appender 0.2.5 `RollingFileAppender::new` calls `expect`) when the current-hour `miru.log` file predates the upgrade and lacks the service ACE; the panic aborts the process right after `scm.rs` reports Running, so the MSI's StartServices (`Wait="yes"`) can fail with 1920 and roll back the major upgrade, or the service crash-loops. The README now grants the service SID on existing items before the upgrade and resets them afterwards; the installer replaces the temporary grant on the four protected directories with their exact three-ACE descriptor.
- 2026-09-29 (refine): F1: removed the README LocalSystem upgrade procedure and the hardcoded SID from the README. No release shipped the MSI, the procedure was untested in CI, and its post-upgrade recursive `icacls /reset` ran as an administrator over service-writable folders, so a planted junction would redirect it. Pre-release LocalSystem installs are replaced instead (uninstall, delete `%ProgramData%\Miru`, reinstall, reprovision). The README points to `sc.exe showsid miru-agent`.
- 2026-09-29 (refine): F2: reversed the `RequiredPrivilege` deferral. `<RequiredPrivilege Name="changeNotify" />` adds an `MsiServiceConfig` type-6 row `SeChangeNotifyPrivilege`, so the token loses `SeImpersonatePrivilege` and the other default service privileges. The agent needs none of them (outbound TLS, file I/O, the SCM dispatcher); `SeChangeNotifyPrivilege` is kept for bypass-traverse checking. Package tests pin both rows; the integration run asserts `sc.exe qprivs`.
- 2026-09-29 (refine): F3: each protected directory grants the service SID `FRFWFX` on the directory itself plus an inherit-only `FA` ACE, so the service cannot delete, rename, re-permission, or take ownership of those directories (it only deletes children such as `resources\` and `events\`, using DELETE on the child). Files created inside still get exactly three inherited full-control ACEs. This does not stop a compromised service from turning a protected directory into a mount point that a later SYSTEM repair or upgrade would follow when `MsiLockPermissionsEx` reapplies the descriptor by path. The inherited `FA` gives the service DELETE on every child, so it can empty `logs\` or `auth\` itself, and `FW` includes the `FILE_WRITE_DATA` and `FILE_WRITE_ATTRIBUTES` rights that `FSCTL_SET_REPARSE_POINT` needs on an empty directory; mount points need no privilege. `tmp\` is exposed too: `provision`, run as an administrator, deletes it, and the service can then create a new `tmp` in the root, which it owns, and make that a mount point. Exploiting this needs an already-compromised service and an administrator-initiated repair or upgrade. Hardening is a follow-up (see Outcomes & Retrospective).
- 2026-09-29 (refine): F4: the downgrade-rejection and rollback stages assert the installed service; rollback also asserts runtime identity. (Narrowed for rollback by the F4 contingency entry below; restored 2026-09-29, rollback custom action: rollback asserts the full configuration again.)
- 2026-09-29 (refine): F5: the install stage asserts no agent log exists before install, removes leftover `miru.log*` files during setup, and checks the new log file's inherited ACEs.
- 2026-09-29 (refine): F6: the `windows-package` pull-request filter includes agent and workspace code and the Cargo and toolchain configuration. (Reverted 2026-09-29 at the maintainer's request: the pull-request filter stays limited to `build/windows/**` and the CI/release workflows to save Windows runner time. `windows-package` still runs on every push to `main` and `release/*` and in the release workflow's CI call, so it still gates releases; restricted-account regressions from agent changes surface there rather than on the pull request.)
- 2026-09-29 (refine): F7: the README deploy recipe removes the inherited Authenticated Users Modify ACE on `C:\srv` and `C:\srv\miru`.
- 2026-09-29 (refine): F8: rationale comments consolidated; the WIX1149 reason lives next to `<ServiceConfig>`.
- 2026-09-29 (refine): F4 contingency applied: Assert-ServiceInstalled split into Assert-ServiceCoreInstalled plus extended checks; rollback asserts the core checks and runtime identity only. (Superseded 2026-09-29, rollback custom action: Assert-ServiceCoreInstalled was folded back into Assert-ServiceInstalled, and rollback asserts the full configuration.)
- 2026-09-29 (refine, iteration 3): After rollback, the harness logs rather than asserts the dropped failure actions, SID type, and privileges; asserting a known Windows Installer behavior would pin the quirk, not the product. It instead repairs the restored v2 (`REINSTALL=ALL REINSTALLMODE=vomus`, the maintenance-stage shape) and asserts the full service configuration, proving the remedy the README now documents under Package behavior. `msiexec /fa` was not used: it needs the ProductCode and force-overwrites the running executable. (Superseded 2026-09-29, rollback custom action: the harness asserts the full configuration right after rollback; the log dump, the post-rollback repair, and the README repair instructions were removed. Repair itself stays covered by the maintenance stage.)
- 2026-09-29 (rollback custom action): Considered `MajorUpgrade Schedule="afterInstallExecute"`, so a failed upgrade never removes the old service, and rejected it. That schedule installs the new version over the old one before removing it, which relies on strict component rules (stable component GUIDs and key paths across versions); a future layout mistake would then fail silently, after an upgrade that reports success, instead of failing loudly. The schedule stays `afterInstallInitialize`.
- 2026-09-29 (rollback custom action): Added four rollback-only custom actions to `build/windows/miru-agent.wxs`: `MiruRollbackServiceFailureActions` (`sc.exe failure miru-agent reset= 86400 actions= restart/10000/restart/10000/restart/10000`), `MiruRollbackServiceSidType` (`sc.exe sidtype miru-agent unrestricted`), `MiruRollbackServicePrivileges` (`sc.exe privs miru-agent SeChangeNotifyPrivilege`), and `MiruRollbackServiceRestart`. Each is the Util extension's `WixQuietExec64` entry in `Wix4UtilCA_X64` with `Execute="rollback" Impersonate="no" Return="ignore"` (CustomAction Type 3393), and a `SetProperty` (type 51, `Set<action>`) passes its command line as CustomActionData. They are chained with `Before` between `InstallInitialize` and `RemoveExistingProducts`, conditioned on `WIX_UPGRADE_DETECTED`, so rollback runs them after the old service is recreated. Each ignores failure (for example, a missing service) so the rest of the rollback completes. (Placement and condition superseded 2026-09-29, rollback placement: ICE63 rejected the chain before `RemoveExistingProducts`; the actions now run in the removed version's uninstall, before `StopServices`, conditioned on `UPGRADINGPRODUCTCODE AND REMOVE~="ALL"`.)
- 2026-09-29 (rollback custom action): Rollback restarts the old service inside the rollback of `RemoveExistingProducts`, before the rollback actions run, and the SID type and required privileges apply only to a newly started process. So the restored service would keep the default token, including `SeImpersonatePrivilege`, until its next restart, even though `sc.exe qprivs` and `qsidtype` would look right. `MiruRollbackServiceRestart` is sequenced first, so it runs last in rollback, and runs `cmd.exe /d /s /c "net.exe stop miru-agent && net.exe start miru-agent"` (full `System64Folder` paths). `net.exe` waits for each transition, and `&&` restarts only a service that was running, so a service that was stopped before the upgrade stays stopped. The integration harness proves the restart by reading the running service process's token and asserting it holds no privilege outside the required set; it applies the same check after install and upgrade.
- 2026-09-29 (rollback custom action): Single source of truth. `miru-agent.wxs` defines the service name, failure action, restart delay, reset period, SID type, and required privilege once as preprocessor variables, used by `ServiceInstall`, `ServiceControl`, `util:ServiceConfig`, the core `ServiceConfig` (which accepts the full name `SeChangeNotifyPrivilege`; the WiX 7 compiler passes unknown names through), and the `sc.exe` command lines. The preprocessor cannot convert units, so the delay (seconds and milliseconds) and reset period (days and seconds) are defined in pairs. `MsiTest.psm1` exports one spec, `$MsiServiceSettings`; `package-tests.ps1` checks the `Wix4ServiceConfig` row, both `MsiServiceConfig` rows, and every rollback command line against it (rendering the commands with the unit conversions), so drift between the install-time and rollback settings, or a unit mistake in either pair, fails the package test. The integration harness parses `sc.exe qfailure`, `qsidtype`, and `qprivs` against the same spec.
- 2026-09-29 (rollback placement): The rollback actions now belong to the version being removed rather than the version being installed. In `InstallExecuteSequence` they are chained with `Before="StopServices"` and conditioned on `UPGRADINGPRODUCTCODE AND REMOVE~="ALL"` (the setters too), so they run only in the uninstall that a newer version's `RemoveExistingProducts` starts; Windows Installer sets `UPGRADINGPRODUCTCODE` in that session, and `REMOVE` is `ALL` because the `Upgrade` row's `Remove` column is empty. WiX numbers the chain down from `StopServices` (1900): `SetMiruRollbackServiceRestart` 1892, `MiruRollbackServiceRestart` 1893, `SetMiruRollbackServiceFailureActions` 1894, `MiruRollbackServiceFailureActions` 1895, `SetMiruRollbackServiceSidType` 1896, `MiruRollbackServiceSidType` 1897, `SetMiruRollbackServicePrivileges` 1898, `MiruRollbackServicePrivileges` 1899, after `RemoveExistingProducts` (scheduled directly after `InstallInitialize`, 1500) and `UnpublishFeatures` (1800). That satisfies ICE63, and ICE77 (in-script actions between `InstallInitialize` and `InstallFinalize`). In the old version's uninstall script the actions precede `StopServices` and `DeleteServices`, so a rollback, which replays the script in reverse, recreates the service, restarts it, and then runs the actions (privileges, SID type, failure actions, and the restart last). Each version now reapplies its own settings, which is more correct than the installing version's. A plain uninstall, a repair, and the installing version's session do not run them; after a successful upgrade the old version's rollback operations are discarded. Caveat: this protects only upgrades from versions that contain these actions; no MSI has shipped, so every installable version does. The integration rollback stage exercises v2's actions, since v2 is built from this branch.

## Outcomes & Retrospective

The MSI installs miru-agent as `NT SERVICE\miru-agent` with an unrestricted service SID. CI on f6da1f1 proved that the hardcoded SID matches Windows' derivation and lets the service create and write its log. The refine pass (Milestone 5) adds a `SeChangeNotifyPrivilege`-only token, non-destructive directory rights, rollback and downgrade service checks, and a stricter deploy recipe.

Lessons: Windows Installer does not re-propagate a reapplied descriptor to existing children, so an in-place upgrade from a Local System build needs manual ACL repair; because none shipped, the migration was dropped rather than supported. The core `ServiceConfig` needs WIX1149 suppressed.

Follow-ups: tracing-appender panics when it cannot open the log file under the SCM (F10, separate task); junction hardening for the protected directories, which a compromised service can empty (`logs\`, `auth\`) or recreate after `provision` deletes it (`tmp\`) and then turn into a mount point (for example, an installer-owned, permanent sentinel file in each directory whose DACL grants only SYSTEM and Administrators, plus provisioning deleting only the key files it created in `tmp\` rather than the directory, with matching Linux provisioning tests and Windows harness updates); after a failed upgrade, Windows Installer's rollback restores the service without failure actions, with SID type `NONE` and the default service privileges, until the customer repairs it (README, Package behavior; verified by the rollback stage); an installer-side fix, for example evaluating `MajorUpgrade Schedule="afterInstallExecute"` so a failed upgrade never removes the old service, is a follow-up (superseded 2026-09-29: rollback custom actions reapply the settings and restart the service; `afterInstallExecute` was rejected; see the Decision Log, rollback custom action); Authenticode signing; WinGet publication.

## Context and Orientation

**Windows service accounts.** The Service Control Manager (SCM) starts each service under an account, stored as the service's `StartName`. `LocalSystem` (SID `S-1-5-18`, SDDL alias `SY`) is all-powerful. A *virtual account* `NT SERVICE\<service name>` needs no password and exists only while the service is installed. Its identity is the *service SID*: `S-1-5-80-` followed by five decimal numbers. Windows computes it as the SHA-1 of the service name, uppercased and encoded UTF-16LE, read as five little-endian unsigned 32-bit integers. `sc.exe showsid <name>` prints the same value. A service's *SID type* (`sc.exe qsidtype`) controls whether its SID is added to the process token; `UNRESTRICTED` adds it. For a virtual account the service SID is also the token's user, and Windows may already use `UNRESTRICTED`; this plan still sets it explicitly (see Design choices). On the network a virtual account acts as the computer account; the agent only makes outbound HTTPS/MQTT connections, so that does not matter here.

**SDDL** is the text form of a security descriptor. `O:SY` makes SYSTEM the owner; `D:P` starts a *protected* DACL (it does not inherit from the parent); `(A;OICI;FA;;;X)` allows (`A`) full access (`FA`) to principal `X`, inherited by files (`OI`) and subfolders (`CI`). `BA` means built-in Administrators (`S-1-5-32-544`).

**Installer.** `build/windows/miru-agent.wxs` is a WiX v4-schema source built with `WixToolset.Sdk/7.0.0` and `WixToolset.Util.wixext` 7.0.0 (`build/windows/miru-agent.wixproj`, which sets `TreatWarningsAsErrors=true` and runs MSI validation). Relevant parts:
- In component group `AgentBinary`, element `ServiceInstall Id="MiruAgentService" Name="miru-agent" ... Account="LocalSystem"`, which has a `util:ServiceConfig` child for restart-on-failure.
- In component group `DataDirs`, four components (`MiruDataDir`, `MiruLogsDir`, `MiruAuthDir`, `MiruTmpDir`) for directories `MIRUDATA` (`%ProgramData%\Miru`), `MIRULOGS`, `MIRUAUTH`, and `MIRUTMP`. Each has `<PermissionEx Sddl="O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)" />`, which compiles to rows in the `MsiLockPermissionsEx` table.

Each child DACL is protected, so every folder needs the new ACE; it cannot inherit it from `MIRUDATA`. The ACE must also be identical in all four folders because the agent moves files between them: provisioning writes keys in `tmp\` and renames them into `auth\` (`agent/src/disk/setup.rs`), and a renamed file keeps its original ACL. The agent also creates `resources\`, `events\`, and several root files (`agent/src/disk/layout.rs`); these inherit from `MIRUDATA`.

**Why no admin rights are needed.** `verify_effective_user` in `agent/src/privilege/mod.rs` is a Windows stub that leaves access control to the installer's service account and NTFS ACLs. File-mode helpers `mode_open_options` and `apply_mode` in `agent/src/filesys/files.rs` do nothing on Windows. The local device API is disabled on Windows (`agent/src/app/run.rs`). Outside `%ProgramData%\Miru`, the agent touches only customer folders:
- Config deploys (`agent/src/deploy/filesys.rs`) write `miru.backup.<name>` beside the target and use the `atomicwrites` crate. On Windows that crate creates a temporary subdirectory inside the target's parent directory and replaces the target with `MoveFileExW`. The target folder therefore needs Modify access. A denial surfaces as a `WriteAccessDeniedErr` deploy error, or `BackupAccessDeniedErr` when the target already exists and its backup copy is denied.
- File-rule scanning, uploading, and retention delete (`agent/src/data_uploads/`) read and delete files in source folders.

**Runtime behavior useful for tests.** On service start, `run_agent` in `agent/src/main.rs` first initializes logging. `tracing_appender::rolling::hourly` opens `%ProgramData%\Miru\logs\miru.log.<date-hour>` and panics if it cannot. Next, `await_activation` (`agent/src/app/await_activation.rs`) on an unprovisioned device logs `Device is not yet activated; waiting for provisioning...` and keeps polling while the service stays Running. So after an MSI install in CI, the service is running and has written that line, but only if its account can write `logs\`. MSI install success alone proves nothing about ACLs, because `agent/src/windows/scm.rs` reports `Running` before the agent body runs. The comment above `Assert-ServiceInstalled` in `integration-lib.ps1`, which says the unprovisioned agent "exits shortly after start", is out of date.

**Tests.** They run only on Windows, in the `windows-package` job of `.github/workflows/ci.yml` (runner `windows-latest`). That job runs on pull requests that touch `build/windows/**`, `.github/workflows/ci.yml`, or `.github/workflows/release.yml`, and on pushes to `main`/`release/*`. It builds `miru-agent.exe`, then runs:
- `build/windows/tests/package-tests.ps1`: a static MSI-table contract. `Assert-ProtectedPermissionRows` compares every `MsiLockPermissionsEx` row to `$MsiExpectedSddl`. `Assert-ServiceInstallRow` checks the `ServiceInstall` row, including `StartName`.
- `build/windows/tests/integration-tests.ps1`, which dot-sources `integration-lib.ps1`: a real install → maintenance → upgrade → downgrade → rollback → uninstall run on the disposable runner. Before install, maintenance, and upgrade, it loosens the ACLs with `Add-PermissiveAces`. Relevant functions:
  - `Assert-ProtectedAcl`: owner SYSTEM, protected, exactly 2 ACEs.
  - `Assert-InheritedProtection`: new files have exactly 2 inherited ACEs.
  - `Assert-TrustedIdentities`: expects exactly `S-1-5-18,S-1-5-32-544`.
  - `Assert-ServiceInstalled`: `StartName` `LocalSystem`.
  - `Invoke-NonAdminProbe`, using `non-admin-probe.ps1`: an ordinary local user is denied read, create, and re-grant.
- `build/windows/tests/MsiTest.psm1`: shared constants (`$MsiExpectedSddl`, etc.) and helpers (`Assert-Equal`, `Assert-True`, `Open-MsiDatabase`, `Get-MsiRows`, `Test-MsiTable`).

PowerShell and WiX are not available on the Linux dev host, so these tests are validated in CI.

**Design choices.**
- Grant the hardcoded service SID `S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695`, not the account name. The `Sddl` attribute accepts only SID strings and two-letter aliases, and the name could not be resolved anyway: `CreateFolders` applies the descriptor before `InstallServices` creates the service. Service SIDs are deterministic, so the value is known ahead of time.
- Set the SID type explicitly with `ServiceConfig ServiceSid="unrestricted"`. Windows may already do this for virtual accounts, but the explicit setting is safer and lands in an MSI table the package test can check. `MsiServiceConfig` needs Windows Installer 5.0, which the package already declares (`InstallerVersion="500"`).
- No Rust changes and no service-DACL grant. The agent's only SCM calls, in `agent/src/windows/scm.rs`, are `service_dispatcher::start`, `service_control_handler::register`, and `set_service_status`; none opens the SCM or a service handle. There is no `service_manager`, `OpenSCManager`, `OpenService`, `msiexec`, or `Command::new` in `agent/src`.
- Full control (`FA`) for the service SID, like SYSTEM and Administrators. It mirrors Linux, where `miru` owns its data directories, and keeps the harness's single "every ACE is FullControl" check. Modify would deny the agent `WRITE_DAC`, but the agent never sets ACLs and would need per-SID masks in the tests; left as possible hardening. (Superseded 2026-09-29: directory objects get `FRFWFX` plus an inherit-only `FA` ACE; see Decision Log.)
- No default config-deploy folder in the MSI. Deploy targets are customer-chosen absolute paths, so the README documents `icacls` grants instead.
- No `RequiredPrivilege` restriction. The virtual account keeps the default service privileges (for example `SeImpersonatePrivilege`); stripping them is follow-up hardening. (Superseded 2026-09-29: required privileges are now `SeChangeNotifyPrivilege` only; see Decision Log.)
- Leave `plans/completed/20260916-windows-msi-service.md` (historical) and `plans/active/20260910-windows-support.md` unedited. The active plan has no LocalSystem reference, but its Phase 2 device-API discovery-file ACL work will need to include this SID.

## Plan of Work

**Milestone 1 — installer and static contract.**

In `build/windows/miru-agent.wxs`, change `Account="LocalSystem"` on `ServiceInstall` to `Account="NT SERVICE\miru-agent"` and add no `Password`. Update the comment above it to say the service runs as its virtual account. Add a core-namespace child next to the existing `util:ServiceConfig`:

    <ServiceConfig ServiceSid="unrestricted" OnInstall="yes" OnReinstall="yes" />

`OnInstall`/`OnReinstall` set flag values 0x1 and 0x4 in the row's `Event` column, so the SID type is applied on install and repair. WiX is expected to warn that the core `ServiceConfig` family "does not work as expected" and to suggest `util:ServiceConfig`, which has no SID-type attribute, so do not switch. If it warns, because `build/windows/miru-agent.wixproj` sets `TreatWarningsAsErrors=true`, add `<SuppressSpecificWarnings>NNNN</SuppressSpecificWarnings>` to its `PropertyGroup`, where `NNNN` is the warning number from the build output, with an XML comment naming the warning and this reason. Record the number in the Decision Log. In every case, confirm the compiled `MsiServiceConfig` row (see the package test below).

In the `DataDirs` group, replace all four `Sddl` values with:

    O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695)

Add an XML comment above the group explaining: the last ACE is the service SID of `miru-agent`, derived as S-1-5-80 plus SHA-1 of the UTF-16LE uppercase service name split into five little-endian uint32 values (equal to `sc.exe showsid miru-agent`); it is hardcoded because `CreateFolders` runs before the service exists; all four folders need it because their DACLs are protected and files are renamed between them; and it must change if the service `Name` ever changes. (Superseded 2026-09-29: each directory grants the service SID FRFWFX on the directory plus an inherit-only FA ACE, `(A;;FRFWFX;;;<SID>)(A;OICIIO;FA;;;<SID>)`, four ACEs total; see Decision Log F3.)

In `build/windows/tests/MsiTest.psm1`, add `$MsiServiceName = "miru-agent"`, `$MsiServiceAccount = "NT SERVICE\miru-agent"`, and `$MsiServiceSid = "S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695"`. Define `$MsiServiceSid` first and build `$MsiExpectedSddl` from it so the value appears only once. Add `$MsiTrustedSids = @("S-1-5-18", "S-1-5-32-544", $MsiServiceSid)`. Add `Get-ServiceSid`, which computes the SID from a name:

    function Get-ServiceSid {
        param([Parameter(Mandatory = $true)][string]$ServiceName)
        $bytes = [Text.Encoding]::Unicode.GetBytes($ServiceName.ToUpperInvariant())
        $sha1 = [Security.Cryptography.SHA1]::Create()
        try { $hash = $sha1.ComputeHash($bytes) } finally { $sha1.Dispose() }
        $parts = 0..4 | ForEach-Object { [BitConverter]::ToUInt32($hash, $_ * 4) }
        return "S-1-5-80-" + ($parts -join "-")
    }

The module ends with an explicit `Export-ModuleMember -Function @(...) -Variable @(...)`, and anything not listed there is invisible to callers. Add `Get-ServiceSid` to the `-Function` list and `MsiServiceName`, `MsiServiceAccount`, `MsiServiceSid`, and `MsiTrustedSids` to the `-Variable` list.

In `build/windows/tests/package-tests.ps1`:
- In `Assert-ServiceInstallRow`, expect `$MsiServiceAccount` in `StartName` (`$row[6]`), and update the comment above the function.
- Add `Assert-ServiceSidConfig` and call it from `Assert-ServiceTables`. It asserts the `MsiServiceConfig` table exists (`Test-MsiTable`) and has exactly one row, with `Name='miru-agent'`, `Component_='MiruAgentExe'`, `[int]ConfigType` equal to 5 (SERVICE_CONFIG_SERVICE_SID_INFO), `Argument='1'` (SERVICE_SID_TYPE_UNRESTRICTED), and `([int]Event -band 1) -ne 0` (applied on install). Query `SELECT MsiServiceConfig, Name, Event, ConfigType, Argument, Component_ FROM MsiServiceConfig` with 6 columns, backtick-quoting identifiers like the existing queries. `Get-MsiRows` returns every column as a string. (Superseded 2026-09-29: renamed Assert-MsiServiceConfig, which expects two rows, the type-5 SID row and a type-6 SeChangeNotifyPrivilege row, both with Event bits 0x1 and 0x4; see Decision Log F2.)
- Add `Assert-ServiceSidDerivation`, called once before building packages. It asserts `Get-ServiceSid "TrustedInstaller"` equals the well-known `S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464`, and that `Get-ServiceSid $MsiServiceName` equals `$MsiServiceSid`. Print `PASS service SID derivation`.

`Assert-ProtectedPermissionRows` needs no edit; it picks up the new `$MsiExpectedSddl`.

**Milestone 2 — integration assertions** (`build/windows/tests/integration-lib.ps1`).

1. In `Assert-ProtectedAcl` and `Assert-InheritedProtection`, replace the literal `2` counts and their messages with `$MsiTrustedSids.Count`.
2. In `Assert-TrustedIdentities`, compare with `(($MsiTrustedSids | Sort-Object) -join ",")`. The owner check (`S-1-5-18`) stays.
   These checks also run after uninstall, when the service ACE remains with an unresolvable SID. `Get-Acl` reports such ACEs as raw SIDs, so the SID comparisons still pass.
3. In `Assert-ServiceInstalled`, expect `$MsiServiceAccount` as `StartName`, and replace the stale comment above it. Add a call to a new `Assert-ServiceSidIdentity $Stage`, which asserts:
   - `sc.exe qsidtype miru-agent` output matches `SERVICE_SID_TYPE:\s+UNRESTRICTED`;
   - `sc.exe showsid miru-agent` output contains `$MsiServiceSid` (Windows' own derivation from the name; it works even for a missing service);
   - `([Security.Principal.NTAccount]$MsiServiceAccount).Translate([Security.Principal.SecurityIdentifier]).Value` equals `$MsiServiceSid` (proves the account exists and maps to the hardcoded SID).
4. Add `Get-ActivationWaitCount`. It counts occurrences of `waiting for provisioning` across `miru.log*` files in `$logsRoot`, returning 0 if there are none. Open each file with `[IO.File]::Open($path, 'Open', 'Read', 'ReadWrite')` and read it through an `IO.StreamReader`, because the service holds it open for writing.
   Add `Assert-ServiceRuntimeIdentity $Stage $Baseline`. For up to 30 seconds, poll once a second until `Get-AgentService` reports `State='Running'` with a non-zero `ProcessId` and `Get-ActivationWaitCount` exceeds `$Baseline`, then assert both. Store the ID in `$processId`; do not name it `$pid`, which is PowerShell's read-only automatic variable for its own process. Assert that `Get-CimInstance Win32_Process -Filter "ProcessId=$processId" | Invoke-CimMethod -MethodName GetOwner` returns `Domain='NT SERVICE'` and `User='miru-agent'`.
   In `Invoke-InstallStage` and `Invoke-UpgradeStage`, capture `$baseline = Get-ActivationWaitCount` before `Install-Msi` and call `Assert-ServiceRuntimeIdentity` after `Assert-ServiceInstalled`. Skip the maintenance stage, because a REINSTALL does not reliably restart the service. The new log line proves that the account's ACE, applied before the service existed, lets the service write `logs\`. The install stage is the decisive case: no `miru.log*` file exists yet, so the service must create one using its own ACE.
5. In `Assert-ProtectedState`, after `Assert-ProtectedAcls`, run `Assert-InheritedProtection $file.Path` for every file in `$representativeFiles` (empty until the first probe). The maintenance and upgrade stages loosen the directories first, so this proves that re-applying the directory descriptor reaches files that already exist, the same mechanism an upgrade from a LocalSystem build relies on. If CI shows Windows Installer does not propagate to existing children, remove this step and the matching Validation sentence ("Files created earlier…"). Change manual test 5 to expect the README reset step. Add a README note for upgrades from LocalSystem builds: before the upgrade, grant the service SID on every existing item (`icacls /grant ... /T /C`), because the upgraded service panics if it cannot open the current-hour log file; after the upgrade, reset every existing file and folder under `%ProgramData%\Miru` except the root and its `logs`, `auth`, and `tmp` children, parents before children, so agent-created folders such as `resources\` and `events\` also inherit the service ACE, then restart the service (see the README for the exact command). Record all of this in Surprises & Discoveries and the Decision Log. (Superseded 2026-09-29: removing the representative-file loop stands, but the README LocalSystem upgrade note and the manual-test-5 reset step were removed; see Decision Log F1.)

Leave `non-admin-probe.ps1` unchanged; it must keep passing.

**Milestone 3 — docs.**

In `build/windows/README.md`:
- Replace each "LocalSystem"/"Local System" mention describing the *service account* (intro paragraph, "Package behavior" bullet, Validation paragraph) with the virtual account. Mentions of SYSTEM as *directory owner* and ACE holder stay.
- Update the DACL descriptions to include the service SID.
- Add a section "Service account and folder access" after "Install and provision". It should explain:
  - why the service runs as `NT SERVICE\miru-agent` (no admin rights, no password, full control of `%ProgramData%\Miru` only; the Windows counterpart of the Linux `miru` user) (Superseded 2026-09-29: full control of the contents only; the service cannot delete or re-permission the installer-created folders; see Decision Log F3.);
  - that the grants below work only after the MSI is installed (the name resolves once the service exists), or can use `*S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695` at any time (Superseded 2026-09-29: the README points to sc.exe showsid miru-agent instead of the literal SID; see Decision Log F1.);
  - config deploy target folders (the Windows counterpart of Linux `/srv/miru`) need Modify, because deploys create a temporary subfolder and a `miru.backup.*` file next to the target. Create the folder first; `icacls` fails on a missing path (Superseded 2026-09-29: the README recipe also runs icacls /inheritance:r /grant:r on C:\srv and C:\srv\miru to drop the inherited Authenticated Users Modify ACE; see Decision Log F7.):

        New-Item -ItemType Directory -Force "C:\srv\miru" | Out-Null
        icacls "C:\srv\miru" /grant "NT SERVICE\miru-agent:(OI)(CI)M"

  - file-rule source folders need read plus delete (delete is for retention):

        icacls "D:\robot\logs" /grant "NT SERVICE\miru-agent:(OI)(CI)(RX,D)"

  - that without a grant the service has only what the folder's ACL already gives groups such as `Users` and `Authenticated Users` (often write access for folders created at the root of `C:\`; none under `C:\Windows` or `C:\Program Files`). An explicit grant is the supported setup; a denied deploy fails with an access-denied deployment error and leaves the target unchanged;
  - that the applications reading deployed configs need their own read access to the target folder.

In `ARCHITECTURE.md`, at the "Agent runtime mode" sentence "On Windows it runs as service `miru-agent`…", add "as virtual account `NT SERVICE\miru-agent`".

**Milestone 4 — delivery validation.** See Concrete Steps and Validation.

## Concrete Steps

Before Milestone 1, from the agent repo root, move this plan to `plans/active/` and commit it so Progress and log updates travel with the PR (skip if it is already there):

    mkdir -p plans/active && mv plans/backlog/20260928-windows-virtual-service-account.md plans/active/
    git add plans/active/20260928-windows-virtual-service-account.md && git commit -m "docs(plans): activate windows virtual service account plan"

Update Progress in each milestone commit.

Milestone 1. From the agent repo root, re-derive the SID before editing and confirm the algorithm on a known SID:

    python3 - <<'EOF'
    import hashlib, struct
    def sid(n):
        d = hashlib.sha1(n.upper().encode('utf-16-le')).digest()
        return "S-1-5-80-" + "-".join(map(str, struct.unpack('<5I', d)))
    print(sid("TrustedInstaller")); print(sid("miru-agent"))
    EOF

Expected output:

    S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464
    S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695

The first line must equal the well-known TrustedInstaller SID. Make the Milestone 1 edits, then check that the SID appears in all four SDDLs:

    grep -c "S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695" build/windows/miru-agent.wxs   # expect 4 (+1 if the comment quotes it)
    grep -n "LocalSystem" build/windows/miru-agent.wxs build/windows/tests/MsiTest.psm1 build/windows/tests/package-tests.ps1   # expect no hits; integration-lib.ps1 is updated in Milestone 2

Commit: `git add build/windows plans/active && git commit -m "feat(windows): run miru-agent service as NT SERVICE\\miru-agent"`.

Milestone 2. Make the edits, then run `grep -n "LocalSystem\|only two" build/windows/tests/integration-lib.ps1` and expect no hits. Commit: `git commit -am "test(windows): assert virtual-account ACEs and service identity"`.

Milestone 3. Make the edits, then run `grep -n "LocalSystem" build/windows/README.md ARCHITECTURE.md` and expect only hits that describe the upgrade from earlier builds, if any. Commit: `git commit -am "docs(windows): document virtual service account and folder grants"`.

Milestone 4. Write a PR body file (for example `/tmp/pr-body.md`) with a short summary followed by the manual test plan from Validation and Acceptance. Push the branch and open a draft PR against `main`:

    git push -u origin feat/windows-virtual-service-account
    gh pr create --draft --base main --title "feat(windows): run miru-agent service as NT SERVICE\\miru-agent" --body-file /tmp/pr-body.md

If a PR for the branch already exists, set its body with `gh api -X PATCH repos/mirurobotics/agent/pulls/<number> -F body=@/tmp/pr-body.md` (`gh pr edit` fails in this organization because of the Projects (classic) deprecation).

Then run the `$preflight` agent skill (it pushes fixes and watches CI) until it reports `CLEAN`, meaning every CI job on the pushed head is green. Confirm the `windows-package` job ran and did not skip; it runs because `build/windows/**` changed. To check by hand:

    gh pr checks --watch
    gh run view <run-id> --log | grep -E "PASS |ASSERT"

Expect `PASS service SID derivation`, both `PASS package` lines, the `PASS` line of every integration stage (install, maintenance, upgrade, downgrade, rollback, uninstall), and no `ASSERT` lines. When CI is green, mark Milestone 4 done in Progress, fill in Outcomes & Retrospective, move the plan to `plans/completed/`, commit, push, and re-confirm preflight `CLEAN` on that new head.

## Validation and Acceptance

Automated acceptance, from the `windows-package` CI job on the pushed head:
- `package-tests.ps1` passes. `ServiceInstall.StartName` is `NT SERVICE\miru-agent`; `MsiServiceConfig` has the SID-type row (type 5, argument 1) and the required-privileges row (type 6, `SeChangeNotifyPrivilege`), both applied on install and repair; `MsiConfigureServices` runs between `InstallServices` and `StartServices`; the `Wix4ServiceConfig` row has three `restart` actions, a 10-second delay, and a 1-day reset period; the four rollback custom actions are Type 3393 `WixQuietExec64` calls that, with their `Set<action>` property actions, are conditioned on `UPGRADINGPRODUCTCODE AND REMOVE~="ALL"` and sequenced (restart first, each after its setter) between `RemoveExistingProducts` and `StopServices`, and their command lines match `$MsiServiceSettings`; all four `MsiLockPermissionsEx` rows carry the four-ACE SDDL (SYSTEM and Administrators full control; service SID `FRFWFX` on the directory plus inherit-only full control); and the SID derivation matches both TrustedInstaller and `$MsiServiceSid`.
- `integration-tests.ps1` passes every stage. After install, maintenance, upgrade, downgrade rejection, and rollback, the service starts automatically from the installed binary path and its `StartName` is the virtual account; `sc.exe qfailure` reports three restart actions with a 10000 ms delay and an 86400-second reset period; `sc.exe qsidtype` reports UNRESTRICTED; `sc.exe qprivs` lists only SeChangeNotifyPrivilege; `sc.exe showsid` and the account-name lookup both return the hardcoded SID; after install, upgrade, and rollback, the running process's token user is the service SID (`GetOwnerSid`), its owner is `NT SERVICE\miru-agent`, and its token holds no privilege but `SeChangeNotifyPrivilege` (after rollback, this proves the rollback restart); and a new `waiting for provisioning` line appears in `logs\` (decisive in the install stage, where no log file exists yet). All four protected directories are owned by SYSTEM with the SYSTEM and Administrators full-control ACEs and the two service ACEs; the install-stage log file inherits exactly the three trusted ACEs. The non-admin probe is still denied read, create, and re-grant.
- The primary proof for the riskiest step, the hardcoded SID applied before the service exists, is the new log line together with the showsid and account-name checks.

**Completion gate:** preflight must report `CLEAN` (CI green on the pushed branch head) before the PR leaves draft or the task is reported complete. A red or skipped `windows-package` job blocks completion.

Manual test plan (copy into the PR description; needs a real Windows 10/11 x64 machine and an elevated PowerShell):
1. Fresh install of the new MSI. `Get-CimInstance Win32_Service -Filter "Name='miru-agent'" | Select StartName, State` shows `NT SERVICE\miru-agent`, `Running`.
2. `miru-agent provision` (README flow) succeeds. The log shows `Device activated; starting agent.`; the service reads `auth\` and creates or updates files in `resources\`, `events\`, and `logs\` (check with `Get-ChildItem -Recurse "$env:ProgramData\Miru"`; `tmp\` is written only by the admin-run `provision` command). System metadata and the backend/MQTT connections work (device shows online).
3. Create and grant a folder (the README recipe for `C:\srv\miru`) and deploy a config there: it succeeds. Deploy a config targeting `C:\Windows\miru-test.json`: the deployment fails with an access-denied error and nothing is written.
4. With a file rule on a folder granted `(OI)(CI)(RX,D)`, a file is uploaded and later deleted by retention. On a folder whose DACL grants only Administrators, the agent logs an access error and keeps running.
5. Upgrade with reconcile under the virtual account: after test 2, run `Set-Content -LiteralPath "$env:ProgramData\Miru\agent_version" -Value v0.0.0` to force the upgrade reconcile, then install this branch's MSI at a higher version (for example `1.1.0`) with `msiexec /i ... /qn`; it must exit 0. `resources\` and `events\` are recreated. The newest `logs\miru.log.*` contains `upgrade: resetting storage state for version` followed by `Running the server with options`, and no log contains `updating agent version storage failed` (check with `Select-String -Path "$env:ProgramData\Miru\logs\miru.log.*" -Pattern 'resetting storage state for version|Running the server with options|updating agent version storage failed'`). `icacls "$env:ProgramData\Miru\auth"` shows the SYSTEM and Administrators full-control ACEs and the two `NT SERVICE\miru-agent` ACEs, none marked `(I)`. The device shows online.
6. As a non-admin local user, `Get-Content "$env:ProgramData\Miru\auth\token.json"` fails with access denied.
7. Uninstall: the service is removed and `%ProgramData%\Miru` is kept; files the service created show an orphaned owner SID, and Administrators keep full control.

Pre-release builds from `main` that ran as Local System are not upgraded in place: uninstall, delete `%ProgramData%\Miru`, install this MSI, and reprovision.

## Idempotence and Recovery

All edits are text edits and can be re-applied. CI runs on disposable runners, and the integration script refuses to run on a machine without `-ConfirmDisposableTestMachine`. Risky points and what to do:
- **Wrong or unresolvable SID.** Symptom: the log-line or showsid assertion fails. Re-run the Python snippet and correct all four SDDLs and `$MsiServiceSid`.
- **WiX fails the build on `ServiceConfig`** (warnings are errors). Suppress only that warning number, as described in Plan of Work. If the element still cannot build, or `MsiServiceConfig` does not apply in CI, remove the element and its package-test assertion, rely on Windows' default for virtual accounts, and keep the `qsidtype` integration assertion as the guard; record the choice in the Decision Log.
- **The service does not stay Running in CI.** Use the log line as the identity proof, drop the process-owner assertion, and record the change in Surprises & Discoveries.
- **Existing-file propagation fails.** Follow the fallback in Plan of Work, Milestone 2, step 5.

To roll back entirely, revert the milestone commits. Windows support has not shipped, so no customer installs need migrating.
