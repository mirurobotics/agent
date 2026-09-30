# Windows MSI packaging

This directory contains the pinned WiX project for a validated x64 Miru Agent
MSI. Native Windows compile CI was established in PR #234; PR #236 adds the
package contract and direct Windows Installer lifecycle validation.

The MSI installs `miru-agent.exe` as a Windows service named `miru-agent`
(display name "Miru Agent", description "Miru Config Agent"). The service runs
as the virtual account **`NT SERVICE\miru-agent`** with start type
**Automatic**; it is started on install and stopped and removed on uninstall.
Major upgrades stop and delete the old service before file replacement, then
install and start the new one. The service is configured to **restart on
failure** with a 10 second delay and a 1 day reset period.

## Package behavior

The MSI:

- installs `miru-agent.exe` under 64-bit `Program Files\Miru\Agent`;
- registers `miru-agent.exe` as the service `miru-agent`, running as the
  virtual account `NT SERVICE\miru-agent` with an unrestricted service SID type
  and only the `SeChangeNotifyPrivilege` privilege, set to automatic start,
  started on install, stopped and deleted on uninstall, and configured to
  restart on failure (10 second delay, 1 day reset period);
- uses the permanent UpgradeCode `B5ED0336-5F14-4308-A667-3CE8CDEF7D48`;
- rejects downgrades and schedules major upgrades transactionally so a failed
  replacement can restore the previously installed package;
