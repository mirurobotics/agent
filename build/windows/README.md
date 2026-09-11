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

Build the real Windows executable first, then build the project with explicit
inputs:

```powershell
cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked
dotnet build build\windows\miru-agent.wixproj `
    -p:Version=0.10.3 `
    -p:BinDir=target\x86_64-pc-windows-msvc\debug `
    -p:Configuration=Release
```

`Version` is deliberately stricter than general SemVer. It must contain exactly
three numeric fields, with `MAJOR` and `MINOR` from 0 through 255 and `PATCH`
from 0 through 65535. Leading `v`, prerelease/build labels, and fourth fields are
not accepted at the MSI build boundary. `BinDir` must contain
`miru-agent.exe`. WiX is restored through the pinned `WixToolset.Sdk` 5.0.2
project; package validation is enabled and warnings fail the build.

## PowerShell tools

- `scripts/install/install.ps1` checks elevation and x64 execution before side
  effects, verifies an exact SHA-256 record for downloads, rejects MSI metadata
  outside the package contract before invoking `msiexec`, and reports exit 3010
  distinctly when Windows requires a restart.
- `scripts/install/provision.ps1` accepts its secret only through the process
  environment variable `MIRU_PROVISIONING_TOKEN`, invokes the installed
  executable directly, restores the prior environment exactly, and preserves
  the `provision --check` exit contract.

Published checksums detect accidental corruption but do not authenticate the
publisher. Authenticode signing of the executable and MSI remains deferred,
along with the GoReleaser/PDB release lane and artifact publication.
