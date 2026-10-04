# Move Windows agent state and logs to %ProgramData%\Miru\Agent

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `/home/ben/miru/workbench1/repos/agent`) | read-write | Rust path defaults, Windows installer (WiX), installer test scripts, docs |

This plan lives in `agent/plans/` because every change is in this repo. Branch: `refactor/windows-agent-data-subdir` (already created off `main`). Linux paths do not change.

## Purpose / Big Picture

Today the Windows agent keeps its private state and logs directly in `%ProgramData%\Miru`, next to the two folders that local applications read (`configs` and `device-api`). After this change, everything private to the agent lives in its own folder, `%ProgramData%\Miru\Agent`. Only SYSTEM, Administrators and the agent's service account can access that folder. `%ProgramData%\Miru\configs` and `%ProgramData%\Miru\device-api` stay where they are, and members of the `Miru Agent Users` group can still read them.

Target layout on Windows (`%ProgramData%` is normally `C:\ProgramData`):

    %ProgramData%\Miru\                 SYSTEM+Admins full; service: list/traverse only
      Agent\                            private (SYSTEM, Admins, service SID)
        device.json, settings.json, agent_version, system_metadata.json,
        scanner.json, upload_queue.json, delete_queue.json   (runtime files)
        resources\, events\             (created by the agent at runtime)
        logs\installer-sentinel\
        auth\installer-sentinel\
        tmp\installer-sentinel\
      configs\installer-sentinel\       private + read for Miru Agent Users
      device-api\installer-sentinel\    private + read for Miru Agent Users

How to see it working: install the MSI on Windows and start the service. `C:\ProgramData\Miru\Agent\logs\miru.log*` appears. `C:\ProgramData\Miru\logs` does not exist. `icacls C:\ProgramData\Miru` shows the service SID with read and traverse access only. The Windows CI job `windows-package` runs exactly these checks.

No migration code is written. Windows has shipped only as an MSI in the betas v0.10.4-beta.2 and v0.11.0-beta.1, and beta testers will reinstall. The README says how to do that.

## Progress

