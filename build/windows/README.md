# Windows MSI packaging

This directory contains the pinned WiX project for a validated x64 Miru Agent
MSI. Native Windows compile CI was established in PR #234; PR #236 adds the
package contract and direct Windows Installer lifecycle validation.

The current executable is console-capable but is not a Windows Service Control
Manager executable. Accordingly, this MSI does not create, start, stop, or
remove a `MiruAgent` service. Service lifecycle, account selection, and recovery
configuration remain separate roadmap work.

## Package behavior

The MSI:

- installs `miru-agent.exe` under 64-bit `Program Files\Miru\Agent`;
- uses the permanent UpgradeCode `B5ED0336-5F14-4308-A667-3CE8CDEF7D48`;
- rejects downgrades and schedules major upgrades transactionally so a failed
  replacement can restore the previously installed package;
- protects `%ProgramData%\Miru` and its authored `logs` child with a non-inherited,
  inheritable DACL granting full control only to Local System and built-in
  Administrators, so state created after installation inherits that protection;
  and
- leaves populated customer state under `%ProgramData%\Miru` in place during
  maintenance, upgrades, rollback, and ordinary uninstall.

The UpgradeCode is part of the product's permanent identity and must never be
changed after publication. Each package version receives a different ProductCode.

## Build

Run these commands from the repository root on Windows 10 or 11 x64. The build
requires Git, the Rust MSVC toolchain with the `x86_64-pc-windows-msvc` target,
Visual Studio Build Tools with the C++ workload, NASM on `PATH`, and the .NET
SDK. Installation and integration testing additionally require an elevated
64-bit Windows PowerShell 5.1 session.

Restore the pinned WiX SDK, build the real Windows executable, and exercise the
package contract with explicit inputs:

```powershell
Set-Location C:\src\agent
cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked --release
dotnet restore build\windows\miru-agent.wixproj
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\package-tests.ps1 -ProjectPath build\windows\miru-agent.wixproj -BinDir target\x86_64-pc-windows-msvc\release -ArtifactsDirectory build\windows\artifacts\package-tests
```

`Version` is deliberately stricter than general SemVer. It must contain exactly
three numeric fields, with `MAJOR` and `MINOR` from 0 through 255 and `PATCH`
from 0 through 65535. Leading `v`, prerelease/build labels, and fourth fields are
not accepted at the MSI build boundary. `BinDir` must contain
`miru-agent.exe`. WiX is restored through the pinned `WixToolset.Sdk` 5.0.2
project; package validation is enabled and warnings fail the build.

## Install and provision

Run installation from an elevated 64-bit Windows PowerShell 5.1 session. The
current package is built and tested locally; publishing release artifacts and a
WinGet manifest remains follow-up work. Install a trusted MSI directly with
Windows Installer:

```powershell
$msi = "C:\path\to\miru-agent-1.0.0.msi"
$log = Join-Path $env:TEMP "miru-agent-install.log"
$arguments = @("/i", ('"{0}"' -f $msi), "/qn", "/norestart", "/l*v", ('"{0}"' -f $log))
$process = Start-Process -FilePath "msiexec.exe" -ArgumentList $arguments -Wait -PassThru
if (@(0, 3010) -notcontains $process.ExitCode) {
    throw "Installation failed with exit code $($process.ExitCode); see $log"
}
```

An install exit code of 0 means success. Exit code 3010 also means success, but
Windows must be restarted to complete the installation. Any other result is a
failure; inspect the verbose log. Published packages must be Authenticode-signed
before customer distribution.

Provision by placing the secret only in the process environment, then invoke the
installed executable directly. Do not put the token on the command line:

```powershell
$agent = Join-Path $env:ProgramW6432 "Miru\Agent\miru-agent.exe"
$tokenPointer = [IntPtr]::Zero
try {
    $secureToken = Read-Host "Provisioning token" -AsSecureString
    $tokenPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
    $env:MIRU_PROVISIONING_TOKEN = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($tokenPointer)
    & $agent provision
    if ($LASTEXITCODE -ne 0) { throw "Provisioning failed with exit code $LASTEXITCODE" }
} finally {
    if ($tokenPointer -ne [IntPtr]::Zero) {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($tokenPointer)
    }
    Remove-Item Env:MIRU_PROVISIONING_TOKEN -ErrorAction SilentlyContinue
}
& $agent provision --check
```

`provision --check` is read-only and returns 0 when provisioned, 3 when not
provisioned, and 1 when the state is undetermined or an error occurs.

## Validation

From an elevated 64-bit Windows PowerShell 5.1 session, run the native package
integration matrix only on a disposable test machine. Normal integration removes
allowlisted fixture products, changes `%ProgramData%\Miru`, and creates and
deletes a temporary local user, so the explicit confirmation is required:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ConfirmDisposableTestMachine
```

The matrix covers direct MSI install, maintenance, upgrade, downgrade rejection,
failed-upgrade rollback, uninstall, ACL repair, state retention, direct binary
provisioning check exit 3 on a fresh install, and the absence of a `MiruAgent`
service.
Maintenance, upgrade, rollback, and ordinary uninstall must retain customer
state, including customer-owned files under `%ProgramData%\Miru\logs`. The
root and `logs` DACLs must remain protected and permit inheritable full control
only for Local System and built-in Administrators; non-administrators must not
read sensitive state created after installation or create children.

For the production smoke pass, start from a disposable clean Windows 10 or 11
x64 VM snapshot with no installed Miru product. Build the production 1.0.0 and
1.1.0 packages, then run:

```powershell
Set-Location C:\src\agent
Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsBuildNumber
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ManualProductionSmoke -ConfirmDisposableCleanVm -TranscriptPath C:\Windows\Temp\miru-msi-smoke.txt
Get-FileHash C:\Windows\Temp\miru-msi-smoke.txt -Algorithm SHA256
```

Record the transcript hash with the validation evidence. The smoke pass must
record any 3010 reboot result and stage-specific PASS lines for install,
maintenance, upgrade, and uninstall. It confirms the no-service expectation,
repairs deliberately permissive root and `logs` ACLs, and leaves the retained
ProgramData sentinel and customer-owned log for inspection. Revert the VM
snapshot afterward rather than deleting retained customer state.

Authenticode signing of the executable and MSI remains deferred, along with the
GoReleaser/PDB release lane, artifact and WinGet publication, Windows service
lifecycle, account and recovery handling, full live-backend provisioning, and
Windows Server certification.
