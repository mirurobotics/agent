# Windows support (x64)

High-level roadmap for making the agent run natively on Windows 10/11 x64 for customer
deployments. This is a multi-PR umbrella plan; each PR gets its own ExecPlan when work
starts. Cross-repo coordination (backend path validation, frontend provision snippets,
docs, e2e) is tracked in the workbench plan `plans/backlog/20260910-agent-windows-support.md`;
this document covers the agent repo's share.

Phasing:

- **Phase 1 — config sync only.** Agent runs as a Windows service, provisions, syncs
  deployments to disk, reports status. The local device API server stays disabled
  (`enable_socket_server: false` already supports this).
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
   `device-api.json` (`{schema_version, port, token}`), and requires
   `Authorization: Bearer` (constant-time compare) on every route. Authorization =
   NTFS ACL on the discovery file (readable by the `Miru Clients` local group), set by
   the installer via inheritable ACEs — no security-descriptor FFI in agent code.
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

**PR 3 — cfg-gates + Windows compile check in CI.** Gate `tokio::signal::unix`,
`privilege` (Windows: warn-only stub), `.mode()` calls, and the unix-socket server path
behind `cfg(unix)`; add minimal Windows counterparts (ctrl handlers; no-op modes). Add
`cargo check --target x86_64-pc-windows-msvc` to CI — runs natively on a
`windows-latest` runner (aws-lc-sys cannot cross-compile from Linux; verified
2026-09-11) and prevents Unix-ism regressions from day one.

**PR 4 — platform paths.** Per-OS defaults: `disk::Layout` root
(`/var/lib/miru` ↔ `C:\ProgramData\Miru`), `logs::Options`
(`/var/log/miru` ↔ `C:\ProgramData\Miru\logs`), resolved via `%ProgramData%` rather
than a hardcoded `C:`. `Layout` stays parameterized by `filesystem_root` for tests.

**PR 5 — Windows service lifecycle.** `windows-service` crate: service entry point,
`SERVICE_CONTROL_STOP`/`SHUTDOWN` wired into the existing shutdown broadcast channel
(same channel SIGTERM feeds today; AppState shutdown ordering untouched). `--console`
mode for interactive debugging. Force persistence on Windows (decision 3).

**PR 6 — test-suite portability + Windows CI job.** Fix Unix assumptions (unix-path
fixtures, `/tmp/miru.sock` `#[serial]` tests, passwd/root lookups in privilege tests,
`std::os::unix::fs::symlink` in retention tests, delete-while-open differences). Add a
windows runner job running `scripts/test.sh` equivalents (build + test; covgates remain
enforced on Linux only).

**PR 7 — msvc build lane.** Windows runner job builds
`x86_64-pc-windows-msvc` (via the same cargo-auditable wrapper), uploads binary + PDB;
`build/.goreleaser.yaml` gains a `prebuilt` build id ingesting it; zip archives for the
windows target.

**PR 8 — WiX MSI.** Under `build/windows/`: install to Program Files; register the
service (auto-start, restart-on-failure recovery — parity with the systemd unit);
create the `Miru Clients` local group; create `C:\ProgramData\Miru` tree with
inheritable ACLs (SYSTEM + Administrators full; service account modify; `Miru Clients`
read on the auth/discovery dir — parity with `postinst` dir creation + `miru.socket`
group model); upgrade = stop/replace/start; uninstall parity with `postrm`.

**PR 9 — Authenticode signing.** Sign binary + MSI in the release pipeline
(osslsigncode from the Linux pipeline, or signtool on the Windows runner), RFC 3161
timestamped. Cert procurement is tracked in the workbench plan (long lead — started
independently).

**PR 10 — PowerShell install + provision scripts.** `install.ps1` / `provision.ps1`
with parity to `scripts/install/*.sh` (download, verify, install MSI, provision with
`MIRU_PROVISIONING_TOKEN`, `--check` exit-code contract preserved).

### Phase 2 — local device API (gated on customer need)

**PR 11 — TCP listener.** `server/serve.rs` transport split: `cfg(windows)` path binds
`127.0.0.1:<port>` (port in settings; `0` = OS-assigned). Unix socket + `LISTEN_FDS`
path unchanged on Linux.

**PR 12 — token auth + discovery file.** Token generation at startup, atomic
`device-api.json` write into the ACL'd dir, Bearer middleware (constant-time compare)
on all routes, SSE verified over TCP. Python SDK work happens in
`python-device-sdk` (transport + discovery file + re-read-on-401).

## Risks

- **Crypt migration blast radius** (PR 2): mitigated by golden fixtures, dual PEM-format
  reads, and staging soak with pre-migration keys before any release.
- **MSI upgrade semantics**: service stop/start ordering and in-place file replacement
  differ from dpkg; the PR 8 ExecPlan needs an explicit install→upgrade→uninstall→
  reboot test matrix on a clean VM.
- **Locked-down customer environments**: WDAC/AppLocker may require publisher
  whitelisting beyond a valid Authenticode signature; enterprise TLS-intercepting
  proxies are handled by SChannel's OS trust store, but MQTT egress on 8883 may be
  blocked outright (rumqttc's websocket transport is the fallback — out of scope until
  a customer needs it).
- **rumqttc upstream stall**: no current exposure (native-tls), monitored; triggers in
  Decisions.

## Non-goals

- Windows ARM64, Windows Server certification (until a customer requires them)
- Named-pipe transport, socket activation, or idle-exit mode on Windows
- Agent self-update (updates ship via MSI upgrades / customer MDM)
- WSL2 productization (viable as a customer pilot; not a supported target)
- macOS (nothing here should preclude it, but it is not in scope)