- [ ] Milestone 1: Rust path changes and Rust tests.
- [ ] Milestone 2: installer (`miru-agent.wxs`) and installer test scripts.
- [ ] Milestone 3: docs.
- [ ] Milestone 4: preflight reports `CLEAN`.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: the `Miru\` root SDDL becomes `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFX;;;<svc>)`. The service gets read and traverse on the root folder itself only. That ACE does not inherit and grants no write.
  Rationale: the agent no longer keeps files at the root. It reaches `configs\` and `device-api\` through their own protected descriptors. Its bypass-traverse privilege already permits traversal, and the explicit read/traverse ACE is cheap insurance for directory existence checks such as `create_dir_all`'s `is_dir` fallback. Approved by the coordinator.
  Date/Author: 2026-10-03, plan author.
- Decision: `Agent\` and its installer-created children `logs\`, `auth\` and `tmp\` all get the same SDDL. It is defined once as a WiX preprocessor variable (`<?define AgentPrivateSddl = ... ?>`). This is the descriptor the root has today: SYSTEM and Admins full control (inherited), service `FRFWFX` on the folder itself, and service full control of its contents (inherit-only). Each of the four folders applies that string as its own protected descriptor; the children do not simply inherit from `Agent\`.
  Rationale: if `logs\`, `auth\` and `tmp\` inherited from `Agent\`, the inherit-only ACE would give the service full control of the folder itself. Full control includes FILE_DELETE_CHILD, so the service could delete the sentinel and turn the emptied folder into a mount point. A repair or upgrade running as SYSTEM would then follow that mount point. Approved by the coordinator.
  Date/Author: 2026-10-03, plan author.
- Decision: `Agent\` gets no installer sentinel.
  Rationale: `Agent\` always contains `logs\`, `auth\` and `tmp\`. The service cannot delete them because it has no DELETE right on them and no FILE_DELETE_CHILD on `Agent\`. Those folders are never removed on uninstall because each holds a permanent sentinel. So `Agent\` can never be empty and can never become a mount point. This is the same reasoning that leaves today's `Miru\` root without a sentinel. In the new layout the service also has no write access on `Miru\` at all. Approved by the coordinator.
  Date/Author: 2026-10-03, plan author.
- Decision: the components for `logs`, `auth`, `tmp` and their sentinels get new component GUIDs and new IDs. The new `Agent` directory gets a new component. The `Miru` root, `device-api`, `configs` and their sentinels keep their GUIDs.
  Rationale: Windows Installer component rules require a new GUID when a component's key path location changes. The old sentinel components are `Permanent`, so they stay registered at their old paths after upgrade, and reusing their GUIDs at a new path would conflict. Approved by the coordinator.
  Date/Author: 2026-10-03, plan author.
- Decision: no migration code. The README tells beta testers to uninstall, delete `%ProgramData%\Miru`, reinstall and reprovision.
  Rationale: Windows has never shipped in a stable release.
  Date/Author: 2026-10-03, task requirement.

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

Terms:

- *SDDL* is Windows' text form of a security descriptor. In `O:SYD:P(...)(...)`:
  - `O:SY` makes Local System the owner, and `D:P` is a protected DACL that inherits nothing from its parent.
  - Each `(A;flags;rights;;;SID)` is an allow ACE. `OI` and `CI` mean it inherits to files and folders. `IO` (inherit-only) means it applies to children but not to the folder itself.
  - `FA` is full control, `FR`/`FW`/`FX` are generic file read/write/execute (execute on a folder means traverse), `SY` is Local System and `BA` is built-in Administrators.
- The *service SID* is `S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695`. It is the identity of the virtual account `NT SERVICE\miru-agent`, and `sc.exe showsid miru-agent` prints it.
- An *installer sentinel* is an `installer-sentinel` subfolder that the MSI creates in each folder where the service has write access. It is owned by SYSTEM, accessible only to SYSTEM and Admins, and permanent. It keeps the parent non-empty, and a non-empty folder cannot be converted into a mount point (a junction). Without it, a compromised service could turn the parent into a mount point, and the next repair or upgrade, running as SYSTEM, would follow it and re-permission an arbitrary location.

Rust code (crate `miru-agent`, under `agent/`):

- `agent/src/platform/mod.rs`:
  - `data_root_suffix()` returns `var/lib/miru` on Unix and `Miru` on Windows.
  - `data_root_base()` returns `/` or `%ProgramData%`. `windows_data_root_base(Option<OsString>)` and `windows_log_dir(Option<OsString>)` are compiled on every OS, so they are unit-testable on Linux.
  - `windows_log_dir` currently returns `<base>\Miru\logs`.
  - The module doc comments mention "the layout appends `Miru`".
- `agent/src/disk/layout.rs`: `Layout { filesystem_root }`.
  - `root()` is `filesystem_root/data_root_suffix()`. `temp_dir()` (`tmp`), `auth()`, the JSON files, `resources()` and `events_dir()` all derive from `root()`, so they move automatically.
  - `device_api()` on Windows is currently `self.root()/device-api/device-api.json` and must change to `filesystem_root/Miru/device-api/device-api.json`. Its doc comment mentions "under the data root".
  - There is no `Layout::installer_sentinels()` function, and no Rust code references installer sentinels. Only the WiX file and the PowerShell tests do.
- Other Rust consumers need no code change:
  - `agent/src/logs/mod.rs:57` (`log_dir: platform::log_dir()`).
  - `agent/src/disk/setup.rs`, which creates `auth` and `events` and moves keys from `tmp` to `auth`.
  - `agent/src/provisioning/{provision,reprovision}.rs`, which write keys to `layout.temp_dir()`.
  - `agent/src/app/run.rs:116,438` (`layout.device_api()`).
- Default configs deploy target: the agent has none in code. Each config instance carries an absolute `filepath` from the backend, validated in `agent/src/deploy/filesys.rs` (`validate_filepath`: absolute, no `..`). `%ProgramData%\Miru\configs` is only a folder that the installer creates and the README documents as the default target. It is not derived from `data_root_suffix()` or `Layout`, so it stays at `Miru\configs` without any Rust change.
- Permissions on rename (tmp\ to configs\): config deploys never pass through `tmp\`.
  - `agent/src/deploy/filesys.rs` writes each config with `files::write_string(..., OVERWRITE_ATOMIC)`. That calls `atomicwrites::AtomicFile::new(path, ...)` in `agent/src/filesys/files.rs`, which creates its temporary file in a temporary subfolder of the destination's own parent folder.
  - So a deployed config is born inside `configs\`, inherits its ACL (including the `Miru Agent Users` read ACE), and is renamed within `configs\`.
  - Backups (`miru.backup.<name>`) are also siblings in the same folder.
  - `tmp\` feeds only `auth\`: provisioning writes `private.key`/`public.key` to `tmp\`, and `disk/setup.rs` moves them into `auth\`. Both folders keep identical private descriptors.
  - On NTFS, a same-volume rename keeps a file's existing ACL. If a file were ever moved from `tmp\` into `configs\`, it would lack the group read ACE. That would be pre-existing behaviour, since `tmp\` was already private, and no code does this. This move does not make it worse, so nothing is fixed here.
- Rust tests:
  - `agent/tests/disk/layout.rs`: the Windows `expected_root_under` returns `base.join("Miru")`, and the `device_api` test expects it under the root.
  - `agent/tests/platform/mod.rs`: the two `windows_defaults::log_dir_*` tests expect `Miru\logs`.
  - `agent/tests/logs/mod.rs:70` compares against `windows_log_dir(...)` and needs no edit.
  - `agent/tests/app/run.rs` uses `layout.device_api()` under a temp root and needs no edit.
  - `agent/tests/provisioning/shared.rs` creates an `installer-sentinel` in `layout.temp_dir()` and expects `tmp` to contain only it. It stays valid because `Agent\tmp` remains installer-owned.
  - The comment at `agent/src/provisioning/shared.rs:29` ("tmp\ itself is installer-owned on Windows") also stays true.
- Runtime folder creation: the agent creates missing folders, which then inherit their parent's ACL:
  - `dirs::create_if_absent` creates `auth` and `events`.
  - `files::write_bytes` creates the parent folder of any file it writes.
  - `tracing_appender` creates `logs`.

  After this change the service has no write access on `Miru\`. If an administrator deletes `Agent\` or `device-api\`, the agent cannot recreate them, and a missing `Agent\logs` makes the service exit at startup. An MSI repair restores them. This is accepted: the installer owns those folders.
- Other installer files: `build/windows/miru-agent.wixproj` (around line 81) and `build/windows/tests/integration-test.wxs` both reference `DirectoryRef Id="MIRUDATA"` for the test fixture. That ID is unchanged, so neither needs an edit.
- The Python device SDK hardcodes `%ProgramData%\Miru\device-api\device-api.json`, so `device-api` must stay at the `Miru\` level, as planned.

Installer (`build/windows/`):

- `build/windows/miru-agent.wxs`:
  - `StandardDirectory CommonAppDataFolder` contains `MIRUDATA` (`Miru`), with children `MIRULOGS`, `MIRUAUTH`, `MIRUTMP`, `MIRUDEVICEAPI` and `MIRUCONFIGS`. Each child has an `installer-sentinel` child directory (`MIRU*SENTINEL`).
  - `ComponentGroup DataDirs` has one directory-keyed component per folder, each with `CreateFolder/PermissionEx Sddl=...`. Every data folder, the root included, currently uses the identical "private" SDDL.
  - Sentinels use `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)` with `Permanent="yes"`.
  - `device-api` and `configs` add `util:PermissionEx User="Miru Agent Users"` read. The `util:Group` lives in `MiruDeviceApiDir`.
  - Two XML comments above `ComponentGroup DataDirs` explain the service ACEs, the sentinels and the group.
- `build/windows/tests/MsiTest.psm1`:
  - Pins `$MsiExpectedSddl`, `$MsiServiceDirectoryRights` (0x1201BF) and `$MsiExpectedDirectories` (component, GUID, directory ID, parent, name).
  - Also pins `$MsiSentinelDirectoryIds` and `$MsiAgentUsersFolders`. Its `Export-ModuleMember` list is around line 300.
- `build/windows/tests/package-tests.ps1`:
  - `Assert-DirectoryComponents` / `Assert-RetainedDirectoryComponent` check component GUIDs, directories and parents.
  - `Assert-ProtectedPermissionRows` (around line 143) expects every `$MsiExpectedDirectories` row to carry `$MsiExpectedSddl`.
- `build/windows/tests/integration-tests.ps1` (lines 36-60) defines:
  - `$programDataRoot`, `$logsRoot`, `$authRoot` and `$tmpRoot`.
  - `$agentUsersRoots` and `$protectedRoots`; the latter currently includes the root.
  - `$installerSentinelDirs`, `$markerPath` and `$customerOwnedFiles`.
- `build/windows/tests/integration-lib.ps1`:
  - `Assert-ProtectedAcls` and `Assert-ProtectedAcl` expect four ACEs: SY, BA and two service ACEs.
  - `Assert-ServiceDirectoryAces` and `Assert-InheritedProtection` expect files to inherit SY, BA and the service ACE.
  - `New-RepresentativeFiles` iterates `$protectedRoots + $agentUsersRoots`.
  - Other functions that iterate those lists: `Initialize-CustomerState`, `Add-PermissiveAces`, `Assert-ProtectedRootsRetained`, `Get-AgentLogFiles` (uses `$logsRoot`) and `Write-FailureEvidence`.
- `build/windows/tests/integration-test.wxs` installs `rollback-payload.txt` into `MIRUDATA` and needs no change.

CI (`.github/workflows/ci.yml`):

- A Windows job runs `cargo test --package miru-agent --locked`. Windows-only `#[cfg(windows)]` tests, such as the layout assertions, run only there.
- `windows-package` runs `build\windows\tests\package-tests.ps1` and `build\windows\tests\integration-tests.ps1`. It is gated on changes under `build/windows/**`, which this plan touches.
- Linux jobs run lint and the coverage gates (`scripts/covgate.sh`, with per-module `.covgate` files such as `agent/src/disk/.covgate` and `agent/src/logs/.covgate`).
- The repo's `$preflight` skill drives CI until it reports `CLEAN`.

