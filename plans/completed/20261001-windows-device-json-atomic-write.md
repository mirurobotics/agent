# Windows: refuse to provision into a wiped data directory and surface the cause of failed atomic writes

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (`/home/ben/miru/workbench1/repos/agent`) | read-write | Error-message fix, Windows provisioning guard, Rust tests, MSI integration-harness stage, Windows README troubleshooting. |

This plan lives in `agent/plans/` because every change is in the agent repository. Branch: `fix/windows-device-json-atomic-write`, base `main`. All paths are relative to the agent repo root, and every command runs from that root unless stated otherwise.

## Purpose / Big Picture

A user reset a Windows device with the MSI still installed. The steps that broke it, confirmed with the user:

1. `Remove-Item -Recurse -Force "$env:ProgramData\Miru"`. This deleted the installer-created folders, their protected DACLs, and the `installer-sentinel` folders.
2. `miru-agent.exe provision` as an elevated administrator, without reinstalling. Provision recreated `C:\ProgramData\Miru` and its children with ProgramData's default inherited ACL, which has no `NT SERVICE\miru-agent` entry.
3. `Restart-Service miru-agent`. The service, acting only as a member of Users, could read the administrator-owned `device.json` but not delete or replace it, so the startup atomic write failed:

       ERROR miru_agent::app::run: ... Failed to start server: failed to write file atomically: C:\ProgramData\Miru\device.json

Uninstalling, wiping, and reinstalling v0.10.4-beta.2 restored the sentinels and ACLs. After this change:

1. On Windows, `miru-agent provision` and `miru-agent reprovision` refuse to run when the installer-owned folders are missing. They create nothing and exit 1 with an error that says to repair or reinstall the MSI.
2. On every platform, atomic-write and read-directory errors include the OS error, for example `failed to write file atomically 'C:\ProgramData\Miru\device.json': Access is denied. (os error 5)`.
3. A CI stage in the MSI integration harness runs the confirmed sequence. It checks that provision is refused and creates nothing, that the documented MSI repair restores the folders, and that, after the repair, the service can replace an administrator-written `device.json`.
4. `build/windows/README.md` says not to delete `%ProgramData%\Miru` while the MSI is installed, and explains how to reset state and how to recover.

## Progress

- [x] Milestone 1: OS error in `AtomicWriteFileErr` and `ReadDirErr` messages, with tests; commit.
- [x] Milestone 2: Windows provisioning guard (`assert_installer_layout`), with tests; commit.
- [x] Milestone 3: MSI harness stage for the wiped-state sequence; commit.
- [x] Milestone 4: README troubleshooting; commit.
- [ ] Preflight reports `CLEAN`; PR leaves draft.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Detect the installer-owned layout by checking that `auth\installer-sentinel` and `tmp\installer-sentinel` under the data root are real directories (`std::fs::symlink_metadata(..).is_dir()`, so links and junctions do not count).
  Rationale:
  - Only the MSI creates these folders. The agent never does, and a wipe deletes them.
  - `auth\` and `tmp\` are the installer folders that provision writes into, and their sentinels are the oldest: they shipped in v0.10.4-beta.1 (#269), before the `device-api` and `configs` sentinels. The exe and the MSI ship together, so any installed exe that carries this guard has them.
  - Checking only that the directories exist is not enough. ProgramData lets Users create folders, so a running service or an earlier provision can recreate `Miru`, `logs`, or `auth` with default ACLs.
  - Checking the DACL for the service SID entry would need new `windows-sys` security features and ACE parsing, and would copy the installer's SDDL into Rust as a second source of truth. The sentinel check catches the confirmed sequence with a few lines of portable code.
  Date/Author: 2026-10-01, plan author.
- Decision: Call the guard from `agent/src/main.rs` (`run_provision` and `run_reprovision`, under `#[cfg(windows)]`, as their first statement), not from inside `provision::provision` or `reprovision::reprovision`. Keep the guard function itself platform-neutral.
  Rationale:
  - Tests call those library functions with temporary layouts on every platform.
  - Unix installs have no sentinels.
  - Running the guard first means nothing is written (no temp keys, no root folder) before it fails.
  - The guard is a public library function, so its logic is tested on Linux and in `windows-check`.
  `provision --check` is read-only and is not guarded. The service and `--console` are out of scope: the service cannot recreate the sentinels, and `--console` is a diagnostic path.
  Date/Author: 2026-10-01, plan author.
