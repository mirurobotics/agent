# Windows service hardening follow-ups: log-open failure, protected-folder sentinels, customer-grant retention

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (GitHub `mirurobotics/agent`) | read-write | Rust logging and provisioning code and tests, the WiX installer, the PowerShell MSI test harness, `build/windows/README.md`, and a note in one completed plan. |

This plan lives in `agent/plans/` because every change is in the agent repo. Work on branch `fix/windows-service-hardening-followups` (already created from `main` at `6aeb279`); the PR targets `main`. All paths are relative to the agent repo root, and every command runs from that root.

## Purpose / Big Picture

PR #263 made the Windows service run as the low-privilege virtual account `NT SERVICE\miru-agent`. This plan closes three follow-ups from that PR.

1. If the service cannot open its log file, it currently panics, and Windows kills it without a status or exit code; the restart-on-failure actions then restart it in a loop. After this change it stops cleanly and reports service-specific exit code 1: `sc.exe query miru-agent` shows `STOPPED` with `SERVICE_EXIT_CODE : 1`, and the System event log records event 7024. Provisioning and Linux get the same error instead of a panic.
2. A compromised service can currently empty `%ProgramData%\Miru\logs`, `auth`, or `tmp` and turn the empty folder into a mount point, which a later administrator-run repair or upgrade would follow. After this change, each of those folders holds an installer-owned `installer-sentinel` folder that the service cannot delete, so the folder can never be emptied. Provisioning also stops deleting `tmp\`.
3. The integration matrix proves that a customer's grant to the service on a folder outside `%ProgramData%\Miru` survives repair, upgrade, rollback, and uninstall, and stays keyed to the SID that the reinstalled service runs as.

## Progress

- [x] Milestone 1: log-open failure returns an error instead of panicking.
- [x] Milestone 2: provisioning deletes only its key files in `tmp\`.
- [ ] Milestone 3: installer sentinels and harness checks.
- [ ] Milestone 4: customer-grant retention check.
- [ ] Milestone 5: documentation.
- [ ] Milestone 6: push, draft PR, preflight `CLEAN`.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

(Add entries as work proceeds. Design choices made while authoring are under "Design choices" in Context and Orientation.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

**Terms.** The *SCM* (Service Control Manager) starts Windows services. A *virtual account* `NT SERVICE\<name>` is a per-service identity whose *service SID* is derived from the service name; for `miru-agent` it is `S-1-5-80-1251439239-454917380-1008020685-2030257057-91624695` (`sc.exe showsid miru-agent`). *SDDL* is the text form of a security descriptor: `O:SY` sets owner SYSTEM, `D:P` starts a protected DACL (no inheritance from the parent), `(A;OICI;FA;;;SY)` allows full access to SYSTEM, inherited by files (`OI`) and subfolders (`CI`); `BA` is built-in Administrators; `IO` makes an ACE inherit-only. `FRFWFX` is generic read, write, and execute, which on a folder includes list, add file, add subfolder, and write attributes but not `DELETE` or `FILE_DELETE_CHILD`. A *junction* (mount point) is a folder with an `IO_REPARSE_TAG_MOUNT_POINT` *reparse point*: opening a path through it goes to the target folder instead. `FSCTL_SET_REPARSE_POINT` creates one; it needs no privilege, only `FILE_WRITE_DATA` and `FILE_WRITE_ATTRIBUTES` on the folder, and the folder must be empty.

**Item 1: logging.** `agent/src/logs/mod.rs` `build_layers` calls `tracing_appender::rolling::hourly(options.log_dir, "miru.log")`. In `tracing-appender` 0.2.5 (the version in `Cargo.lock`), `hourly` is `RollingFileAppender::new(Rotation::HOURLY, dir, prefix)`, which is `RollingFileAppender::builder().rotation(Rotation::HOURLY).filename_prefix(prefix).build(dir).expect(...)`. So it panics when the current-hour file `miru.log.YYYY-MM-DD-HH` cannot be opened or its folder cannot be created. `build` returns `Result<RollingFileAppender, tracing_appender::rolling::InitError>`, and using it directly produces identical file names. Later hourly rotation does not panic (a failed rotation prints to stderr and keeps the old file). `build_layers` opens the file even when `options.stdout` is true; keep that. `logs::init` calls `build_layers` and then `set_global_default`; `LogsErr` has variants `SetGlobalDefault` and `ReloadFailed`.

`agent/src/main.rs` `run_agent` already maps `logs::init` errors to `eprintln!` plus `RunOutcome::Failed`; that path never runs today because the panic comes first. Under the SCM, `windows_service_body` runs inside the `extern "system"` thunk generated by `define_windows_service!`, so a panic aborts the process. `agent/src/windows/scm.rs` `run_lifecycle` reports `StartPending`, then `Running`, runs the body, then `StopPending` and `Stopped` with `exit_code(outcome)`, which maps `RunOutcome::Failed` to `ServiceExitCode::ServiceSpecific(1)` (unit-tested in `scm.rs`). No `scm.rs` change is needed. `run_provision` and `run_reprovision` in `main.rs` call `logs::init(...)?` with a fresh temp folder; `ProvisionErr::LogsErr` (`agent/src/provisioning/errors.rs`) wraps any `LogsErr`, so a new variant needs no change there. On Linux, `launch_agent` discards the foreground `RunOutcome`, so a log-open failure now exits 0 after printing to stderr (the systemd journal), where the panic exited 101 (see Design choices). `build/debian/miru.service` has no `Restart=`; in both cases the next connection to the activation socket `/run/miru/miru.sock` (`build/debian/miru.socket`) starts it again.

Tests for `logs` live in `agent/tests/logs/mod.rs` (integration target `mod`, from `agent/tests/mod.rs`). Five tests call `logs::build_layers(options)` and destructure the returned 4-tuple; the helper `build_layers_tempdir(prefix)` returns a `test_dirs::TempDir` guard and its `PathBuf`. `agent/src/logs/.covgate` is 95.48.

**Item 2: installer and provisioning.** `build/windows/miru-agent.wxs` component group `DataDirs` has four directory-keyed components (`MiruDataDir`, `MiruLogsDir`, `MiruAuthDir`, `MiruTmpDir`) for folders `MIRUDATA` (`%ProgramData%\Miru`), `MIRULOGS`, `MIRUAUTH`, `MIRUTMP`. Each has `<CreateFolder><PermissionEx Sddl="O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFWFX;;;<SID>)(A;OICIIO;FA;;;<SID>)" /></CreateFolder>`, which compiles to `MsiLockPermissionsEx` rows. Windows Installer reapplies that descriptor to the folder by path on every install, repair, and upgrade (the harness proves this on folders pre-loosened with a hostile owner), but not to existing children. The service therefore cannot delete, rename, or re-permission these four folders, but it has inherited full control, including `DELETE`, on every child.

The residual risk recorded in `plans/completed/20260928-windows-virtual-service-account.md` (Decision Log F3 and Outcomes): a compromised service can delete everything in `logs\` or `auth\`, then turn the empty folder into a mount point, and the next administrator-initiated repair or upgrade would follow it when it reapplies the descriptor (which grants the service SID inheritable full control) by path. `tmp\` is exposed as well: the administrator-run `miru-agent provision` and `reprovision` delete `tmp\` itself (`agent/src/provisioning/shared.rs` `cleanup_temp_dir` calls `dirs::delete`, a `remove_dir_all`). The service, which can add subfolders to the root, can then create a new `tmp` that it owns.

Provisioning (`agent/src/provisioning/provision.rs` `provision`, `agent/src/provisioning/reprovision.rs` `reprovision`) takes `temp_dir = layout.temp_dir()`, writes `temp_dir.file("private.key")` and `temp_dir.file("public.key")` with `rsa::gen_key_pair` (an atomic write that creates the parent folder if missing), and calls `disk::setup::bootstrap` (`agent/src/disk/setup.rs`). That moves both keys into `auth\` as `private_key.pem` and `public_key.pem`, then `reset` rewrites `device.json`, `settings.json`, and `auth\token.json` and deletes and recreates `resources\` and `events\`. It never deletes `auth\`, `logs\`, or the root. After that, `shared::cleanup_temp_dir(&temp_dir)` deletes `tmp\`. On success the keys are already gone; on failure they remain in `tmp\`. `files::delete` (`agent/src/filesys/files.rs`) removes one file and treats "not found" as success. No other agent code enumerates or cleans `logs\`, `auth\`, or `tmp\`. `tracing-appender` prunes nothing (no `max_files`). The upgrade reconcile only resets root children. File-rule scanning (`files::glob`) returns regular files only and skips unreadable folders, so a customer file rule over `logs\` ignores a sentinel folder.

Provisioning tests: `agent/tests/provisioning/shared.rs` defines `Env` (a temp-folder `Layout` with `Env::new(prefix)`), `validate_storage`, and `StorageSnapshot`. The assertion `assert!(!layout.temp_dir().exists(), "temp dir not cleaned")` appears in `validate_storage`, in `provision.rs` tests `http_error_aborts_provision` and `http_error_preserves_existing_storage`, and in `reprovision.rs` test `http_error_preserves_existing_storage`. `agent/src/provisioning/.covgate` is 96.57.

**Harness.** CI job `windows-package` in `.github/workflows/ci.yml` runs on `windows-latest` for pull requests touching `build/windows/**`, `.github/workflows/ci.yml`, or `.github/workflows/release.yml`. It runs `build/windows/tests/package-tests.ps1` and then `build/windows/tests/integration-tests.ps1`, which dot-sources `integration-lib.ps1` and imports `MsiTest.psm1` (shared constants, exported by an explicit `Export-ModuleMember` list at the end of the file). The Rust tests run in the `test` (Linux) and `windows-check` jobs. PowerShell and WiX are not available on the Linux dev host, so CI validates the Windows changes.
- `package-tests.ps1` `Assert-ProtectedPermissionRows` requires the `MsiLockPermissionsEx` rows to equal exactly one row per `$MsiExpectedDirectories` entry with `$MsiExpectedSddl`.
- `integration-tests.ps1` `Initialize-IntegrationPaths` defines `$protectedRoots` (the four folders), `$customerOwnedFiles`, and `$artifactsRoot` (a unique folder under `%TEMP%` that `Remove-TestFiles` deletes at the end). `Initialize-IntegrationRuntime` initializes script state.
- `integration-lib.ps1` `Invoke-IntegrationLifecycle` runs the install, maintenance (`REINSTALL=ALL REINSTALLMODE=vomus`), upgrade (v1 to v2), downgrade rejection, rollback (a failing v3 upgrade), and uninstall stages (a failing uninstall, then a real one). `Add-PermissiveAces` (before install, maintenance, and upgrade) creates each protected folder if missing and gives it a hostile owner and an Everyone full-control DACL, so the installer must repair it. `Assert-ProtectedState` (after every stage, including uninstall) calls `Assert-CustomerStateRetained` and `Assert-ProtectedAcls` (which calls `Assert-ProtectedAcl` on each protected folder: owner SYSTEM, protected, four ACEs). `New-RepresentativeFiles` and `Assert-InheritedProtection` check fresh files inside each protected folder. Helpers include `Assert-FullControlAce` (with `-Inheritable` for `OICI`), `Get-RuleSid`, `Assert-True`, and `Assert-Equal`.

**Design choices.**
- The sentinel is a folder, not a file. A folder authored with `CreateFolder` plus `PermissionEx` gets its descriptor reapplied on every install, repair, and upgrade, the mechanism the harness already proves on hostile pre-existing folders, and it needs no payload. An MSI file needs a checked-in `Source`, and file-versioning and hash rules can skip reinstalling an existing unversioned file (for example, a planted zero-byte file with a matching hash), which would leave that file's descriptor unapplied.
- The sentinel name is `installer-sentinel`, created in `logs\`, `auth\`, and `tmp\`. Its SDDL is `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`, with no service ACE. The service has no `DELETE` on the sentinel, and the parent's `FRFWFX` excludes `FILE_DELETE_CHILD`, so the service can neither delete nor rename the sentinel. The parent therefore stays non-empty and cannot become a mount point, and the service cannot write to the sentinel to make it one. `OICI` matches the existing SYSTEM and Administrators ACEs, so `Assert-FullControlAce -Inheritable` applies unchanged.
- The root needs no sentinel: it always contains `logs\`, `auth\`, and `tmp\`, which the service cannot delete (no `DELETE` on them, no `FILE_DELETE_CHILD` on the root), and after this change nothing the service can trigger deletes `tmp\`.
- The sentinel components are `Permanent="yes"`, so no uninstall, major-upgrade removal, or rollback removes them; the harness then asserts their presence and descriptor after every stage, uniformly. `NeverOverwrite` applies only to files and is not used. The components need explicit GUIDs (directory key paths cannot use `Guid="*"`).
- Provisioning deletes only `tmp\private.key` and `tmp\public.key`, on every platform. On Linux, `/var/lib/miru/tmp` now stays (empty) after provisioning; nothing depends on its absence.
- Only the existing exact-rows package test changes (it would otherwise fail). No new static package checks; the integration matrix proves the sentinels on a real install.
- No test deletes as the service identity: starting a process as a virtual account is not cheap from the harness. The sentinel DACL check, together with the existing exact `FRFWFX` mask check on each parent (`Assert-ServiceDirectoryAces`), proves the service has no delete right.
- The customer-grant folder lives under `$artifactsRoot`, so the existing `Remove-TestFiles` cleans it up. It is granted by account name, as in the README recipe, after the install stage creates the service.
- A log-open failure now leaves the Windows service stopped with exit code 1 instead of crash-looping. The SCM runs failure actions only for crashes unless the non-crash failure flag is set, which the MSI does not set; this plan leaves that unchanged.
- On Linux the foreground failure exits 0 (systemd shows the unit inactive rather than failed), like the existing settings-read and server-error `RunOutcome::Failed` paths. Making the foreground path exit 1 on `Failed` would change those paths too, so it is left as a one-line follow-up.
- The customer-grant check does not prove the MSI leaves the folder alone (it never touches it). Together with the existing `Assert-ServiceSidIdentity` and `Assert-ServiceRuntimeIdentity` checks, it proves that the grant stays keyed to the SID the repaired, upgraded, or rolled-back service runs as.
- In the harness, "installer sentinel" (`$installerSentinelDirs`, `Assert-InstallerSentinelAcl`) is distinct from the existing customer-owned file `integration-sentinel.txt`.

**Residual risk after this plan** (for Outcomes and the completed-plan note). (a) If an administrator deletes an `installer-sentinel` or a whole protected folder, the original risk returns until the next repair or upgrade recreates it, and that repair could itself follow a mount point created in the meantime. The README says not to delete them. (b) Out of scope: the administrator-run `provision` and `reprovision` still write inside folders the service can modify (the root, `auth\`, `tmp\`). The `atomicwrites` crate creates a `.atomicwrite*` temporary subfolder next to each target, and that subfolder inherits the service's full control, so a compromised service that wins a race could make it a mount point before the elevated process writes `tmpfile.tmp` into it. Closing that requires provisioning to stop writing as an administrator into service-writable folders. No MSI has shipped, so no installed version lacks the sentinels.

## Plan of Work

**Milestone 1: log-open failure (Rust).** In `agent/src/logs/mod.rs`:
- Add to `LogsErr`: `#[error("failed to open log file: {0}")] OpenLogFile(#[from] tracing_appender::rolling::InitError),`.
- Change `build_layers` to return `Result<(BoxedLogLayer, WorkerGuard, ReloadHandle, bool), LogsErr>`. Replace the `hourly` call with:

        let file_appender = RollingFileAppender::builder()
            .rotation(Rotation::HOURLY)
            .filename_prefix("miru.log")
            .build(options.log_dir)?;

  Import `tracing_appender::rolling::{RollingFileAppender, Rotation}` in the external-crates group, and wrap the final tuple in `Ok(...)`. If clippy reports `type_complexity`, add a private `type Layers = (BoxedLogLayer, WorkerGuard, ReloadHandle, bool);` alias.
- In `init`, use `build_layers(options)?`.

In `agent/tests/logs/mod.rs`, append `.unwrap()` to the five existing `logs::build_layers(options)` calls, and add one test after `test_build_layers_file_only_warn`:

    #[tokio::test]
    async fn test_build_layers_returns_open_log_file_err_when_dir_is_a_file() {
        let (_tmp, root) = build_layers_tempdir("miru_test_build_layers_bad_dir").await;
        let log_dir = root.join("not-a-dir");
        std::fs::write(&log_dir, b"").unwrap();
        let options = Options {
            stdout: false,
            log_level: LogLevel::Info,
            log_dir,
        };
        assert!(matches!(
            logs::build_layers(options),
            Err(LogsErr::OpenLogFile(_))
        ));
    }

It asserts only the variant, never message text. It is portable (opening a file under a regular file fails and so does `create_dir_all`), so it also runs in `windows-check`. Do not test through `logs::init`: a regression there would install a process-global subscriber in the shared test binary.

**Milestone 2: provisioning tmp cleanup (Rust).** In `agent/src/provisioning/shared.rs`, replace `cleanup_temp_dir` with:

    // tmp\ itself is installer-owned on Windows; delete only what provisioning wrote.
    pub(super) async fn cleanup_temp_files(temp_files: &[&filesys::File]) {
        for file in temp_files {
            if let Err(e) = files::delete(file).await {
                debug_assert!(false, "failed to clean up temp file: {e}");
                warn!("failed to clean up temp file: {e}");
            }
        }
    }

Change the import `crate::filesys::{self, dirs}` to `crate::filesys::{self, files}`. In `provision.rs` and `reprovision.rs`, move the `private_key_file` and `public_key_file` bindings out of the `async` block, above it, and replace `shared::cleanup_temp_dir(&temp_dir)` with `shared::cleanup_temp_files(&[&private_key_file, &public_key_file])`.

Tests, in `agent/tests/provisioning/shared.rs`:
- Add `pub(super) const TEMP_SENTINEL: &str = "installer-sentinel";`, with a one-line comment saying that it stands in for the Windows installer's sentinel folder in `tmp\`.
- In `Env::new`, after building the layout, run `dirs::create_if_absent(&layout.temp_dir().subdir(TEMP_SENTINEL)).await.unwrap();` (import `miru_agent::filesys::dirs`).
- Add `pub(super) fn assert_temp_dir_cleaned(layout: &Layout)`. It collects the entry names of `layout.temp_dir()` with `std::fs::read_dir` and asserts that they equal `vec![TEMP_SENTINEL.to_string()]`: the keys are gone, and `tmp\` and the sentinel remain.
- In `validate_storage`, replace the `!layout.temp_dir().exists()` assertion with `assert_temp_dir_cleaned(layout)`. Replace the same assertion in the three tests named in Context (`provision.rs` twice, `reprovision.rs` once) with `assert_temp_dir_cleaned(&env.layout)`, and import it from `super::shared`.

This adds no test functions. Existing success and failure tests now prove that the sentinel survives and that the key files are removed.

**Milestone 3: installer sentinels and harness.** In `build/windows/miru-agent.wxs`:
- Nest `<Directory Id="MIRULOGSSENTINEL" Name="installer-sentinel" />` in `MIRULOGS`, `MIRUAUTHSENTINEL` in `MIRUAUTH`, and `MIRUTMPSENTINEL` in `MIRUTMP` (turn the three self-closing `Directory` elements into open and close pairs).
- Add three components to `DataDirs`:

        <Component Id="MiruLogsSentinel" Directory="MIRULOGSSENTINEL" Guid="51B41B07-B719-4D73-BD1E-3675FC8E7CA8" Bitness="always64" KeyPath="yes" Permanent="yes">
          <CreateFolder>
            <PermissionEx Sddl="O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)" />
          </CreateFolder>
        </Component>

  Use `MiruAuthSentinel` / `MIRUAUTHSENTINEL` / `6CDA574C-42E3-40A3-92C4-BF304E2F368A` and `MiruTmpSentinel` / `MIRUTMPSENTINEL` / `7AF370AB-A333-4D60-BC95-ACAB7306D1C6` for the other two. Never change these GUIDs after release.
- Extend the XML comment above `DataDirs` with one sentence: the sentinels, which the service cannot delete, keep `logs`, `auth`, and `tmp` non-empty so that none can become a mount point that a repair or upgrade follows; they are permanent.

In `build/windows/tests/MsiTest.psm1`, add `$MsiSentinelName = "installer-sentinel"`, `$MsiSentinelSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"`, and `$MsiSentinelDirectoryIds = @("MIRULOGSSENTINEL", "MIRUAUTHSENTINEL", "MIRUTMPSENTINEL")`, and add all three to the `-Variable` export list.

In `build/windows/tests/package-tests.ps1` `Assert-ProtectedPermissionRows`, build `$expected` from both sources before sorting: the existing `$MsiExpectedDirectories` rows plus `"$_|CreateFolder|$MsiSentinelSddl"` for each `$MsiSentinelDirectoryIds` entry.

In `build/windows/tests/integration-tests.ps1` `Initialize-IntegrationPaths`, add `$script:installerSentinelDirs = @($script:logsRoot, $script:authRoot, $script:tmpRoot | ForEach-Object { Join-Path $_ $MsiSentinelName })`.

In `build/windows/tests/integration-lib.ps1`:
- `Add-PermissiveAces`: loop over `@($protectedRoots) + @($installerSentinelDirs)`. `Set-PermissiveAcl` creates each sentinel before install, so every install, maintenance, and upgrade starts from a hostile pre-existing sentinel that the installer must repair.
- `Assert-ProtectedAcls`: after the existing loop, add `foreach ($path in $installerSentinelDirs) { Assert-InstallerSentinelAcl $path }`. This runs in every `Assert-ProtectedState`, so presence and DACL are checked after every stage, including uninstall (retention).
- Add `Assert-InstallerSentinelAcl`, preceded by the one-line comment `# SYSTEM and Administrators only: the service cannot delete it, so it cannot empty the parent.` The function asserts:

        Test-Path -LiteralPath $LiteralPath -PathType Container
        ((Get-Item -LiteralPath $LiteralPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0
        owner S-1-5-18; AreAccessRulesProtected
        exactly 2 ACEs, each not IsInherited and passing Assert-FullControlAce $_ $LiteralPath -Inheritable
        sorted SIDs joined with "," equal "S-1-5-18,S-1-5-32-544"

- `Remove-TestFiles`, `Initialize-CustomerState`, the non-admin probe, `Assert-ProtectedAcl`, and the representative-file checks need no change. The parents' own DACLs are unchanged, representative files are created directly in the parents, and `Get-AgentLogFiles` filters `miru.log*` files. Leaving permanent sentinels on the disposable runner is expected, and a re-run on the same machine re-loosens them in `Add-PermissiveAces`.

**Milestone 4: customer-grant retention.** In `integration-tests.ps1` `Initialize-IntegrationRuntime`, add `$script:customerGrantPath = $null`. In `integration-lib.ps1`:
- Add `New-CustomerGrant`. It sets `$script:customerGrantPath = Initialize-Directory (Join-Path $artifactsRoot "customer-grant")`, runs `& icacls.exe $customerGrantPath /grant "$MsiServiceAccount`:(OI)(CI)M" | Out-Null`, asserts `$LASTEXITCODE` is 0, and calls `Assert-CustomerGrant "grant"`.
- Add `Assert-CustomerGrant $Stage`. Among the explicit (not `IsInherited`) ACEs whose `Get-RuleSid` equals `$MsiServiceSid`, it asserts: exactly one; `Allow`; `([int]$rule.FileSystemRights -band [int]Modify) -eq [int]Modify`; and `InheritanceFlags` equal to `ContainerInherit -bor ObjectInherit`. `Get-RuleSid` also works after uninstall, when `Get-Acl` reports the orphaned ACE as a raw SID.
- In `Invoke-IntegrationLifecycle`, call `New-CustomerGrant` right after `Invoke-InstallStage`.
- In `Assert-CustomerStateRetained`, add `if ($null -ne $customerGrantPath) { Assert-CustomerGrant $Stage }`, so the grant is checked after maintenance, upgrade, downgrade rejection, rollback, and uninstall. `Remove-TestFiles` already deletes `$artifactsRoot`.

**Milestone 5: documentation.** In `build/windows/README.md`:
- In "Package behavior", before the retention bullet, add a bullet. The MSI creates an `installer-sentinel` folder in each of `logs`, `auth`, and `tmp`, owned by Local System and accessible only to Local System and Administrators, so the service can never empty those folders. The MSI reapplies the sentinels' permissions on repair and upgrade and never removes the sentinels, even on uninstall. Do not delete them while the agent is installed. Also amend the retention bullet: uninstall always leaves the `%ProgramData%\Miru` folder tree, and after uninstalling, an administrator can delete `%ProgramData%\Miru` to remove all remaining state.
- After the rollback paragraph, add one sentence: if the service cannot open its log file, it stops with service-specific error 1 (System event 7024) instead of crashing, so the restart-on-failure actions do not apply.
- In "Validation", add that the sentinels are checked after every stage, including uninstall, and that a folder outside `%ProgramData%\Miru` granted Modify to the service keeps the grant through maintenance, upgrade, rollback, and uninstall.

In `plans/completed/20260928-windows-virtual-service-account.md`, append to Decision Log entry F3: `(Superseded 2026-09-30: installer sentinels close this for logs\, auth\, and tmp\; see plans/completed/20260930-windows-service-hardening-followups.md.)`. In Outcomes, change the F10 and junction-hardening follow-ups to say they were done in that plan, and state residual risks (a) and (b) from Context in one sentence each.

## Concrete Steps

Before Milestone 1, activate the plan:

The plan file is untracked when authored, so move it and add it (skip if it is already in `plans/active/`):

    mv plans/backlog/20260930-windows-service-hardening-followups.md plans/active/
    git add plans/active/20260930-windows-service-hardening-followups.md
    git commit -m "docs(plans): activate windows service hardening follow-ups plan"

Milestone 1. After the edits:

    cargo fmt -p miru-agent
    RUST_LOG=off cargo test --package miru-agent --test mod -- logs::

Expected: every `logs::` test passes, including `logs::test_build_layers_returns_open_log_file_err_when_dir_is_a_file`. Before the change that test cannot compile (`build_layers` returns a tuple); with only the test adapted, it panics inside `tracing-appender` with `initializing rolling file appender failed`. Then:

    cargo clippy --package miru-agent --all-features -- -D warnings
    git add agent plans && git commit -m "fix(logs): return an error instead of panicking when the log file cannot be opened"

Milestone 2. After the edits:

    cargo fmt -p miru-agent
    RUST_LOG=off cargo test --package miru-agent --test mod -- provisioning::
    cargo clippy --package miru-agent --all-features -- -D warnings
    git add agent plans && git commit -m "fix(provisioning): delete only generated key files from the temp dir"

Expected: all `provisioning::` tests pass. With the old `cleanup_temp_dir`, `assert_temp_dir_cleaned` fails because `read_dir` on the deleted `tmp` errors.

Milestone 3. Check that the WiX file is well-formed and has the three sentinels, then commit:

    python3 -c 'import xml.dom.minidom as m; m.parse("build/windows/miru-agent.wxs")'           # expect no output
    grep -c 'Sddl="O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"' build/windows/miru-agent.wxs   # expect 3
    grep -c 'Permanent="yes"' build/windows/miru-agent.wxs                                   # expect 3
    git diff --check                                                                          # expect no output
    git add build/windows plans && git commit -m "feat(windows): add installer-owned sentinels to protected folders"

Milestone 4:

    git add build/windows plans && git commit -m "test(windows): assert a customer grant survives repair and upgrade"

Milestone 5:

    git add build/windows plans && git commit -m "docs(windows): document installer sentinels and log-open failure"

Milestone 6. Write a PR body file (for example `/tmp/pr-body.md`) with a short summary and the manual test plan from Validation and Acceptance, then open a draft PR:

    git push -u origin fix/windows-service-hardening-followups
    gh pr create --draft --base main --title "fix(windows): service log-open failure, protected-folder sentinels, grant retention" --body-file /tmp/pr-body.md

If a PR for the branch already exists, set its body with `gh api -X PATCH repos/mirurobotics/agent/pulls/<number> -F body=@/tmp/pr-body.md` (`gh pr edit` fails in this organization). Run the `$preflight` agent skill until it reports `CLEAN`. Confirm that `windows-package` ran and was not skipped (it runs because `build/windows/**` changed):

    gh pr checks --watch
    gh run view <run-id> --log | grep -E "PASS |ASSERT"

Expect every `PASS` stage line (two package versions, fixture contract, invalid inputs, install, maintenance, upgrade, downgrade, rollback, uninstall) and no `ASSERT` line. Then update Progress, fill Outcomes & Retrospective, move this plan to `plans/completed/`, commit, push, and re-confirm preflight `CLEAN` on that head.

## Validation and Acceptance

Automated acceptance on the pushed head:
- `test` (Linux) and `windows-check` (Windows) pass, including `logs::test_build_layers_returns_open_log_file_err_when_dir_is_a_file` and all `provisioning::` tests. After every provision and reprovision, success or failure, `tmp` contains exactly `installer-sentinel`. The `lint` job passes, and `scripts/covgate.sh` keeps `logs` at 95.48 or higher and `provisioning` at 96.57 or higher.
- `windows-package` passes. The package test sees exactly seven `MsiLockPermissionsEx` rows: four protected folders with the four-ACE SDDL and three sentinels with `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`. The integration matrix pre-loosens each sentinel with a hostile owner and an Everyone ACE before install, maintenance, and upgrade. After install, maintenance, upgrade, downgrade rejection, rollback, and uninstall, each `logs\installer-sentinel`, `auth\installer-sentinel`, and `tmp\installer-sentinel` exists, is not a reparse point, is owned by SYSTEM, and has a protected DACL with exactly SYSTEM and Administrators full control. The parent-folder, representative-file, non-admin, and service assertions pass unchanged. After maintenance, upgrade, downgrade rejection, rollback, and uninstall, the `customer-grant` folder still has one explicit Modify `(OI)(CI)` ACE for the service SID.

**Completion gate:** preflight must report `CLEAN` (CI green on the pushed branch head) before the PR leaves draft or the task is reported complete. A red or skipped `windows-package` job blocks completion.

Manual test plan (copy into the PR description; needs a Windows 10 or 11 x64 machine and an elevated PowerShell):
1. Fresh install of this branch's MSI. `Get-ChildItem -Force "$env:ProgramData\Miru\logs","$env:ProgramData\Miru\auth","$env:ProgramData\Miru\tmp"` lists `installer-sentinel` in each; `icacls "$env:ProgramData\Miru\tmp\installer-sentinel"` shows only `NT AUTHORITY\SYSTEM:(OI)(CI)(F)` and `BUILTIN\Administrators:(OI)(CI)(F)`.
2. Provisioning with sentinels present. Run `miru-agent provision` (README flow): it prints `Successfully provisioned this device as ...`. `tmp\` then contains only `installer-sentinel` (no `private.key` or `public.key`), and `auth\` has `private_key.pem`, `public_key.pem`, and `token.json`. Run `miru-agent reprovision` with a new token: it prints `Successfully reprovisioned this device as ...`, `auth\private_key.pem` changes, and all three sentinels are still present with the same `icacls` output. The device shows online in the Miru dashboard, and the newest `logs\miru.log.*` contains `Running the server with options`.
3. Log-open failure. Run `Stop-Service miru-agent` and `icacls "$env:ProgramData\Miru\logs" /deny "NT SERVICE\miru-agent:(OI)(CI)(W)"`, then `Start-Service miru-agent` (it may report that the service stopped). Within a few seconds, `sc.exe query miru-agent` shows `STATE : 1 STOPPED`, `WIN32_EXIT_CODE : 1066`, and `SERVICE_EXIT_CODE : 1`. `Get-WinEvent -FilterHashtable @{LogName='System'; Id=7024} -MaxEvents 1` names Miru Agent, there is no new 7031 or 7034 crash event, and the service does not restart. Undo with `icacls "$env:ProgramData\Miru\logs" /remove:d "NT SERVICE\miru-agent"` and `Start-Service miru-agent`; it runs.
4. Customer grant. Grant `C:\srv\miru` with the README recipe, then upgrade to a higher-version MSI and run a repair (`msiexec /fa <msi>`). `icacls C:\srv\miru` still shows `NT SERVICE\miru-agent:(OI)(CI)(M)`, and a config deploy to it succeeds.
5. Uninstall: the service is removed; the three sentinels remain with the same `icacls` output.

## Idempotence and Recovery

All edits are text edits and can be re-applied. CI runs on disposable runners, and the integration script refuses to run without `-ConfirmDisposableTestMachine`. Risky points:
- **WiX or ICE rejects `Permanent` on the sentinel components, or CI shows a permanent sentinel's descriptor is not reapplied on upgrade** (the `Assert-InstallerSentinelAcl` check after the upgrade stage fails with the hostile owner). Drop `Permanent="yes"`. Then the old product's uninstall removes the empty sentinel while the service is stopped, and the new install recreates it. Change the uninstall stage to assert that the sentinels are absent after uninstall (skip `Assert-InstallerSentinelAcl` there), and record the choice in the Decision Log.
- **clippy `type_complexity` on `build_layers`**: add the `Layers` type alias described in Milestone 1.
- **A provisioning test sees an extra `tmp` entry** (for example, an `.atomicwrite*` folder left by a failed write): record it in Surprises & Discoveries. Do not widen `assert_temp_dir_cleaned`; a leftover entry is a real cleanup gap.

To roll back entirely, revert the milestone commits. Windows support has not shipped, so no installed MSIs need migrating. Reverting does not remove permanent sentinels from a machine where a branch MSI was installed, and neither does uninstall. After uninstalling, delete them as an administrator (`Remove-Item -Recurse -Force "$env:ProgramData\Miru"`); the leftover component registration is harmless. The sentinel GUIDs above must stay fixed once any MSI with them is published.
