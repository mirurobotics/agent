# Windows support (x64)

High-level roadmap for making the agent run natively on Windows 10/11 x64 for customer
deployments. This is a multi-PR umbrella plan; each PR gets its own ExecPlan when work
starts. Cross-repo coordination (backend path validation, frontend provision snippets,
docs, e2e) is tracked in the workbench plan `plans/backlog/20260910-agent-windows-support.md`;
this document covers the agent repo's share.

Phasing:

- **Phase 1 — config sync only.** The console-capable agent provisions, syncs
  deployments to disk, and reports status. The local device API server stays disabled
  (`enable_socket_server: false` already supports this). Windows service lifecycle is
  a later step and must not be inferred from the current executable or MSI.
- **Phase 2 — local device API.** Server exposed over localhost TCP with token auth;
  Python SDK follows (external repo).

## Current Linux-only surface (verified inventory)

Production code with a hard Unix dependency — everything else in the crate is already
portable (tokio, axum, reqwest, rumqttc via native-tls, aws/gcs SDKs, atomicwrites,
glob, sysinfo, tracing-appender):

| Location | Dependency | Windows replacement |
|---|---|---|
| `server/serve.rs` | `tokio::net::UnixListener` at `/run/miru/miru.sock`; systemd socket activation via `LISTEN_FDS`/`from_raw_fd(3)` | TCP `127.0.0.1:<port>` + token (Phase 2); no activation equivalent — persistent only |
| `main.rs` (`await_shutdown_signal`) | `tokio::signal::unix` SIGTERM/SIGINT | `tokio::signal::windows` ctrl handlers + service control (`SERVICE_CONTROL_STOP`) |
| `privilege/` | `nix` geteuid/getegid + passwd lookup of the `miru` user | service-account check (warn-only initially) |
| `filesys/files.rs` | `OpenOptionsExt::mode()` on created files | no-op; NTFS ACLs inherited from installer-created dirs |
| `disk/layout.rs`, `logs/mod.rs`, `server/serve.rs` | defaults `/var/lib/miru`, `/var/log/miru`, `/run/miru/miru.sock` | `C:\ProgramData\Miru\{...,logs}` via platform-paths module |
| `crypt/rsa.rs` | `openssl` (vendored) for RSA keygen/PEM/RS256/RS512/fingerprint | aws-lc-rs (see Decisions) |
| `build/` | goreleaser targets `*-unknown-linux-gnu` only; `.deb` + systemd units | msvc build lane + WiX MSI |

Unix usage in `deploy/filesys.rs` and `data_uploads/retention/deleter.rs` is
`#[cfg(test)]`-only.

## Decisions (settled 2026-09-10; rationale in workbench research doc)

1. **Crypto: migrate `crypt/rsa.rs` from openssl to aws-lc-rs.** aws-lc-rs is already
   in the dependency tree (rustls provider for the S3/GCS clients), so the Windows
   build gains no new crypto stack — and loses OpenSSL entirely (native-tls compiles to
   SChannel on Windows). `openssl` is retained as a **unix-only** target dependency,
   solely backing native-tls on Linux. The pure-Rust `rsa` crate is rejected
   (unfixed RUSTSEC-2023-0071). The existing TLS routing rationale in the workspace
   `Cargo.toml` (rumqttc webpki pin) is unchanged by this plan.
2. **Local API transport: localhost TCP + cookie-file token** (Phase 2). Agent mints a
   256-bit CSPRNG token at startup, writes an atomic discovery file
   `device-api.json` (`{port, token}`), and requires
   `Authorization: Bearer` (constant-time compare) on every route. Authorization =
   NTFS ACL on the discovery file (readable by the `Miru Agent Users` local
   group), set by the installer via inheritable ACEs — no security-descriptor
   FFI in agent code.
   Named pipes rejected: instance-per-connection axum Listener, unsafe SDDL FFI, and a
   custom HTTP transport in every SDK, for marginal gain over an ACL'd token file.
3. **Windows is persistent-only.** Socket activation and the `is_persistent: false`
   idle-exit mode remain Linux-only; runtime mode on Windows forces persistence.
4. **Build lane: msvc on a native Windows runner**, artifacts ingested by GoReleaser
   Pro's `prebuilt` builder (PDBs for customer-facing debugging). zigbuild windows-gnu
   stays viable as a fallback once OpenSSL is out of the Windows build, but is not the
   primary lane.