- Decision: Do not repair ACLs from the CLI. The recovery is an MSI repair, not `icacls /reset`.
  Rationale: The provisioning CLI runs as an administrator inside folders the service can write. Re-permissioning or rewriting files there by name can follow a link that a compromised service planted, and `plans/completed/20260930-windows-service-hardening-followups.md` already treats this as residual risk. `icacls /reset` does not help either: once the folders are wiped, the installer's protected DACL is gone, so `/reset` would only re-inherit ProgramData's defaults.
  Date/Author: 2026-10-01, plan author.
- Decision: Do not add a Windows retry around atomic writes.
  Rationale: The failure is a permanent permission denial, not a transient lock. No agent handle on `device.json` is open during the write. A retry would only delay real denials.
  Date/Author: 2026-10-01, plan author.
- Decision: The harness renames `%ProgramData%\Miru` away and back instead of deleting it.
  Rationale: The later uninstall stage still checks that customer-owned and representative files survive. Renaming within the same volume keeps the original DACLs, so restoring is exact.
  Date/Author: 2026-10-01, plan author.

- Decision: `assert_installer_layout` treats only `NotFound` / `NotADirectory` (and a sentinel that exists but is not a directory) as a missing layout. Any other `symlink_metadata` error, such as `PermissionDenied`, returns `ProvisionErr::FileSysErr(FileSysErr::DirMetadataErr)` with the OS error as its source.
  Rationale: Milestone 2 step 3 originally mapped every error to `InstallerLayoutErr`. A non-elevated `provision` on a healthy install cannot traverse the protected `auth` folder, and would have been told the folder was missing and to repair the MSI, hiding the real cause.
  Date/Author: 2026-10-01, implement (refine pass).
- Decision: The README troubleshooting trigger says the service "starts but then stops with service-specific error 1 (System event 7024)" rather than "fails to start with `Access is denied. (os error 5)`", and the sentence explaining `-File` was removed.
  Rationale: The OS error text only appears in the log; the SCM reports `ServiceSpecific(1)` (`agent/src/windows/scm.rs`). The removed `-File` sentence's reason was wrong: `-Filter "miru.log*"` already excludes the `installer-sentinel` folder. `-File` stays in the snippet.
  Date/Author: 2026-10-01, implement (refine pass).

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

**Installer layout.** `build/windows/miru-agent.wxs` installs the service `miru-agent` running as the virtual account `NT SERVICE\miru-agent`, with service SID `S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695`. It creates `%ProgramData%\Miru` and the subfolders `logs`, `auth`, `tmp`, `device-api`, and `configs`. Each gets the protected DACL `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFWFX;;;<svcSID>)(A;OICIIO;FA;;;<svcSID>)`:
- On the folder itself, the service gets read, write, and traverse (`FRFWFX`), but not `FILE_DELETE_CHILD`. This is deliberate, so the service cannot delete the protected folders.
- Every file created inside inherits full control for the service.

Each subfolder also contains a permanent `installer-sentinel` folder that only SYSTEM and Administrators can modify. The installer applies all of this when it installs, repairs, or upgrades. Nothing recreates it if someone deletes the folders.

**Why the startup write fails after a wipe.** Several facts combine:
- `provision` (`agent/src/provisioning/provision.rs`) and `reprovision` (`agent/src/provisioning/reprovision.rs`) call `disk::setup::bootstrap` (`agent/src/disk/setup.rs`). Through `filesys::files::write_bytes`, which calls `dirs::create_if_absent` on each parent, this creates any missing folder. It then writes `device.json` with `status: Online`, from `impl From<&backend_api::models::Device>` in `agent/src/models/device.rs`.
- At service start, `AppState::init` (`agent/src/app/state.rs`) runs `disk::Storage::init`, which calls `init_device_storage` (`agent/src/disk/mod.rs`) to patch the status to `Offline`. That patch is an atomic write: `write_bytes_atomic` in `agent/src/filesys/files.rs`, using the `atomicwrites` crate. The crate writes a temporary file in a `.atomicwrite*` subfolder of the same folder, then calls `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)`.
- Replacing the file needs `DELETE` on `device.json` or `FILE_DELETE_CHILD` on its folder.
- With ProgramData's default ACL, the service gets only Users' read access to administrator-created files, so the replace fails with OS error 5.