- allows same-version upgrades, so a stable release replaces a prerelease of
  the same `MAJOR.MINOR.PATCH` in place (see [Code signing](#code-signing) for
  prerelease versions);
- protects `%ProgramData%\Miru` and its authored `logs`, `auth`, and `tmp`
  children by setting their owner to Local System and applying a non-inherited
  DACL that gives Local System and built-in Administrators inheritable full
  control, and gives the `miru-agent` service SID read, write, and traverse
  access to each directory (but not delete or permission changes) plus full
  control of the files and folders created inside it; and
- leaves populated customer state under `%ProgramData%\Miru` in place during
  maintenance, upgrades, rollback, and ordinary uninstall.

The UpgradeCode is part of the product's permanent identity and must never be
changed after publication. Each package version receives a different ProductCode.

If an uninstall or major upgrade fails and rolls back, the installer reapplies
the restored service's restart-on-failure actions, SID type, and privilege
restriction before it starts the service again.

## Build

Run these commands from the repository root on Windows 10 or 11 x64. The build
requires Git, the Rust MSVC toolchain with the `x86_64-pc-windows-msvc` target,
Visual Studio Build Tools with the C++ workload, NASM on `PATH`, and the .NET 8
SDK or later. Installation and integration testing additionally require an
elevated 64-bit Windows PowerShell 5.1 session.

Restore the pinned WiX SDK, build the real Windows executable, and exercise the
package contract with explicit inputs:

```powershell
Set-Location C:\src\agent
cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked --release
dotnet restore build\windows\miru-agent.wixproj
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\package-tests.ps1 -ProjectPath build\windows\miru-agent.wixproj -BinDir target\x86_64-pc-windows-msvc\release -ArtifactsDirectory build\windows\artifacts\package-tests
```

The produced `miru-agent.exe` statically links the MSVC C runtime (via
`+crt-static` in `.cargo/config.toml`), so the installed agent is self-contained
and needs no Visual C++ Redistributable prerequisite on the target machine for
the service to start.

`Version` is deliberately stricter than general SemVer. It must contain three
numeric fields, with `MAJOR` and `MINOR` from 0 through 255 and `PATCH` from 0
through 65535, plus an optional fourth `BUILD` field from 0 through 65535 that
release builds use for prereleases. Leading `v` and prerelease/build labels are
not accepted at the MSI build boundary. `BinDir` must contain
`miru-agent.exe`. WiX is restored through the pinned `WixToolset.Sdk` 7.0.0
project; package validation is enabled and warnings fail the build.

The project sets `AcceptEula=wix7` for noninteractive builds under the
[WiX maintenance-fee agreement](https://docs.firegiant.com/wix/osmf/).
Any applicable maintenance fee must be covered separately; this setting does
not purchase a subscription.

## Install and provision

Run installation from an elevated 64-bit Windows PowerShell 5.1 session. Stable
releases attach a signed `miru-agent-<version>.msi` to the GitHub release (see
[Code signing](#code-signing)); a WinGet manifest remains follow-up work.
Install a trusted MSI directly with Windows Installer:

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
failure; inspect the verbose log. Check the signature before installing:
`Get-AuthenticodeSignature` on the MSI must report `Valid`.

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

## Service account and folder access

The service runs as the virtual account `NT SERVICE\miru-agent`, the Windows
counterpart of the Linux `miru` user. Windows creates the account from the
service name, so it has no password to manage. It has no administrator rights
and holds only the bypass-traverse-checking privilege (`SeChangeNotifyPrivilege`),
so it cannot impersonate other accounts. The installer grants it access to
`%ProgramData%\Miru` only: full control of the files and folders inside, but it
cannot delete or change the permissions of the installer-created folders. Any
other folder the agent uses must be granted to it explicitly.

Run the grants below from an elevated PowerShell session after the MSI is
installed, because the account name resolves only once the service exists. At
any time, including before installation, you can use the service SID instead:
`sc.exe showsid miru-agent` prints it, and `icacls` accepts it as `*<SID>`.

Config deploy target folders (the Windows counterpart of `/srv/miru` on Linux)
need Modify access, because a deploy creates a temporary subfolder and a
`miru.backup.*` file next to the target. A new folder under `C:\` inherits
Modify access for every signed-in user, so replace the inherited permissions on
the target and its parent. Only Local System and Administrators can then change
them, local users can read them, and the service can modify the target:

```powershell
foreach ($dir in "C:\srv", "C:\srv\miru") {
    New-Item -ItemType Directory -Force $dir | Out-Null
    icacls $dir /inheritance:r /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F" "*S-1-5-32-545:(OI)(CI)RX"
}
icacls "C:\srv\miru" /grant "NT SERVICE\miru-agent:(OI)(CI)M"
```

If either folder already existed, first check with `icacls` that Administrators
or Local System owns it; a folder's owner can always change its permissions.

File-rule source folders need read access, plus delete for retention:

```powershell
icacls "D:\robot\logs" /grant "NT SERVICE\miru-agent:(OI)(CI)(RX,D)"
```

Without a grant, the service has only what the folder's ACL already gives to
groups such as `Users` and `Authenticated Users`. That is often write access
for folders created at the root of `C:\`, and none under `C:\Windows` or
`C:\Program Files`. An explicit grant is the supported setup. A deploy to a
folder the service cannot write fails with an access-denied deployment error
and leaves the target unchanged.

Applications that read deployed configs and do not run as a member of the
local `Users` group need their own read grant on the target folder.

## Validation

Every pull request runs the agent test suite natively on Windows
(`windows-check`). Pull requests that change this directory or the CI/release
workflows additionally run the package and installer lifecycle on a separate
`windows-package` job. That job also runs after pushes to `main` and
`release/*`, and when the release workflow calls CI for a tag. Pull requests
that touch `build/**` or the workflows also run `windows-release-build` (the
`cargo auditable` MSVC release build that uploads `miru-agent.exe` and
`miru_agent.pdb`) and `goreleaser-snapshot` (a GoReleaser dry run proving the
`agent_Windows_x86_64.zip`, its SBOM and the PDB are produced). Superseded
pull-request CI runs are cancelled.

From an elevated 64-bit Windows PowerShell 5.1 session, run the native package
integration matrix only on a disposable test machine. Normal integration removes
allowlisted fixture products, changes `%ProgramData%\Miru`, and creates and
deletes a temporary local user, so the explicit confirmation is required:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ConfirmDisposableTestMachine
```

The matrix covers direct MSI install, maintenance, upgrade, downgrade rejection,
failed-upgrade rollback, uninstall, ACL repair, and state retention. It asserts
the `miru-agent` service is installed (automatic start, `NT SERVICE\miru-agent`,
the installed binary path, the restart failure actions and reset period, an
unrestricted SID type, and only `SeChangeNotifyPrivilege`) after install,
maintenance, upgrade, failed-upgrade rollback, and failed uninstall. It asserts the service runs as its service SID, holds no privilege
but `SeChangeNotifyPrivilege`, and writes its log after install, upgrade, and
both rollbacks, and that it is removed after uninstall.
Maintenance, upgrade, rollback, and ordinary uninstall must retain customer
state, including customer-owned files under `%ProgramData%\Miru` and its
`logs`, `auth`, and `tmp` children. The
root and its `logs`, `auth`, and `tmp` children must be owned by Local System,
with protected DACLs permitting inheritable full control only for Local System
and built-in Administrators, and for the `miru-agent` service SID read, write,
and traverse access to each directory plus full control of its contents,
including when those directories existed with hostile ownership and protected
permissions before installation or maintenance.
Non-administrators must not read sensitive files created in those directories
after installation, create children, or change the directory permissions.

Integration runs write verbose MSI logs directly beneath
`build\windows\artifacts\package-tests\logs\<unique-run-id>`. Each operation
prints its log path before starting Windows Installer. These logs survive
temporary build-output cleanup, including when only cleanup fails.

WinGet publication, full live-backend provisioning, Windows Server
certification, and the Phase 2 `Miru Clients` local group and device-API
discovery-directory permissions remain deferred.

## Code signing

The `windows-sign` job in `.github/workflows/release.yml` signs
`miru-agent.exe` with
[Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/),
builds the MSI around it, signs the MSI, and checks that both signatures are
valid and timestamped. The release publishes only these signed files; if the
job fails, nothing is released.

MSI versions follow the tag. Prereleases add a fourth field (`alpha.N` → `1NN`,
`beta.N` → `2NN`, `rc.N` → `3NN`); other tag shapes fail the release:

| Tag | MSI version | File |
| --- | --- | --- |
| `v0.10.4-beta.1` | `0.10.4.201` | `miru-agent-0.10.4-beta.1.msi` |
| `v0.10.4` | `0.10.4` | `miru-agent-0.10.4.msi` |

Windows Installer ignores the fourth field, so the MSI allows same-version
upgrades: betas and the stable release of `0.10.4` replace each other in place,
and anything below `0.10.4` is rejected as a downgrade. Ordering within one
version is not enforced, so an older beta can be installed over a newer build.

The job logs in with GitHub OIDC, so no Azure secret is stored in GitHub. The
setup is Terraform in the infra repository: `azure/terraform` creates the
managed identity, its federated credential for
`repo:mirurobotics/agent:environment:release`, and its **Artifact Signing
Certificate Profile Signer** role on the certificate profile;
`github/terraform` creates this repository's `release` environment (limited to
`v*` tags) and its `AZURE_*` variables.