Docs: `build/windows/README.md` and `ARCHITECTURE.md`.

- `build/windows/README.md`:
  - Lines 31-55 cover installer behaviour, and lines 155-172 the service account and folder access.
  - Lines 210-225 cover access for local applications, and lines 270-305 the integration-test expectations.
- `ARCHITECTURE.md` line 74 (`disk`) and line 112 (Storage) say the discovery file is "under the data root on Windows". Line 46 already gives the correct `C:\ProgramData\Miru\device-api\device-api.json`.
- `build/debian/README.md:55` mentions `ProgramData\Miru\configs`, which is unchanged.
- Leave everything under `plans/completed/` and `plans/active/` alone.

## Plan of Work

### Milestone 1: Rust paths

1. In `agent/src/platform/mod.rs`, change `data_root_suffix()` under `cfg(windows)` to `PathBuf::from("Miru").join("Agent")`. Change `windows_log_dir` to join `"Miru"`, `"Agent"`, `"logs"`. Update the doc comments on `data_root_base` and `data_root_suffix` to say `Miru\Agent`.
2. In `agent/src/disk/layout.rs`, change the Windows arm of `device_api()` to:

        self.filesystem_root
            .subdir("Miru")
            .subdir("device-api")
            .file("device-api.json")

   Rewrite its doc comment. On Windows the file is `%ProgramData%\Miru\device-api\device-api.json`, a sibling of the private `Miru\Agent` data root, in the folder that the installer opens to `Miru Agent Users`.