**Why the cause is missing from the log.**
- `agent/src/app/run.rs` logs `error!("Failed to start server: {}", e)`, and `ServerErr` → `DiskErr`/`StorErr` → `FileSysErr` are all `#[error(transparent)]`.
- In `agent/src/filesys/errors.rs`, `AtomicWriteFileErr` is `#[error("failed to write file atomically: {file}")]`, which leaves out its `source` field. `ReadDirErr` (`"failed to read directory: {dir}"`) has the same defect.
- These two are the only filesys errors whose message leaves out the source. `atomicwrites` passes the OS error through, so adding `{source}` is enough.

**Provisioning code.**
- `agent/src/main.rs` `run_provision` and `run_reprovision` initialize logging into a temporary folder, build the HTTP client, set `layout = disk::Layout::default()`, read `MIRU_PROVISIONING_TOKEN` (`provisioning::read_token_from_env`), and call the library function.
- Errors reach `handle_provision_result` / `handle_reprovision_result`, which print `An error occurred during provisioning.\n\nError: {e}` and exit 1.
- `ProvisionErr` is in `agent/src/provisioning/errors.rs`, with its `impl_error!` list.
- `agent/src/provisioning/shared.rs` is private and re-exported selectively from `agent/src/provisioning/mod.rs`.
- `agent/src/disk/layout.rs` `Layout` has `root()`, `auth()` (an `AuthLayout` with `.root`), and `temp_dir()`. On Windows `root()` is `%ProgramData%\Miru`.
- Provisioning tests live in `agent/tests/provisioning/` (`shared.rs` already defines `TEMP_SENTINEL = "installer-sentinel"` as a fixture).

**Tests and CI.** `.github/workflows/ci.yml` runs these jobs:
- `lint`.
- `test`: `./scripts/covgate.sh` on Ubuntu.
- `windows-check`: `cargo test --package miru-agent --locked` on Windows Server 2025, so `#[cfg(windows)]` tests run.
- `windows-package`: on `windows-latest`, builds the MSI and runs `build\windows\tests\package-tests.ps1` and `build\windows\tests\integration-tests.ps1 -ConfirmDisposableTestMachine`. On pull requests it runs only when `build/windows/**` changes, which Milestone 3 does.
- `tools`.

The integration test target is `agent/tests/mod.rs`, so it is invoked as `--test mod`. Per `AGENTS.md`, gate a test with `#[cfg(unix)]` only for Unix semantics such as mode bits or symlinks.

The harness stages are in `build/windows/tests/integration-lib.ps1`:
- Entry points: `Invoke-IntegrationLifecycle` and the `Invoke-*Stage` functions.
- Package helpers: `Install-Msi`, `Invoke-Msi`, and `$Packages` (with keys `V1`, `V2`, `V3`, `V2Beta`). After `Invoke-RollbackStage`, `fixture-v2` is the installed product.
- Assertions: `Assert-ProtectedAcls`, `Assert-InheritedProtection`, `Assert-ServiceRuntimeIdentity`.
- Log helpers: `Get-AgentLogFiles`, `Get-ActivationWaitCount`.
- Variables: `$programDataRoot`, `$protectedRoots`, `$installerSentinelDirs`, `$agentPath`, `$MsiServiceSid`.

## Plan of Work

**Milestone 1: OS error in the message.**
1. In `agent/tests/filesys/errors.rs` module `display`, extend `atomic_write_file_err` to assert `msg.contains("atomic write failed")`, and `read_dir_err` to assert `msg.contains("readdir failed")`. These are the tests' own `io::Error::other` texts, so both tests fail at this point.
2. In `agent/src/filesys/errors.rs`, change only:

       #[error("failed to write file atomically: {file}")]   ->   #[error("failed to write file atomically '{file}': {source}")]
       #[error("failed to read directory: {dir}")]           ->   #[error("failed to read directory '{dir}': {source}")]