5. **MQTT: stay on rumqttc + native-tls.** Upstream bump PR (bytebeamio/rumqtt#1037)
   is stalled, but the native-tls routing has no exposure and usage is contained to 4
   files. Re-evaluate on: a CVE in rumqttc core with ~4 weeks upstream silence, a
   blocked tokio/rustls major, or the community fork reaching maintained releases.

## PR roadmap

### Phase 1 — compile, run, package

**PR 1 — dependency hygiene.** Move `nix` to `[target.'cfg(unix)'.dependencies]`;
remove the unused `users` workspace dependency. No behavior change; keeps
`cargo machete` honest about per-target deps.

**PR 2 — crypt migration to aws-lc-rs.** Replace openssl in `crypt/rsa.rs`:

- keygen (`Rsa::generate` → aws-lc-rs RSA generation, still on `spawn_blocking`),
- PEM I/O: write PKCS#8; **read both PKCS#1 (`BEGIN RSA PRIVATE KEY`, what openssl
  wrote on every provisioned device) and PKCS#8** — existing fleet keys must load
  forever,
- `fingerprint`, `sign_rs256`, `sign_rs512`, `verify` (RSASSA-PKCS1-v1_5 —
  deterministic, so byte-exact cross-checkable),
- golden fixtures committed under `testdata/`: openssl-generated keys + signatures
  verified by aws-lc-rs, and vice versa (fixtures pre-generated; openssl not needed at
  test time),
- demote `openssl` to unix-only target dependency in the same PR (the Windows build
  must not require it after this lands).

Validation beyond unit tests: on a staging device, (1) fresh provision, (2) token
refresh using a **pre-migration** on-disk key. This path is device identity — a
regression is a fleet-wide auth outage, so it gets the staging soak before release.