3. Update the tests:
   - `agent/tests/disk/layout.rs`: the Windows `expected_root_under` becomes `base.join("Miru").join("Agent")`.
   - The same file's `device_api` test builds its Windows expectation from `expected_default_base()` joined with `Miru\device-api\device-api.json`.
   - Add a `#[cfg(windows)] fn device_api_is_outside_data_root()` asserting `!layout.device_api().path().starts_with(layout.root().path())`.
4. In `agent/tests/platform/mod.rs`:
   - Make the two `windows_defaults::log_dir_*` tests expect `[base, "Miru", "Agent", "logs"]`, and rename them to `log_dir_nests_miru_agent_logs_under_program_data` and `log_dir_falls_back_to_program_data_default`.
   - Add `dispatch` tests for the suffix: `#[cfg(unix)] data_root_suffix_is_var_lib_miru` expecting `var/lib/miru`, and `#[cfg(windows)] data_root_suffix_is_miru_agent` expecting `Miru\Agent`.
5. `agent/tests/logs/mod.rs` needs no edit, because it compares against `windows_log_dir`. Confirm with grep that no other Rust test hardcodes the Windows root.

### Milestone 2: installer and installer tests (one commit, so CI stays green at every commit)

1. Edit `build/windows/miru-agent.wxs`.
   - Near the top, after `ProductName`/`Manufacturer`, add:

         <?define ServiceSid = "S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695" ?>
         <?define RootSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFX;;;$(ServiceSid))" ?>
         <?define AgentPrivateSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFWFX;;;$(ServiceSid))(A;OICIIO;FA;;;$(ServiceSid))" ?>
         <?define SentinelSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)" ?>

     The WiX preprocessor should substitute `$(ServiceSid)` when it defines the other variables. If the build shows the literal text `$(ServiceSid)` in `MsiLockPermissionsEx`, which `package-tests.ps1` catches, inline the SID in the defines and log it in Surprises.
     `configs` and `device-api` keep the private descriptor plus the group ACE, so they use `$(AgentPrivateSddl)` too. That string is correct for any folder where the service has full control of the contents. Name it in the comment as the "service-writable private descriptor".
   - Replace the `MIRUDATA` subtree with:

         <Directory Id="MIRUDATA" Name="Miru">
           <Directory Id="MIRUAGENTDATA" Name="Agent">
             <Directory Id="MIRUAGENTLOGS" Name="logs">
               <Directory Id="MIRUAGENTLOGSSENTINEL" Name="installer-sentinel" />
             </Directory>
             (same for MIRUAGENTAUTH "auth" and MIRUAGENTTMP "tmp")
           </Directory>
           <Directory Id="MIRUDEVICEAPI" Name="device-api"> ...unchanged... </Directory>
           <Directory Id="MIRUCONFIGS" Name="configs"> ...unchanged... </Directory>
         </Directory>

   - Components in `DataDirs`:
     - `MiruDataDir` keeps GUID `D0542DF7-5B61-4F09-938B-57F05C1B5458` and uses `Sddl="$(RootSddl)"`.
     - Add new components that use `$(AgentPrivateSddl)`, in place of `MiruLogsDir`, `MiruAuthDir` and `MiruTmpDir`:
       - `MiruAgentDataDir` (`MIRUAGENTDATA`, GUID `CAAEBE5E-2175-4DFC-A6DE-62973DF3A85D`)
       - `MiruAgentLogsDir` (`MIRUAGENTLOGS`, `8E86BA6A-C047-47C5-B274-515DBEE58C69`)
       - `MiruAgentAuthDir` (`MIRUAGENTAUTH`, `B26BE954-6594-4944-AFCC-24A427B7A45A`)
       - `MiruAgentTmpDir` (`MIRUAGENTTMP`, `D299C395-E3BF-4594-BAAC-36F4E4D18BA3`)
     - Replace the three old sentinel components with `Permanent="yes"` components that use `$(SentinelSddl)`:
       - `MiruAgentLogsSentinel` (`MIRUAGENTLOGSSENTINEL`, `3EB2C08A-9538-4383-ACF8-858CF225D209`)
       - `MiruAgentAuthSentinel` (`MIRUAGENTAUTHSENTINEL`, `EFC1A2C4-60B1-4915-A019-337A528CD424`)
       - `MiruAgentTmpSentinel` (`MIRUAGENTTMPSENTINEL`, `2B08A0FF-FE64-4103-B456-0FC8911C8C94`)
     - `MiruDeviceApiDir`, `MiruConfigsDir`, `MiruDeviceApiSentinel` and `MiruConfigsSentinel` keep their IDs and GUIDs. Switch their literal SDDLs to the defines.
   - Rewrite the two comments above `ComponentGroup DataDirs` so they state:
     - `Miru\` gives the service list and traverse only, because the service keeps nothing there. It reaches `configs` and `device-api` through their own descriptors.
     - `Agent\`, `logs`, `auth` and `tmp` each apply `AgentPrivateSddl` as their own protected descriptor, rather than inheriting, so that the service cannot delete or re-permission the installer-created folders.
     - Sentinels keep `Agent\logs`, `Agent\auth`, `Agent\tmp`, `device-api` and `configs` non-empty.
     - `Agent\` needs no sentinel because its protected children keep it non-empty. `Miru\` needs none for the same reason, and the service cannot write to it anyway.
     - The `Miru Agent Users` text stays, still covering only `device-api` and `configs`.
2. Edit `build/windows/tests/MsiTest.psm1`.
   - Rename `$MsiExpectedSddl` to `$MsiAgentPrivateSddl` (same value) and add `$MsiRootSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFX;;;$MsiServiceSid)"`.
   - Add `$MsiRootServiceRights = 0x1200A9`. That is FRFX: .NET reports `ReadAndExecute, Synchronize`. Verify the number from the first CI run, and record it in Surprises if it differs.
   - Rewrite `$MsiExpectedDirectories` with the new rows (component, GUID, directory, parent, name):
     - `MiruAgentDataDir`, `MIRUAGENTDATA`, parent `MIRUDATA`, name `Agent`.
     - `MiruAgentLogsDir`, `MiruAgentAuthDir` and `MiruAgentTmpDir`, with parent `MIRUAGENTDATA`.
     - The device-api and configs rows unchanged.
     - Add a sixth field holding each row's expected SDDL: `$MsiRootSddl` for `MIRUDATA` and `$MsiAgentPrivateSddl` for the rest.
   - Set `$MsiSentinelDirectoryIds = @("MIRUAGENTLOGSSENTINEL","MIRUAGENTAUTHSENTINEL","MIRUAGENTTMPSENTINEL","MIRUDEVICEAPISENTINEL","MIRUCONFIGSSENTINEL")`.
   - Update `Export-ModuleMember`.