3. In `agent/tests/filesys/files.rs` `pub mod write_bytes`, add two tests:
   - `#[cfg(unix)] atomic_write_denied_reports_os_error`:
     - Make a temp dir (`test_dirs::temp`) and write `test-file` into it.
     - Set the dir to `0o555` with `filesys::dirs::set_permissions`.
     - Call `files::write_bytes(&file, b"x", WriteOptions::OVERWRITE_ATOMIC)`.
     - Restore the dir to `0o755`, then assert: the error is `FileSysErr::AtomicWriteFileErr`, its `source.kind()` is `PermissionDenied`, and `to_string()` contains `"os error 13"`.
   - `#[cfg(windows)] atomic_overwrite_blocked_by_reader_reports_os_error`:
     - Write `test-file`, then hold it open with `std::fs::OpenOptions::new().read(true).share_mode(0x1 /* FILE_SHARE_READ */)`. Import `std::os::windows::fs::OpenOptionsExt` inside the test.
     - While the handle is open, call `files::write_bytes(.., OVERWRITE_ATOMIC)` and assert `AtomicWriteFileErr` whose text contains the path and `"os error"`.
     - Drop the handle and assert the contents are unchanged.

**Milestone 2: provisioning guard.**
1. In `agent/src/disk/layout.rs`, add `pub const INSTALLER_SENTINEL: &str = "installer-sentinel";` and the method `pub fn installer_sentinels(&self) -> [filesys::Dir; 2]`, returning `self.auth().root.subdir(INSTALLER_SENTINEL)` and `self.temp_dir().subdir(INSTALLER_SENTINEL)`. Add a doc comment saying the Windows MSI creates these sentinels and the agent never does.
2. In `agent/src/provisioning/errors.rs`, add:

       #[derive(Debug, thiserror::Error)]
       #[error("Miru agent state directory is missing or was not created by the installer ('{missing}' not found). Do not delete %ProgramData%\\Miru while the Miru Agent is installed. Repair the installation (msiexec /fvomus <path to the Miru Agent MSI>) or reinstall the MSI, then provision again.")]
       pub struct InstallerLayoutErr { pub missing: std::path::PathBuf, pub trace: Box<Trace> }
       impl crate::errors::Error for InstallerLayoutErr {}

   Add `InstallerLayoutErr(InstallerLayoutErr)` (`#[error(transparent)]`) to `ProvisionErr` and to its `impl_error!` list.
3. In `agent/src/provisioning/shared.rs`, add `pub fn assert_installer_layout(layout: &disk::Layout) -> Result<(), ProvisionErr>`. For each `layout.installer_sentinels()`, accept the folder only when `std::fs::symlink_metadata(dir.path())` is `Ok(m)` with `m.is_dir()`. Otherwise return `InstallerLayoutErr { missing: dir.path().clone(), trace: trace!() }`. It must create nothing. Re-export it from `agent/src/provisioning/mod.rs` next to `read_token_from_env`.
4. In `agent/src/main.rs`, make this the first statement of both `run_provision` and `run_reprovision`:

       #[cfg(windows)]
       provisioning::assert_installer_layout(&disk::Layout::default())?;

5. Add tests in `agent/tests/provisioning/shared.rs`. They are portable, so they run on Linux and in `windows-check`. Each builds `Layout::new(temp_dir)`:
   - With both sentinel dirs created (`dirs::create_if_absent`), the guard returns `Ok`.
   - With nothing created, it returns `ProvisionErr::InstallerLayoutErr`, whose text contains `"not created by the installer"` and `"msiexec"`, and afterwards `layout.root()` does not exist (proving it creates nothing).
   - With only the `auth` sentinel present, `missing` is the `tmp` sentinel path.
   - With the `tmp` sentinel present as a regular file, it returns the error.
   - `#[cfg(unix)]`: with the `tmp` sentinel present as a symlink to a directory, it returns the error.
6. Add one test in `agent/tests/disk/layout.rs` that `installer_sentinels()` returns `<root>/auth/installer-sentinel` and `<root>/tmp/installer-sentinel`.

**Milestone 3: MSI harness stage.** In `build/windows/tests/integration-lib.ps1`, add `Invoke-WipedStateStage $Packages` and call it in `Invoke-IntegrationLifecycle` between `Invoke-RollbackStage $packages` and `Invoke-UninstallStage`. The installed product at that point is `fixture-v2` (`$Packages.V2`). Steps:

1. `Stop-Service miru-agent`. Rename `$programDataRoot` to `$backup = "$programDataRoot.harness-backup-<guid>"` with `Rename-Item`. This stands in for the user's `Remove-Item` and keeps the customer files.
2. Remove `MIRU_PROVISIONING_TOKEN` from the process environment. Run `$out = & $agentPath provision 2>&1 | Out-String` and assert:
   - `$LASTEXITCODE -eq 1`.
   - `$out` contains `not created by the installer`.
   - `Test-Path $programDataRoot` is false, so no default-ACL folder was created.

   Do the same for `& $agentPath reprovision`.
