# Windows MSI packaging

This directory contains the pinned WiX project for a validated x64 Miru Agent
MSI. Native Windows compile CI was established in PR #234; PR #236 adds the
package contract and the safe Windows PowerShell install and provisioning tools.

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
  Administrators; and
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
current supported workflow is to install a trusted MSI built locally, optionally
requiring its metadata version to match:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install\install.ps1 -FromMsi C:\path\to\miru-agent-1.0.0.msi -Version v1.0.0
```

An install exit code of 0 means success. Exit code 3010 also means success, but
Windows must be restarted to complete the installation. Any other result is a
failure; its error names the retained verbose installer log.

After Windows release artifacts and their checksum manifests are published, the
installer can download a stable release by version:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install\install.ps1 -Version v1.0.0
```

For that future download workflow, the MSI is matched to an exact SHA-256
checksum record. Checksums detect corruption but do not authenticate the
publisher.

Provision by placing the secret only in the process environment, then invoke the
wrapper. Do not put the token on the command line:

```powershell
$tokenName = "MIRU_PROVISIONING_TOKEN"
$processEnvironment = [Environment]::GetEnvironmentVariables([EnvironmentVariableTarget]::Process)
$callerHadToken = $processEnvironment.Contains($tokenName)
$callerToken = if ($callerHadToken) { [string]$processEnvironment[$tokenName] } else { $null }
$tokenPointer = [IntPtr]::Zero
try {
    $secureToken = Read-Host "Provisioning token" -AsSecureString
    $tokenPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
    $env:MIRU_PROVISIONING_TOKEN = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($tokenPointer)
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install\provision.ps1
} finally {
    if ($tokenPointer -ne [IntPtr]::Zero) {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($tokenPointer)
    }
    if ($callerHadToken) {
        [Environment]::SetEnvironmentVariable($tokenName, $callerToken, [EnvironmentVariableTarget]::Process)
    } else {
        [Environment]::SetEnvironmentVariable($tokenName, $null, [EnvironmentVariableTarget]::Process)
    }
}
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install\provision.ps1 -Check
```

The provisioning wrapper invokes the installed executable directly and restores
the prior process environment exactly. `-Check` is read-only and returns 0 when
provisioned, 3 when not provisioned, and 1 when the state is undetermined or an
error occurs.

## Validation

Run the dependency-free parser and focused script checks under Windows
PowerShell 5.1:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\parse-scripts.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\script-tests.ps1
```

From an elevated 64-bit Windows PowerShell 5.1 session, run the native package
integration matrix only on a disposable test machine. Normal integration removes
allowlisted fixture products, changes `%ProgramData%\Miru`, and creates and
deletes a temporary local user, so the explicit confirmation is required:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ConfirmDisposableTestMachine
```

The matrix covers initial install, maintenance, upgrade, downgrade rejection,
failed-upgrade rollback, uninstall, ACL repair, state retention, provisioning
check exit 3 on a fresh install, and the absence of a `MiruAgent` service.
Maintenance, upgrade, rollback, and ordinary uninstall must retain customer
state, including customer-owned files under `%ProgramData%\Miru\logs`. The
root and `logs` DACLs must remain protected and permit inheritable full control
only for Local System and built-in Administrators; non-administrators must not
read sensitive state or create children.

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
GoReleaser/PDB release lane, artifact publication, Windows service lifecycle,
account and recovery handling, full live-backend provisioning, and Windows
Server certification.