3. Edit `build/windows/tests/package-tests.ps1`.
   - In `Assert-RetainedDirectoryComponent`, destructure the sixth field. It is unused there, so either ignore it or rename the variable.
   - In `Assert-ProtectedPermissionRows`, build each directory row as `"$($_[2])|CreateFolder|$($_[5])"`. Rename the "stable component identity" message if needed. The GUID assertion now pins the new GUIDs.
4. Edit `build/windows/tests/integration-tests.ps1`:

        $script:programDataRoot = Join-Path $env:ProgramData "Miru"
        $script:agentDataRoot = Join-Path $script:programDataRoot "Agent"
        $script:logsRoot  = Join-Path $script:agentDataRoot "logs"
        $script:authRoot  = Join-Path $script:agentDataRoot "auth"
        $script:tmpRoot   = Join-Path $script:agentDataRoot "tmp"
        (deviceApiRoot, configsRoot unchanged under programDataRoot)
        $script:protectedRoots = @($agentDataRoot, $logsRoot, $authRoot, $tmpRoot)
        $script:legacyRoots = @("logs","auth","tmp" | ForEach-Object { Join-Path $programDataRoot $_ })

   - Keep `$markerPath` in the root.
   - Change the `integration-sentinel.txt` customer file to live in `$agentDataRoot`, and add a second customer file `root-owned.txt` in `$programDataRoot`. Both prove retention.