3. Repair with the documented command: `Invoke-Msi @("/fvomus", ('"{0}"' -f $Packages.V2)) "fixture-v2-repair-after-wipe" @(0, 3010) | Out-Null`. Then call the existing `Assert-ProtectedAcls`, which checks every protected folder, the two `Miru Agent Users` folders, and all five sentinels. If `/fvomus` does not recreate them, record it in Surprises & Discoveries. Switch the harness, the README, and the error text together to the harness-proven `/i <msi> REINSTALL=ALL REINSTALLMODE=vomus`.
4. Run `& $agentPath provision` again without a token. Assert exit 1 and that the output contains `MIRU_PROVISIONING_TOKEN`: the guard passed and the run failed at the token check instead.
5. Write provisioned state as an administrator, the way `provision` would, with the service stopped (stop it if the repair started it):
   - `auth\private_key.pem` and `auth\public_key.pem`: placeholder text. Only their existence is checked before the patch.
   - `settings.json`: `{"enable_poller":false,"enable_mqtt_worker":false}`, so the runner never contacts the production backend.
   - `agent_version`: the first line of `& $agentPath --version` with `Version: ` stripped (e.g. `v0.10.4-beta.2`), followed by a newline. It must match exactly, or the service runs a backend version reconcile.
   - `device.json`:

         {"device_id":"dvc_harness","session_id":"harness","name":"harness","activated":true,"status":"online","last_synced_at":"1970-01-01T00:00:00Z","last_connected_at":"1970-01-01T00:00:00Z","last_disconnected_at":"1970-01-01T00:00:00Z"}

   Call `Assert-InheritedProtection` on `device.json`.
6. Do not open `device.json` while the service starts. Any handle, even from PowerShell, blocks the replace and would fake the bug.
   - Add a `Get-AgentLogMatchCount -Pattern` helper that reads `Get-AgentLogFiles` with read-write sharing, as `Get-ActivationWaitCount` does, and make `Get-ActivationWaitCount` call it.
   - Take a baseline count of `Initializing token refresh worker...`, which is logged right after `AppState::init` and so after the patch.
   - Start the service and poll for up to 30 s until the count rises.
   - Read `device.json` once with `ConvertFrom-Json` and assert `.status -eq "offline"`. This proves that, after the repair, the service SID can replace an administrator-written `device.json`.
7. Add a `catch` before cleanup that prints `icacls $programDataRoot` and `icacls` of `device.json` (if they exist) and the last 50 lines of the newest log file, then rethrows. The log file is chosen with `Get-ChildItem -LiteralPath $logsRoot -Filter "miru.log*" -File | Sort-Object LastWriteTime | Select-Object -Last 1`. CI uploads only MSI logs.
8. In `finally`, put the original folder back:
   - Stop the service. Remove the recreated `$programDataRoot` if it exists (`Remove-Item -Recurse -Force -LiteralPath`). Rename `$backup` back to `Miru`.
   - Take a baseline with `Get-ActivationWaitCount`, start the service, and call `Assert-ServiceRuntimeIdentity "wiped-state restore" $baseline`.
   - Print a `PASS` line like the other stages.

**Milestone 4: README.** In `build/windows/README.md`, add a "Resetting state and troubleshooting" section after "Service account and folder access". It covers:
- **Do not delete `%ProgramData%\Miru` while the MSI is installed.** Deleting it removes the installer's folder permissions and sentinels, and nothing recreates them.
- **To reset state, choose one:**
  - Uninstall first (`msiexec /x <msi>` or Apps & Features), then delete the folder, then reinstall.
  - Delete the folder, then repair with `msiexec /fvomus "<path to miru-agent-<version>.msi>"` (or reinstall), then provision.
