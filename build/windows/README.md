# Windows packaging (scaffolding)

Windows installer + provisioning scaffolding for the Miru Agent, part of
Milestone M3 of the Windows support effort
(`plans/active/20260910-windows-support.md`).

**Status: authored but NOT built or validated.** These artifacts describe how the
agent should install and provision on Windows; nothing here is exercised by CI
yet. Do not assume the MSI builds or the ACLs/service config behave as written
until the deferred build lane exists and a real build+install is tested on a
Windows host.

## Contents

- `miru-agent.wxs` — WiX v4/v5 source for the MSI: installs `miru-agent.exe` to
  `Program Files\Miru\Agent`, registers the `MiruAgent` Windows service
  (auto-start, restart-on-failure), and creates `%ProgramData%\Miru` (+ `logs`)
  with SYSTEM/Administrators-only ACLs. Upgrade/uninstall via `MajorUpgrade` +
  `ServiceControl`.
- `../../scripts/install/install.ps1` — download + checksum-verify + `msiexec`
  install (parity with `install.sh`).
- `../../scripts/install/provision.ps1` — run `miru-agent.exe provision` with
  `MIRU_PROVISIONING_TOKEN` (parity with the modern Linux provisioning flow);
  `-Check` mirrors `provision --check`'s exit-code contract.

## Parity mapping (Linux → Windows)

| Debian/systemd | Windows/MSI |
|---|---|
| `miru:miru` user/group, service runs as `miru` | service runs as `LocalSystem` (first cut) |
| `/var/lib/miru`, `/srv/miru` | `%ProgramData%\Miru` (see `agent/src/platform/mod.rs`) |
| `/var/log/miru` | `%ProgramData%\Miru\logs` |
| `postinst`: create dirs, `chown`, enable+start | MSI `CreateFolder` + `PermissionEx` + `ServiceControl Start=install` |
| `miru.service` `Restart=` | `util:ServiceConfig` failure actions (restart, 5s, reset daily) |
| `postrm`: stop/disable/remove | `ServiceControl Stop=both Remove=uninstall` |

## Decisions and open items

- **Service account: `LocalSystem` (first cut).** Simplest working scaffold;
  SYSTEM already has full control of the data dir. Hardening follow-up: switch to
  a least-privilege virtual service account (`NT SERVICE\MiruAgent`) and grant it
  `modify` on `%ProgramData%\Miru` explicitly. Deferred so the first build is
  minimal.
- **`UpgradeCode` GUID is a placeholder.** Generate ONE real GUID, commit it, and
  never change it — it is the upgrade identity. Component GUIDs are `*`.
- **No `Miru Clients` local group.** That group exists only for the Phase 2 local
  Device API (localhost TCP + cookie-file token) and is out of scope here.

## Deferred (gated on the Windows CI lane)

Not done here, intentionally — all blocked on the native-Windows build decision.
A scratch `cargo check --target x86_64-pc-windows-msvc` on Linux fails because
`aws-lc-sys` cannot cross-compile its bundled C there, so the Windows build (and
therefore MSI build + signing) needs a native Windows runner. That runner
question is being settled in the cfg-gate/CI PR (#234).

- Building `miru-agent.exe` for `x86_64-pc-windows-msvc` in CI.
- `wix build` invocation producing the MSI as a release asset (+ checksums).
- Wiring both into `.goreleaser.yaml` (Pro `prebuilt` builder) / the release
  workflow.
- Authenticode signing (binary + MSI), timestamped.

## Building locally (once a Windows binary exists)

```powershell
# Requires the WiX toolset: dotnet tool install --global wix
wix build build\windows\miru-agent.wxs -d Version=0.10.3 -d BinDir=<dir-with-miru-agent.exe> -o miru-agent.msi
```