5. Edit `build/windows/tests/integration-lib.ps1`:
   - Add `Assert-RootAcl $programDataRoot`, called from `Assert-ProtectedAcls`. It checks:
     - Owner is SYSTEM, the DACL is protected, and there are exactly 3 explicit ACEs.
     - SY and BA have inheritable full control.
     - The service has exactly one ACE, with rights `$MsiRootServiceRights`, `InheritanceFlags` 0 and no inherit-only flag.
   - Include `$programDataRoot` in `Initialize-CustomerState`, `Add-PermissiveAces` (so repair of the root is tested) and `Assert-ProtectedRootsRetained`, alongside `$protectedRoots`.
   - Give `New-RepresentativeFile` and `Assert-InheritedProtection` a `-NoService` switch, used for the root, under which inherited ACEs must be SY and BA only. Add one representative root file so the non-admin and member probes still prove that nobody but admins can read root files.
   - In `Assert-ProtectedState`, assert that each `$legacyRoots` path does not exist, so no stage recreates `Miru\logs`, `Miru\auth` or `Miru\tmp`.
   - `Write-FailureEvidence` also prints `icacls` for `$agentDataRoot`.
   - `Get-AgentLogFiles` uses `$logsRoot`, so the existing "service writes its log" checks now prove the agent logs to `Miru\Agent\logs` end to end.