- **The guard's error.** `provision` and `reprovision` now print `... not created by the installer ...` and stop. The fix is the repair above.
- **Diagnosing an `Access is denied. (os error 5)` startup error.** Read the newest log with:

      Get-ChildItem -LiteralPath "$env:ProgramData\Miru\logs" -Filter "miru.log*" -File |
          Sort-Object LastWriteTime | Select-Object -Last 1 | Get-Content -Tail 50

  Use `-File`, because `logs` contains an `installer-sentinel` folder that `Get-Content` cannot read. A line `failed to write file atomically '<path>': Access is denied. (os error 5)` means the service cannot replace that file. `icacls "$env:ProgramData\Miru"` should list `NT SERVICE\miru-agent` (or its SID). If it does not, the folders were not created by the installer: repair the MSI as above. Do not use `icacls /reset`, which only re-inherits ProgramData's defaults.

No other log-tail snippet exists in this repository's docs (checked with `grep -rn "Get-Content" --include=*.md`, excluding `plans/`).

## Concrete Steps

Milestone 1:

    # after step 1 (tests extended):
    RUST_LOG=off cargo test --package miru-agent --test mod filesys::errors::display   # expect: atomic_write_file_err, read_dir_err FAIL
    # after steps 2-3:
    RUST_LOG=off cargo test --package miru-agent --test mod filesys                    # expect: all pass
    ./scripts/lint.sh                                                                 # expect: clean; re-check `git status` (it auto-fixes)
    git add agent/src/filesys/errors.rs agent/tests/filesys/errors.rs agent/tests/filesys/files.rs
    git commit -m "fix(filesys): include the OS error in atomic write and read-dir errors"

Milestone 2:

    RUST_LOG=off cargo test --package miru-agent --test mod provisioning disk::layout  # expect: new guard and layout tests pass
    cargo check --package miru-agent                                                  # Linux; the cfg(windows) call compiles in windows-check
    ./scripts/lint.sh
    git add agent/src/disk/layout.rs agent/src/provisioning agent/src/main.rs agent/tests/provisioning/shared.rs agent/tests/disk/layout.rs
    git commit -m "fix(windows): refuse to provision when the installer-owned data folders are missing"

Milestone 3 (needs an elevated, disposable Windows machine, so it is validated by the `windows-package` CI job, not locally):

    git add build/windows/tests/integration-lib.ps1
    git commit -m "test(windows): cover provisioning after ProgramData\Miru is wiped"

Milestone 4:

    git add build/windows/README.md
    git commit -m "docs(windows): explain resetting state and recovering from a wiped data folder"

Then run the `preflight` skill on the branch until it reports `CLEAN`. Preflight pushes, watches CI, and fixes failures from the CI logs.

## Validation and Acceptance

- **Linux and the `test` job:**
  - `filesys::errors::display::atomic_write_file_err` and `read_dir_err` fail before Milestone 1 and pass after.
  - `write_bytes::atomic_write_denied_reports_os_error` passes, and its text contains `Permission denied (os error 13)`.
  - The new `provisioning::shared` guard tests and the `disk::layout` sentinel test pass. The missing-layout test also shows that no `Miru` root was created.
  - `./scripts/covgate.sh` passes.
- **`windows-check`:** the same guard tests pass on Windows, `atomic_overwrite_blocked_by_reader_reports_os_error` passes, and the `#[cfg(windows)]` call in `main.rs` compiles.
- **`windows-package`:** the stage prints its `PASS` line, which means:
  - With `%ProgramData%\Miru` gone and the MSI installed, `provision` and `reprovision` exit 1 with `not created by the installer` and leave no `Miru` folder.
  - `msiexec /fvomus` restores the protected folders and sentinels.
  - Provision then gets past the guard and fails at the missing token.
  - The service rewrites administrator-written `device.json` to `"status": "offline"`.
  - The original folder is restored, and the uninstall stage still passes.
- **Manual (optional, reporter's device):** repeat the confirmed sequence. `provision` now refuses with the repair instruction. After `msiexec /fvomus` and provision, `Restart-Service miru-agent` starts cleanly.
- **Gate:** the PR stays in draft, and the task is not reported complete, until preflight reports `CLEAN`, meaning CI is green on the pushed branch head (`lint`, `test`, `windows-check`, `windows-package`, `tools`).

## Idempotence and Recovery

The Rust edits and tests can be run again safely. The harness stage is the risky step: it moves `%ProgramData%\Miru` on the CI machine. Its `finally` block always deletes the recreated folder and renames the backup back. If a run is killed in between, rename `%ProgramData%\Miru.harness-backup-*` back to `Miru` by hand, after stopping the service and deleting any recreated `Miru`. Revert any milestone with `git revert <sha>`.