**PR 3 — cfg-gates + Windows compile check in CI (completed by PR #234).** Gate `tokio::signal::unix`,
`privilege` (Windows: warn-only stub), `.mode()` calls, and the unix-socket server path
behind `cfg(unix)`; add minimal Windows counterparts (ctrl handlers; no-op modes). Add
`cargo check --target x86_64-pc-windows-msvc` to CI — runs natively on a
`windows-latest` runner (aws-lc-sys cannot cross-compile from Linux; verified
2026-09-11) and prevents Unix-ism regressions from day one.

**PR 4 — platform paths.** Per-OS defaults: `disk::Layout` root
(`/var/lib/miru` ↔ `C:\ProgramData\Miru`), `logs::Options`
(`/var/log/miru` ↔ `C:\ProgramData\Miru\logs`), resolved via `%ProgramData%` rather
than a hardcoded `C:`. `Layout` stays parameterized by `filesystem_root` for tests.

**PR 5 — Windows MSI foundation (PR #236).** Build and validate a pinned WiX x64
MSI for the current console-capable executable. Install under 64-bit Program Files,
protect retained ProgramData state, and prove direct Windows Installer maintenance,
transactional upgrades, rollback, and uninstall behavior. This package intentionally
creates no Windows service. Customer distribution, Authenticode, release artifact
publication, WinGet, and the GoReleaser/PDB lane remain deferred.

**PR 6 — Windows service lifecycle** (PR #242 — `plans/completed/20260916-windows-service-lifecycle.md`)**.** `windows-service` crate: service entry point,
`SERVICE_CONTROL_STOP`/`SHUTDOWN` wired into the existing shutdown broadcast channel
(same channel SIGTERM feeds today; AppState shutdown ordering untouched). `--console`
mode for interactive debugging. Force persistence on Windows (decision 3).

**PR 7 — test-suite portability + Windows CI job.** Fix Unix assumptions (unix-path
fixtures, `/tmp/miru.sock` `#[serial]` tests, passwd/root lookups in privilege tests,
`std::os::unix::fs::symlink` in retention tests, delete-while-open differences). Add a
windows runner job running `scripts/test.sh` equivalents (build + test; covgates remain
enforced on Linux only).

**PR 8 — msvc release lane.** Windows runner job builds
`x86_64-pc-windows-msvc` (via the same cargo-auditable wrapper), uploads binary + PDB;
`build/.goreleaser.yaml` gains a `prebuilt` build id ingesting it; zip archives for the
windows target. (implemented by this plan's PR; `plans/active/20260916-windows-release-lane.md`.)
The PDB is attached to the GitHub release as a separate asset (`miru_agent.pdb`) rather
than inside the zip.

**PR 9 — service-aware MSI follow-up** (in progress — `plans/completed/20260916-windows-msi-service.md`)**.** After the executable implements Windows
Service Control Manager integration, extend the MSI with service install/start/stop,
account, and recovery behavior. The Phase 2 `Miru Agent Users` group and
discovery directory permissions followed once the local device API existed (PR 13).

**PR 10 — Authenticode signing.** Sign binary + MSI in the release pipeline
(osslsigncode from the Linux pipeline, or signtool on the Windows runner), RFC 3161
timestamped. Cert procurement is tracked in the workbench plan (long lead — started
independently). Publish the signed MSI through GitHub Releases, then submit and
maintain its WinGet manifest.
Signing and MSI publication are implemented with Azure Artifact Signing in
`release.yml`'s `windows-sign` job (see `build/windows/README.md`, "Code
signing"); the WinGet manifest remains.

### Phase 2 — local device API (gated on customer need)

**PR 11 — TCP listener** (done — `server/tcp.rs`)**.** Loopback TCP transport that
binds `127.0.0.1:<port>` when `settings.enable_tcp_server` is set. The port is
`settings.tcp_server.port` (default 6478; `0` = OS-assigned, logged at startup).
`enable_tcp_server` defaults to on for Windows and off elsewhere. Shipped for
all platforms rather than `cfg(windows)` only: on Linux it runs alongside the
Unix socket + `LISTEN_FDS` path, which is unchanged; on Windows it is the only
transport, and startup warns when it is turned off. A failed bind logs an error
and the agent keeps running without the listener. Both transports serve the
same `routes::router()` with the same middleware, and the TCP transport
rejects requests whose Host or Origin isn't its loopback address.

**PR 12 — token auth + discovery file** (done — `server/auth.rs`, `disk/device_api.rs`, `app/run.rs`)**.**
Every TCP request, including `/v0.2/health` and the `/v0.2/events` SSE stream,
must carry `Authorization: Bearer <token>`; otherwise the agent returns 401 with
`WWW-Authenticate: Bearer`. The check runs after the loopback check (403) and
compares in constant time. The agent generates a fresh 32-byte token
(43 base64url characters) at every start and atomically writes
`{port, token}` to `/run/miru/device-api.json` on Unix (mode
0640, beside the socket) and to `device-api/device-api.json` under the data
root on Windows (the file inherits the `device-api` directory ACL). The file
is removed after graceful shutdown and any stale copy is removed at startup. A token or discovery-file failure is logged and the agent runs
without TCP. Authorization headers are marked sensitive before request tracing,
so tokens never reach logs. The Unix socket stays unauthenticated. On Windows
the discovery write and removal retry for up to ~0.5s, because a reader holding
the file open without delete sharing (Python's `open`) blocks both. Python SDK
work happens in `python-device-sdk` (transport + discovery file). The SDK must
close the file right after reading, treat a missing file or a refused
connection as "not serving" and retry by re-reading the file, and re-read the
file on a 401. On Windows, PR 13 lets members of the `Miru Agent Users`
group read the file.

**PR 13 — `Miru Agent Users` group + discovery directory ACL** (PR 9
follow-up; `build/windows/miru-agent.wxs`)**.** The MSI creates the local group
`Miru Agent Users` (WiX Util `util:Group`) and
`ProgramData\Miru\device-api` with the siblings' protected descriptor plus
inheritable read for the group (`util:PermissionEx`), so the discovery file
inherits it. Members get nothing on `ProgramData\Miru` itself; bypass traverse
checking lets them open the file by full path. Administrators add members with
`Add-LocalGroupMember` / `net localgroup`; membership applies at the account's
next logon or service start. `device-api` gets an `installer-sentinel` like
`logs`, `auth`, and `tmp`. The MSI side does not depend on PR 12: the
integration tests use a stand-in file, and the real agent writing the real file
is the end-to-end check. Must land before the first Windows release.

**PR 14 — default config folder** (follow-up to PR 13;
`build/windows/miru-agent.wxs`)**.** The MSI creates `ProgramData\Miru\configs`,
the Windows counterpart of Linux `/srv/miru/configs`, with the data folders'
protected descriptor (the service can create and replace configs inside but
cannot delete or re-permission the folder) plus inheritable read for
`Miru Agent Users`, and an `installer-sentinel`. Config paths on Windows must
be absolute with a drive letter (`C:\ProgramData\Miru\configs\...`); the
backend's path validation and the frontend's suggested paths are tracked in
the workbench plan.

## Decision log

- 2026-09-21: PR 11's TCP transport is cross-platform and opt-in on Linux via
  `enable_tcp_server` (default off except on Windows), not a `cfg(windows)` replacement
  for the Unix socket. Rationale: Linux users get the same loopback HTTP
  transport for tooling that cannot speak Unix sockets, the transport gets
  exercised by the Linux test suite and coverage gate rather than only on the
  Windows runner, and keeping it opt-in means no Linux device starts listening
  on a TCP port through an upgrade. Token auth and the discovery file (PR 12)
  still gate any default-on port.
- 2026-09-30: The TCP bearer token is 32 bytes from `aws_lc_rs::rand::fill`,
  base64url without padding (43 characters), regenerated every start, and held
  only in an `Arc<BearerToken>`, never in `AppOptions` or `server::State`. Rationale:
  the token is header-safe, rotates on restart, and stays out of the structs
  the agent logs with `Debug`.
- 2026-09-30: Tokens compare with `aws_lc_rs::constant_time::verify_slices_are_equal`,
  not `tower_http`'s `ValidateRequestHeaderLayer::bearer`. Rationale: the
  `tower_http` layer is not constant-time; the token length is public (always 43).
- 2026-09-30: Auth covers every TCP route including `/v0.2/health` and
  `/v0.2/events`; the Unix socket stays unauthenticated. Rationale: any local
  process can reach the TCP port, while the socket is already restricted by
  file mode 0660 and group `miru`.
- 2026-09-30: The TCP middleware order is loopback (403), then bearer (401),
  then the router. Rationale: unauthenticated requests never touch the idle
  activity tracker.
- 2026-09-30: The discovery file lives in a dedicated `device-api/` directory
  under the data root. Rationale: the MSI can grant readers that directory
  alone without exposing `settings.json` or `device.json`.
- 2026-09-30: On Unix the discovery file moved to `/run/miru/device-api.json`.
  Rationale: SDK clients are the readers, and `/run/miru` is already their
  contract (the socket). `/var/lib/miru` stays private. A tmpfiles.d entry
  creates `/run/miru` as `miru:miru` mode `0750` at boot (and `postinst` applies
  it on install), so the directory exists before the socket binds, survives
  socket and service restarts, and the `miru` group can read the file without
  a grant on the data root. `RuntimeDirectory=`/`User=`/`Group=` are not used
  on the socket unit: with no `Exec*` commands, systemd never applies them on
  start, and `RuntimeDirectory=` would delete the directory on stop.
  Windows stays at `ProgramData\Miru\device-api\`.
- 2026-09-30: The app layer owns the bearer token and the discovery file;
  `tcp::serve` takes the token and only enforces it. Rationale: startup already
  removes stale discovery files in `app/run.rs`, so writing the file after bind
  and removing it after the TCP server stops (via the shutdown manager) keeps
  the whole lifecycle in one place. The file I/O lives in `disk::device_api`,
  which takes the token as a plain string so `disk` does not depend on
  `server`.
- 2026-09-30: The discovery file mode is `0o640`. Rationale: it matches the
  `miru`-group boundary of the Unix socket and the public-key precedent;
  `0o600` would lock out `miru`-group SDK clients.
- 2026-09-30: The discovery file is removed on graceful shutdown and any stale
  file is removed at startup. Rationale: clients can tell "agent not serving
  TCP" (no file) from "token rotated" (401, then re-read the file).
- 2026-09-30: A `tcp::serve` failure (token generation or discovery write) is
  logged and the agent continues without TCP. Rationale: it matches the
  handling of a TCP bind failure; the TCP listener is optional.
- 2026-09-30: PR 13 adds the `Miru Agent Users` ACE with
  `util:PermissionEx` (by account name) next to the core `PermissionEx` SDDL,
  not in the SDDL string. Rationale: a local group's SID is machine-specific, so
  unlike the service SID it cannot be hardcoded. The core descriptor still makes
  the folder protected and owned by SYSTEM; `ExecSecureObjects` runs after
  `CreateFolders` (and after the group exists), merges the ACE with
  `SetEntriesInAcl`, keeps the DACL protected, and reruns on repair and upgrade.
  `SchedSecureObjectsRollback` snapshots the DACL before the transaction, so a
  failed upgrade restores the ACE that the new package's `CreateFolders`
  dropped. A deferred icacls custom action was rejected as more code for the
  same result.
- 2026-09-30: The group is created by WiX Util `util:Group` inside the
  `device-api` component (WiX 7 supports local group creation). It is created
  if missing and reused if present (`UpdateIfExists="yes"`, which only
  rewrites the comment; without it WiX 7's `CreateGroup` returns the
  "group exists" code and fails every major upgrade), vital, and kept on
  uninstall (`RemoveOnUninstall="no"`). Rationale: `device-api` and its ACE
  survive uninstall (the sentinel keeps the folder), so removing the group
  would orphan the ACE and drop administrator-chosen members on a reinstall;
  WiX removes groups in a commit action, so with removal a major upgrade would
  also delete the group the new package had just reused. Administrators remove
  it by hand after uninstall.
- 2026-09-30: The group gets read only (`Read`, `ReadAttributes`,
  `ReadExtendedAttributes`, `ReadPermission`, `Synchronize`; mask
  `0x120089`), inheritable to files and subfolders, and nothing on
  `ProgramData\Miru`. `Users`, `Authenticated Users`, and `Everyone` are never
  granted: the token would then protect nothing from local users.
- 2026-09-30: `device-api` gets a permanent `installer-sentinel`. Rationale:
  the service has `FRFWFX` on the folder, which is empty whenever the agent is
  stopped, so without a sentinel a compromised service could turn it into a
  mount point that an administrator-run repair would follow (the same
  exposure the `logs`, `auth`, and `tmp` sentinels close).
- 2026-10-01: The group is named `Miru Agent Users`, not `Miru Clients` as
  first drafted in Decision 2. Rationale: it is the Windows counterpart of the
  Linux `miru` group, one group for applications that integrate with the agent:
  it grants the local device API now and is meant to grant reading deployed
  configs too, so it is named for the agent rather than for either use (a
  `Miru Device API Users` name was briefly used and dropped for that reason).
  Windows names such groups `<what it grants> Users` (`Remote Desktop Users`,
  `Performance Monitor Users`), and "clients" could be read as Miru customers
  or cloud clients. Renaming after release would be costly (the group survives
  uninstall and customers script its name), so it was settled before the
  first MSI shipped.
- 2026-10-01: PR 14 puts the default config folder at
  `ProgramData\Miru\configs`, not `C:\srv\miru`. Rationale: machine-wide app
  data belongs under `ProgramData\<vendor>`; a folder at the root of `C:\`
  inherits Modify for every signed-in user and would need its parent locked
  down too; and Linux paths do not carry over anyway, because the agent
  rejects `/srv/miru/...` on Windows as not absolute. Non-admins cannot list
  `ProgramData\Miru`, but members open configs by full path.
- 2026-10-01: Deployed configs are readable by `Miru Agent Users` only, not by
  `Users` as Linux's world-readable `/srv/miru` (0755) would suggest.
  Rationale: configs are customer-defined and may hold secrets, and Windows
  robots tend to have more local accounts (operators, remote support, third-
  party services). Applications that read configs usually also use the device
  API, so one group means one setup step. The folder gets an
  `installer-sentinel` because the service can write it and it can be empty.

## Risks

- **Crypt migration blast radius** (PR 2): mitigated by golden fixtures, dual PEM-format
  reads, and staging soak with pre-migration keys before any release.
- **MSI upgrade semantics**: PR #236 exercises install, maintenance, transactional
  upgrade, downgrade rejection, rollback, and uninstall on Windows. Service
  stop/start ordering remains part of the later service-aware MSI follow-up.
- **Locked-down customer environments**: WDAC/AppLocker may require publisher
  whitelisting beyond a valid Authenticode signature; enterprise TLS-intercepting
  proxies are handled by SChannel's OS trust store, but MQTT egress on 8883 may be
  blocked outright (rumqttc's websocket transport is the fallback — out of scope until
  a customer needs it).
- **rumqttc upstream stall**: no current exposure (native-tls), monitored; triggers in
  Decisions.
- **Local API port squatting after a crash**: a crash leaves a stale discovery
  file until the next start. In that gap another local process can bind the
  port; a client using the stale file sends an already-rotated token (useless)
  but trusts the squatter's responses. Accepted for now, since squatting needs
  local code execution. Stronger options if needed: clients reject a file
  older than the agent's start, or the file carries a second secret the server
  proves it knows.
- **Linux configs are world-readable**: `postinst` makes `/srv/miru` mode 0755,
  so any local account can read deployed configs, unlike Windows (PR 14).
  Tightening it to `0750 miru:miru` would match, but applications that read
  configs without being in the `miru` group would break, so it needs a
  migration note. Not yet scheduled.
- **Windows port hijacking while the agent holds the port**: Windows blocks
  cross-account `SO_REUSEADDR` binds by default, but the agent does not set
  `SO_EXCLUSIVEADDRUSE`. Confirm the default protection (or set the option)
  before the first Windows release.

## Non-goals

- Windows ARM64, Windows Server certification (until a customer requires them)
- Named-pipe transport, socket activation, or idle-exit mode on Windows
- Agent self-update (updates ship via MSI upgrades / customer MDM)
- WSL2 productization (viable as a customer pilot; not a supported target)
- macOS (nothing here should preclude it, but it is not in scope)