### Milestone 3: docs

1. `build/windows/README.md`:
   - In the installer bullet list, describe:
     - `Miru` (service: list and traverse only).
     - `Miru\Agent` and its `logs`, `auth` and `tmp` children (private descriptor).
     - `device-api` and `configs` (private descriptor plus `Miru Agent Users` read).
     - Sentinels in `Agent\logs`, `Agent\auth`, `Agent\tmp`, `device-api` and `configs`.
   - In "Service account and folder access", the service has full control of the contents of `%ProgramData%\Miru\Agent`, `configs` and `device-api`.
   - In "Access for local applications", members get nothing on `%ProgramData%\Miru` or `%ProgramData%\Miru\Agent`.
   - Update the integration-test expectations paragraph to match Milestone 2.
   - Add a short "Upgrading from a beta" note: v0.10.4-beta.2 and v0.11.0-beta.1 kept state in `%ProgramData%\Miru`, and this version does not migrate it. Uninstall, delete `%ProgramData%\Miru` from an elevated shell, install, and provision again.
   - Keep `configs` examples (`C:\ProgramData\Miru\configs\...`) as they are.
2. `ARCHITECTURE.md`: on lines 74 and 112, replace "under the data root on Windows" with `%ProgramData%\Miru\device-api\device-api.json` on Windows. On line 112, add that the Windows data root is `%ProgramData%\Miru\Agent` (logs in `Agent\logs`).
3. Grep for leftover references and fix comments outside `plans/`:

        grep -rn -i -E 'ProgramData|Miru\\\\(logs|auth|tmp)|data root' --exclude-dir=target --exclude-dir=.git --exclude-dir=plans .

## Concrete Steps

All commands run from `/home/ben/miru/workbench1/repos/agent` on branch `refactor/windows-agent-data-subdir`.

Milestone 1:

    cargo test --package miru-agent --test mod platform::
    cargo test --package miru-agent --test mod disk::layout
    cargo test --package miru-agent --test mod logs::
    ./scripts/lint.sh

Expect all listed tests to pass on Linux. The Windows `cfg(windows)` layout tests run only in the CI Windows job. Before the change, `windows_defaults::log_dir_*` fails if only the test is edited, which confirms that the tests pin the new path. Run `git status` after `lint.sh`, because it auto-fixes. Commit with the `$commit` skill using `refactor(platform): root Windows agent state at ProgramData\Miru\Agent`.

Milestone 2 has no local runner, because WiX and the MSI tests need Windows. Sanity-check the XML:

    python3 -c "import xml.dom.minidom,sys;xml.dom.minidom.parse('build/windows/miru-agent.wxs')"
    grep -c 'S-1-5-80-1251439239' build/windows/miru-agent.wxs   # expect 1 (the define)

Commit with `build(windows): move agent-private ProgramData folders under Miru\Agent`.

Milestone 3: run the grep above and expect no stale hits. Commit with `docs(windows): document Miru\Agent data layout`.

Milestone 4: run the `$preflight` skill. It pushes the branch, watches `ci.yml` and fixes failures from the job logs. It must end with `CLEAN`. The `windows-package` job is the only real check of the WiX and ACL changes, and the Windows test job is the only real check of the `cfg(windows)` layout tests.

## Validation and Acceptance

- Linux: `./scripts/test.sh` passes, and `./scripts/covgate.sh` passes with no `.covgate` change. Linux paths are byte-identical: the `root_dir` test still pins `/var/lib/miru`, and `unix_log_dir` is still `/var/log/miru`.
- `windows_log_dir(Some("D:\CustomData"))` returns `D:\CustomData\Miru\Agent\logs` (`agent/tests/platform/mod.rs`, runs on every OS).
- On the CI Windows test job:
  - `Layout::default().root()` is `%ProgramData%\Miru\Agent`.
  - `settings()` is `...\Miru\Agent\settings.json`, and `auth().root` is `...\Miru\Agent\auth`.
  - `device_api()` is `%ProgramData%\Miru\device-api\device-api.json`, and `device_api_is_outside_data_root` passes.
- On the CI `windows-package` job:
  - `package-tests.ps1` passes with the exact `MsiLockPermissionsEx` rows: the root with `RootSddl`; `Agent`, `logs`, `auth`, `tmp`, `device-api` and `configs` with `AgentPrivateSddl`; and the five sentinels with `SentinelSddl`.
  - `integration-tests.ps1` passes through install, repair, upgrade, failed-upgrade rollback, failed uninstall and uninstall. Across those stages:
    - The service writes `miru.log*` in `C:\ProgramData\Miru\Agent\logs`.
    - `C:\ProgramData\Miru\{logs,auth,tmp}` never exist.
    - The root ACL has the service ACE with read and traverse only.
    - `Miru Agent Users` members can read `device-api` and `configs` files but not files in the root, `Agent`, `logs`, `auth` or `tmp`.
    - All five sentinels are present and not reparse points.
    - Customer files in the root and in `Agent` are retained.
- Completion gate: the `$preflight` skill must report `CLEAN`, meaning CI is green on the pushed branch head. Until then the PR stays in draft and the task must not be reported complete.

## Idempotence and Recovery

All edits are plain source changes and can be re-applied. If a milestone's CI run fails, fix forward on the branch. To abandon a milestone, `git revert` its single commit.

The integration tests refuse to run on a machine where `%ProgramData%\Miru` already exists, so CI runners are always clean. Do not run them on a developer machine.

A wrong component GUID is caught by `package-tests.ps1`. Never reuse the old `logs`, `auth` and `tmp` GUIDs listed in `$MsiExpectedDirectories` before this change.
