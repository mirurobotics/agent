# Linux permission hardening: private state, `miru-users` group, tighter unit

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent/` (`/home/ben/miru/workbench5/repos/agent`) | read-write | Rust agent code, Debian packaging (`build/debian/`), install-script template, CI workflow, docs. All edits and commits happen here. |
| `docs/` (`/home/ben/miru/workbench5/repos/docs`) | read-only | Customer docs that tell apps to join the `miru` group (`docs/snippets/agent/permissions.mdx`) and describe `/var/lib/miru/auth` ownership (`docs/developers/agent/security.mdx:55`). Updating them is a follow-up PR in that repo, not part of this plan. |
| `ansible-collection-agent/` | read-only | Reads `/var/lib/miru/auth` with `become: true`, so it is unaffected by the tighter modes. |

Branch: `feat/linux-permission-hardening` (already created from `main`). Commands below run from the repo root `/home/ben/miru/workbench5/repos/agent` unless stated otherwise.

## Purpose / Big Picture

Split on 2026-10-01 (see Decision Log): PR #281 delivers only the non-breaking part; the `miru-users` group, `/srv/miru/configs` restriction, and member migration described below ship in a stacked breaking follow-up PR.

On a Linux device today the backend token is world-readable (`/var/lib/miru/auth/token.json` is `644`), every state file and deployed config is readable by any local account, and the `miru` group is both the service's primary group and the group applications join for the device API socket. Windows already has the target model: the service account owns private data, and only the local `Miru Agent Users` group can read the device API discovery folder and the default configs folder.

After this change a Linux device matches that model:

- `/var/lib/miru` and `auth/` are `0700 miru:miru`; every file the agent writes there is `0600` (public key `0640`). `sudo -u nobody cat /var/lib/miru/auth/token.json` fails with `Permission denied`.
- A new system group `miru-users` (the Linux counterpart of `Miru Agent Users`) is the only group that can use `/run/miru/miru.sock`, read `/run/miru/device-api.json`, and read deployed configs in `/srv/miru/configs` (`2750 miru:miru-users`, setgid so new files inherit the group). Apps join `miru-users`, never `miru`.
- `/var/log/miru` is `0750`. The systemd unit gains low-risk hardening directives.
- Upgrades apply all of this to existing devices and copy existing `miru` group members into `miru-users` once.

Breaking for customers: apps that use the socket or read `/srv/miru/configs` must be in `miru-users`. Accounts listed in `/etc/group` as `miru` members (or with `miru` as primary group) are migrated once, automatically, but must re-login or restart their services. Not migrated, and losing access on upgrade: config readers that were never in `miru` (configs were world-readable, so this is most of them) and services that get `miru` only through systemd `Group=`/`SupplementaryGroups=`. Removing world access on upgrade is the requested Windows-parity behavior, so the release note and `build/debian/README.md` must spell out both cases, and the docs-repo update (`docs/snippets/agent/permissions.mdx`, `docs/snippets/agent/filesys/default-perms.mdx`) must ship with the release.

Out of scope: changing the service `UMask` (consumers of configs deployed outside `/srv/miru/configs` rely on `0644`), `ProtectSystem=strict`/`ProtectHome` (configs deploy to, and uploads read, arbitrary customer paths), `SystemCallFilter`, and the docs repo.

## Progress

- [x] M0: `git mv plans/backlog/20261001-linux-permission-hardening.md plans/active/`, commit (`docs(plans): activate linux permission hardening plan`).
- [x] M1: Agent writes private state with explicit modes (code + tests), commit.
- [x] M2: Debian packaging (group, postinst, tmpfiles, socket, service) + container test + CI job, commit.
- [x] M3: Install-script template + regenerated scripts, commit.
- [x] M4: Docs and release note, commit.
- [x] M5: Push, open draft PR, preflight reports `CLEAN`; fill Outcomes, `git mv` the plan to `plans/completed/`, commit, and re-run preflight to `CLEAN` (CI green on `e3f4bea5` in one round, run 36913309334, all 7 jobs including `debian-package` and `windows-package`; PR #281 draft).

## Surprises & Discoveries

- Observation: on bookworm (systemd 252) `systemd-analyze verify` reports an unknown unit key as `Unknown key 'X' in section ...` and still exits 0, so grepping for `Unknown key name` alone misses it.
  Evidence: container run; `postinst-test.sh` greps `Unknown (key|section)|Failed to parse` instead.
- Observation: `systemd-tmpfiles` refuses a symlinked `/srv/miru/configs` ("already exists and is not a directory"), exits 0, and leaves the target alone.
  Evidence: container test step 5.
- Observation: the CI runner's spare supplementary gid is not guaranteed, so the setgid Rust test skips with a message rather than panicking in CI; container step 3 covers setgid inheritance with the real `miru` user.
  Evidence: test plan review; the test ran (not skipped) locally.
- Observation: a non-member owner's `chmod` clears `S_ISGID` on Linux, so the agent must never `chmod` folders under `/srv/miru/configs`; it does not today (`deploy/filesys.rs` only sets permissions in tests).
  Evidence: code review.
- Observation: `cargo clippy` without `--no-deps` fails locally on Rust 1.97 in generated `libs/backend-api` (`clippy::manual_map`), unrelated to this change.
  Evidence: local clippy run.

## Decision Log

- Decision: postinst takes each tree's top folder from `miru` (root-owned, no group or other write) before any chmod and hands it back with `chown -R` last, because GNU chmod follows command-line symlinks and `miru` can swap entries in folders it owns. A symlinked `/srv/miru/configs` is skipped with a warning rather than deleted, so an admin's deliberate symlink is not broken. A reconfigure runs postinst with the service up, and a miru process holding a folder open inside a tree could still swap entries after the handoff, so post_install stops miru.socket and miru.service first (both are restarted at its end).
  Date: 2026-10-01
- Decision: M2's container test and CI job were committed separately (`test(debian): ...`) from the packaging commit, and Rust tests separately from M1 code, following the implement workflow's source-then-tests order.
  Date: 2026-10-01
- Decision: the test plan was extended with preset/constant tests, append-on-existing-file, `create_private_if_absent` error path, tightening tests for each writer, and container checks for stop ordering (systemctl shim log), symlinks inside the trees, and re-migration on fresh install.
  Date: 2026-10-01
- Decision: split the work. PR #281 ships only the non-breaking part: owner-only agent state (Rust writers, `/var/lib/miru` `0700`, `auth/` `0700`, files `0600`, public key `0640`), `/var/log/miru` `0750`, postinst re-applying those on every configure (service stop, symlink-safe takeover), and the unit hardening. The `miru-users` group (socket, `/run/miru`, setgid `/srv/miru/configs`, member migration, install-script group handling) moves to a stacked breaking PR (`feat/linux-miru-users-group`). Rationale: #281 must not break customers, and the compatibility contract is the public docs: socket and device-api access is membership in `miru`, reading `/srv/miru` needs no extra configuration, and `/var/lib/miru` and `/var/log/miru` are for the agent's internal use only. So the steps above for `miru-users`, configs modes, and migration describe the follow-up PR, not #281.
  Date: 2026-10-01

## Outcomes & Retrospective

Delivered in PR #281 (mirurobotics/agent), non-breaking after the split (see Decision Log). The agent writes all state under the data root `0600` (public key `0640`; `auth/` `0700`); the Debian package makes `/var/lib/miru` private and `/var/log/miru` `0750` on every configure (stopping the socket and service first, with symlink-swap protection) and hardens the unit. The socket, `/run/miru`, and the discovery file keep the `miru` group, and configs in `/srv/miru` keep their modes, so they stay world-readable. The `debian-package` CI job runs `shellcheck` and the postinst container test. The `miru-users` group, configs restriction, and member migration are in the stacked breaking follow-up PR. Remaining before release: the manual on-device check in Validation; the docs-repo `miru-users` update ships with the follow-up, not #281.

## Context and Orientation

Terms used below:

- Data root: `/var/lib/miru`, built by `agent/src/disk/layout.rs` (`Layout::root()`); holds `device.json`, `settings.json`, `agent_version`, `system_metadata.json`, `scanner.json`, `upload_queue.json`, `delete_queue.json`, `resources/` (caches, including config instance contents), `events/events.jsonl`, `tmp/`, and `auth/` (`private_key.pem`, `public_key.pem`, `token.json`).
- Mode: Unix permission bits. "Setgid directory" (`chmod g+s`, shown as a leading `2` in `2750`): files and folders created inside get the folder's group instead of the creator's primary group; new subfolders also inherit the setgid bit. This lets the agent (user `miru`, primary group `miru`, not a member of `miru-users`) create files that end up group `miru-users`.
- `StateFile`: `agent/src/filesys/state_file.rs` — `SingleThreadStateFile` / `ConcurrentStateFile`, JSON files persisted with atomic writes. Used for `device.json` (`disk/device.rs`), `auth/token.json` (`authn/token_mngr.rs`), `scanner.json` (`data_uploads/scan/state.rs`), and the upload/delete queues (`data_uploads/queue/mod.rs`).

Write plumbing in `agent/src/filesys/`:

- `mod.rs` defines `WriteOptions { overwrite, atomic, mode: Option<u32> }` with presets `OVERWRITE_ATOMIC`, `OVERWRITE_NONATOMIC`, `ATOMIC` (all `mode: None`, so files get `0666 & ~umask` = `0644` under systemd's default `UMask=0022`; the unit sets none), and `AppendOptions { sync }` with preset `SYNC`.
- `files.rs` honors `mode` on Unix via `mode_open_options` / `apply_mode` and ignores it on Windows (`#[cfg(windows)]` stubs). `write_bytes_atomic` uses the `atomicwrites` crate, which creates a temporary folder `.atomicwriteXXXX` inside the destination's own parent folder, writes `tmpfile.tmp` there with the given open options, then `renameat`s it over the destination. So an atomic write into a setgid folder produces a file with that folder's group, and an atomic write replaces the old file, so the new mode applies even if the old file was `0644`. `append_bytes` opens with `create(true).append(true)` and no mode.
- `dirs.rs`: `create`/`create_if_absent` (`create_dir_all`, default mode), `set_permissions(dir, std::fs::Permissions)`.

Production writes under the data root that currently pass no mode: `disk/setup.rs` (`reset`: `device.json`, `settings.json`, `auth/token.json`; `reset` and `bootstrap` create `auth/` via `dirs::create_if_absent`), `disk/agent_version.rs::write`, `disk/system_metadata.rs::write`, `filesys/state_file.rs` (`create`, `write`), `cache/file.rs` (two `write_json` calls), `cache/dir.rs` (`write_json` with `mode: None` at about line 88), `events/store.rs` (`append_bytes(..., AppendOptions::SYNC)` at about line 50 and the compaction `write_string(..., OVERWRITE_ATOMIC)` at about line 139). Already correct: `crypt/rsa.rs` (private key `0o600`, public key `0o640`) and `disk/device_api.rs` (`/run/miru/device-api.json`, `0o640`, atomic). Must NOT change: `deploy/filesys.rs` writes customer configs to arbitrary paths with `OVERWRITE_ATOMIC`.

Packaging (`build/debian/`, assembled into the `.deb` by nfpm in `build/.goreleaser.yaml`): `postinst` creates group/user `miru` and creates `/var/lib/miru`, `/var/log/miru`, `/srv/miru` (`755`) only when missing, then runs `systemd-tmpfiles --create miru-agent.conf`, `daemon-reload`, and enables/restarts `miru.socket` and `miru.service`. It has no `set -e`. `miru-agent.tmpfiles` (installed as `/usr/lib/tmpfiles.d/miru-agent.conf`) has `d /srv/miru 0755 miru miru -` and `d /run/miru 0750 miru miru -`; a `d` line also adjusts mode/owner of an existing folder at every boot. `miru.socket` has `SocketGroup=miru`, `SocketMode=0660`. `miru.service` runs `User=miru Group=miru` with `StateDirectory=miru` and `LogsDirectory=miru` but no `StateDirectoryMode`/`LogsDirectoryMode` (default `0755`, which systemd may re-apply on every start, undoing a postinst `chmod`). `postrm` purge removes the three folders and nothing else.

Install scripts: `scripts/install/*.sh` (six files) are generated from `scripts/jinja/templates/` by `scripts/jinja/render.sh` (Python + jinja2 venv; writes a fresh `Build Timestamp` line into each). All of them refuse agent v0.9.0+, so they only install older packages; the shared partial `scripts/jinja/templates/partials/utils/activate.sh` runs `sudo chown -R miru:miru /srv/miru`, which would strip `miru-users` from configs on a device that has it. No CI job checks the scripts for drift.

CI (`.github/workflows/ci.yml`): `lint` (`scripts/lint.sh`: import linter, fmt, machete, diet, audit, clippy), `test` (`scripts/covgate.sh`: tests + per-module `.covgate` thresholds, e.g. `agent/src/filesys/.covgate` 81.69, `agent/src/disk/.covgate` 96.79), `windows-check` (`cargo test` on Windows), `tools`, and `windows-package` (runs only when `build/windows/**` or `ci.yml` changes; this plan touches both, so expect that slow job to run). No job tests Debian packaging.

Design choices (record changes to these in the Decision Log): `miru-users` is kept on purge, like the `miru` user and group, and like Windows' `Miru Agent Users`, so memberships survive a reinstall. Members of `miru` are copied only when `miru-users` is created, so an administrator's later removal sticks. `ProtectHostname` is not used: its private UTS namespace would freeze the hostname that `sync/system_metadata.rs` reports. Only `/srv/miru/configs` gets `miru-users`; `/srv/miru` stays `0755 miru:miru` so members can traverse it, and other folders under `/srv/miru` keep today's access. The `2750` folder, not per-file modes, is the boundary: new deploys keep umask-derived `0644` files. A Docker-based postinst test plus a CI job is added because CI has no Linux packaging test. The install scripts only install pre-0.9 agents (a downgrade on a device with this package); the owner-only `chown` keeps migrated apps' access to configs in that case.

Test conventions (`AGENTS.md`): integration tests in `agent/tests/` mirror `agent/src/` and run as the `mod` test target; use `miru_agent::filesys` helpers rather than `std::fs`; gate tests with `#[cfg(unix)]` only when asserting Unix semantics (mode bits); 4+ `assert_eq!` on fields of one variable trips the field-by-field lint. Shared fixtures live in `agent/tests/test_utils/` and must name the library `miru_agent::`.

## Plan of Work

### M1 — agent code

`agent/src/filesys/mod.rs`: add `pub const PRIVATE_FILE_MODE: u32 = 0o600;` and `pub const PRIVATE_DIR_MODE: u32 = 0o700;` with one-line docs ("agent-private files/folders under the data root; ignored on Windows"). Add preset `WriteOptions::OVERWRITE_ATOMIC_PRIVATE` (`Overwrite::Allow`, `Atomic::Yes`, `mode: Some(PRIVATE_FILE_MODE)`). Add `pub mode: Option<u32>` to `AppendOptions` (doc: creation-time Unix mode, ignored on Windows); `SYNC` gets `mode: None`; add `SYNC_PRIVATE` (`Sync::Yes`, `Some(PRIVATE_FILE_MODE)`). `Default` stays derived.

`agent/src/filesys/files.rs::append_bytes`: build the `tokio::fs::OpenOptions` in a local, call the existing `apply_mode(&mut open_opts, opts.mode)`, then open.

`agent/src/filesys/dirs.rs`: add a `// standard crates` group above `// internal crates` containing `#[cfg(unix)] use std::os::unix::fs::PermissionsExt;`, and

    /// Create `dir` if absent and, on Unix, restrict it to its owner
    /// (`PRIVATE_DIR_MODE`), tightening an existing folder too. Parents keep
    /// default modes. Windows: same as `create_if_absent` (installer ACLs apply).
    pub async fn create_private_if_absent(dir: &Dir) -> Result<(), FileSysErr> {
        create(dir).await?;
        // full path: a `use` of the const would be unused on Windows
        #[cfg(unix)]
        set_permissions(dir, std::fs::Permissions::from_mode(crate::filesys::PRIVATE_DIR_MODE)).await?;
        Ok(())
    }

Callers switch to the private variants:

- `filesys/state_file.rs` (add `PRIVATE_FILE_MODE` to the `crate::filesys::{...}` import): `SingleThreadStateFile::create` uses `mode: Some(PRIVATE_FILE_MODE)`; `write` uses `WriteOptions::OVERWRITE_ATOMIC_PRIVATE`. Add to the struct doc: "State files are agent-private: every write is `0600` on Unix."
- `disk/setup.rs`: the three `write_json` calls use `OVERWRITE_ATOMIC_PRIVATE`; both `dirs::create_if_absent(&auth_dir.root)` calls become `dirs::create_private_if_absent`.
- `disk/agent_version.rs::write`, `disk/system_metadata.rs::write`, `cache/file.rs` (both writes), `events/store.rs` compaction: `OVERWRITE_ATOMIC_PRIVATE`. `cache/dir.rs`: add `PRIVATE_FILE_MODE` to its `crate::filesys::{...}` import and use `mode: Some(PRIVATE_FILE_MODE)`. `events/store.rs` append: `AppendOptions::SYNC_PRIVATE`.
- `crypt/rsa.rs`: replace the private key's literal `0o600` with `filesys::PRIVATE_FILE_MODE` (public key stays `0o640`).
- `deploy/filesys.rs`: no behavior change; add one comment above the `write_string` at about line 186: configs keep umask-derived modes because consumers at custom paths rely on them; `/srv/miru/configs` access is governed by its setgid `miru-users` folder.

Tests (all mode assertions `#[cfg(unix)]`). Add two `#[cfg(unix)]` helpers to `agent/tests/test_utils/filesys/mod.rs`, `pub async fn assert_file_mode(file: &File, expected: u32)` and `pub async fn assert_dir_mode(dir: &Dir, expected: u32)`, which compare `files::permissions` / `dirs::permissions` `.mode() & 0o7777` with `expected` (keep them `pub`, import the library as `miru_agent::` per `agent/tests/test_utils/unit.rs`, and `#[cfg(unix)]`-gate their imports so `windows-check` sees no unused imports):

- `agent/tests/filesys/files.rs`, module `write_bytes`: `overwrite_atomic_private_is_0600`; `atomic_private_write_tightens_existing_0644_file` (seed a file, `files::set_permissions` to `0o644`, write with `OVERWRITE_ATOMIC_PRIVATE`, expect `0o600`); `#[cfg(target_os = "linux")] atomic_write_inherits_setgid_dir_group`: pick a gid from `nix::unistd::getgroups()` that differs from `nix::unistd::getegid()`, `std::os::unix::fs::chown(dir, None, Some(gid))`, then `dirs::set_permissions(dir, Permissions::from_mode(0o2770))`, write once with `OVERWRITE_ATOMIC` and once with `mode: Some(0o640)`, assert both files' `MetadataExt::gid()` equal `gid` and `dirs::subdirs(dir)` is empty (no leftover `.atomicwrite*` folder). If no spare gid exists, `panic!` when the `CI` env var is set, else return early with an `eprintln!`. Module `append_bytes`: `honors_mode_on_create` (`AppendOptions::SYNC_PRIVATE` → `0o600`).
- `agent/tests/filesys/dirs.rs`: new `mod create_private_if_absent` with `creates_missing_dir` (portable, asserts exists), `#[cfg(unix)] creates_with_0700`, `#[cfg(unix)] tightens_existing_0755`.
- `agent/tests/filesys/state_file.rs`: in `create`, `#[cfg(unix)] writes_0600`; in `write`, `#[cfg(unix)] tightens_existing_file_to_0600` (create, chmod `0o644`, `write`, expect `0o600`).
- `agent/tests/disk/setup.rs`: in `bootstrap`, `#[cfg(unix)] clean_install_sets_private_modes` (`auth/` `0o700`; `token.json`, `device.json`, `settings.json`, `agent_version` `0o600`); in `reset`, `#[cfg(unix)] tightens_existing_auth_dir` (pre-create `auth/` as `0o755`, run `reset`, expect `0o700`).
- `agent/tests/disk/system_metadata.rs` (`write`), `agent/tests/cache/file.rs` and `agent/tests/cache/dir.rs` (one test each in `single_thread`), `agent/tests/events/store.rs` (`append` and `compaction`): assert the written file is `0o600`.

### M2 — Debian packaging

`build/debian/postinst` (stays POSIX `sh`, no `set -e`, every step idempotent). Keep `create_miru_group` and `create_miru_user`. Replace `create_miru_directories` and add:

    create_miru_users_group() {
      if getent group miru-users > /dev/null 2>&1; then return 0; fi
      printf "\033[32m Creating the 'miru-users' group\033[0m\n"
      groupadd -r miru-users
      migrate_miru_members
    }

    # One-time copy of accounts that used the miru group for socket access:
    # supplementary members plus accounts whose primary group is miru.
    migrate_miru_members() {
      miru_gid=$(getent group miru | cut -d: -f3)
      members=$(getent group miru | cut -d: -f4 | tr ',' ' ')
      primaries=$(getent passwd | awk -F: -v gid="$miru_gid" '$4 == gid { print $1 }')
      for member in $members $primaries; do
        [ "$member" = "miru" ] && continue
        printf "\033[32m Adding '%s' to 'miru-users' (re-login or restart its services to apply)\033[0m\n" "$member"
        usermod -a -G miru-users "$member"
      done
    }

    # Runs as root on every configure, after post_install stops the service. miru
    # owns these trees and could swap an entry for a symlink, which chmod follows,
    # so each tree's top folder is taken from miru before any chmod and handed
    # back last. That only blocks new path lookups; the stop also ends any miru
    # process holding a folder open inside. chown -R never follows symlinks.
    apply_permissions() {
      mkdir -p /var/lib/miru /var/log/miru /srv/miru
      chown root:root /var/lib/miru /var/log/miru /srv/miru
      chmod 0700 /var/lib/miru /var/log/miru
      chmod 0755 /srv/miru

      chmod -R u=rwX,go= /var/lib/miru
      pub=/var/lib/miru/auth/public_key.pem
      if [ ! -L /var/lib/miru/auth ] && [ -f "$pub" ] && [ ! -L "$pub" ]; then
        chmod 0640 "$pub"
      fi
      chown -R miru:miru /var/lib/miru

      chmod -R o-rwx /var/log/miru
      chmod 0750 /var/log/miru
      chown -R miru:miru /var/log/miru

      if [ -L /srv/miru/configs ]; then
        printf "\033[33m /srv/miru/configs is a symlink; leaving its permissions unchanged\033[0m\n"
      else
        mkdir -p /srv/miru/configs
        chown root:root /srv/miru/configs
        chmod 0700 /srv/miru/configs
        chmod -R g+rX,g-w,o-rwx /srv/miru/configs
        find /srv/miru/configs -type d -exec chmod g+s {} +
        chown -R miru:miru-users /srv/miru/configs
      fi
      chown miru:miru /srv/miru
    }

`post_install` order: `create_miru_group`, `create_miru_user`, `create_miru_users_group`, `systemctl stop "${socket_name}" "${service_name}" 2>/dev/null || true`, `apply_permissions`, then the existing tmpfiles/daemon-reload/socket/service steps. Quote `"${socket_name}"` at the two call sites.

`build/debian/miru-agent.tmpfiles`:

    d /srv/miru 0755 miru miru -
    d /srv/miru/configs 2750 miru miru-users -
    d /run/miru 2750 miru miru-users -

`build/debian/miru.socket`: `SocketGroup=miru-users`; update the comment to "`/run/miru` comes from tmpfiles.d (`miru:miru-users 2750`)".

`build/debian/miru.service`: add `StateDirectoryMode=0700` after `StateDirectory=miru` and `LogsDirectoryMode=0750` after `LogsDirectory=miru`. In the security block add `CapabilityBoundingSet=`, `AmbientCapabilities=`, `LockPersonality=true`, `RestrictNamespaces=true`, `SystemCallArchitectures=native`, `ProtectClock=true`, `ProtectKernelLogs=true`, each with a one-line comment in the file's existing style, plus one comment listing what is deliberately not enabled (`ProtectSystem=strict`/`ProtectHome`: arbitrary deploy and upload paths; `ProtectHostname`: stale reported hostname; `UMask` change: config consumers rely on `0644`). The agent spawns no subprocesses and uses none of the restricted syscalls (checked: no `Command::new`, `unshare`, `setns`, `personality`, `clock_settime`, `sethostname` in `agent/src` or `libs/`). Older systemd ignores unknown keys with a warning.

`build/debian/postrm`: in `purge`, add a comment that the `miru-users` group (like the `miru` user and group) is intentionally kept; remove with `groupdel miru-users`.

New `build/debian/tests/postinst-test.sh` (bash, `set -euo pipefail`, `usage()`, tab indentation). Without arguments it runs itself in `docker run --rm -v "$repo_root:/src:ro" debian:bookworm-slim /src/build/debian/tests/postinst-test.sh --in-container`. In the container it:

1. `apt-get update && apt-get install -y --no-install-recommends systemd` (for `systemd-tmpfiles`, `systemd-analyze`); writes a no-op `systemctl` shim to `/usr/local/bin`; copies the tmpfiles file to `/usr/lib/tmpfiles.d/miru-agent.conf` and the units to `/lib/systemd/system/`; creates an executable stub `/usr/sbin/miru-agent`.
2. Upgrade scenario: recreates the verified legacy device state (group/user `miru`; user `app1` with supplementary group `miru`; user `app2` with primary group `miru`; `/var/lib/miru` `755`, `auth/` `775`, `token.json`/`device.json` `644`, `private_key.pem` `600`, `public_key.pem` `640`; `/var/log/miru` `755`; `/srv/miru/configs/v1/motion.json` `644`, folders `755`; all `miru:miru`), runs `sh /src/build/debian/postinst configure 0.10.3`, and asserts `stat -c '%a %U %G'` for every row of the table in Validation, plus `id -nG app1` and `id -nG app2` contain `miru-users`.
3. Setgid check as the real service user (not a member): `runuser -u miru -- sh -c 'umask 022; mkdir -p /srv/miru/configs/new && echo x > /srv/miru/configs/new/f'`, expect `/srv/miru/configs/new` `2755 miru miru-users` and `f` `644 miru miru-users`.
4. Idempotence: `gpasswd -d app1 miru-users`, run postinst again, expect the same table and `app1` NOT back in `miru-users`.
5. Symlink safety: `mkdir /victim && chmod 0755 /victim`, replace `/srv/miru/configs` with `ln -s /victim /srv/miru/configs` (owned by miru), run postinst, and expect `/victim` still `755 root root`, postinst output containing `is a symlink`, and `/srv/miru` `755 miru miru`. Then restore the folder.
6. Fresh install: `groupdel miru-users`, remove the folders, run `sh /src/build/debian/postinst configure` (empty `$2`), expect the table's rows for `/var/lib/miru`, `/var/log/miru`, `/srv/miru`, `/srv/miru/configs`, and `/run/miru`.
7. `systemd-analyze verify /lib/systemd/system/miru.socket /lib/systemd/system/miru.service`; fail on a non-zero exit or output containing `Unknown key name` or `Failed to parse`.

Print `PASS <check>` per check, exit non-zero on the first failure, and end with `all postinst checks passed`.

`.github/workflows/ci.yml`: add job `debian-package` (`runs-on: blacksmith-4vcpu-ubuntu-2404`, `timeout-minutes: 10`, checkout with the same pinned `actions/checkout` SHA as other jobs) with steps `shellcheck -s sh build/debian/postinst build/debian/postrm` (if absent, `sudo apt-get update && sudo apt-get install -y shellcheck`), `shellcheck build/debian/tests/postinst-test.sh`, and `build/debian/tests/postinst-test.sh`.

### M3 — install scripts

`scripts/jinja/templates/partials/utils/activate.sh`: replace the comment and command with

    # Reset /srv/miru ownership to the miru user. Groups are left alone so
    # /srv/miru/configs keeps the miru-users group set by the package.
    sudo chown -R miru /srv/miru

Regenerate the six `scripts/install/*.sh` with `render.sh` (see Concrete Steps); the only diff per file must be that line pair and the `Build Timestamp` line.

### M4 — docs and release note

- `ARCHITECTURE.md` line ~46: socket "(file mode 0660, group `miru-users`)"; discovery file "(mode 0640, group `miru-users` via the setgid `/run/miru`, ...)". Storage paragraph (~line 113): add one sentence that on Linux the data root and `auth/` are `0700`, the agent writes state files `0600` (`filesys::PRIVATE_FILE_MODE`), and `/run/miru` and `/srv/miru/configs` are setgid `miru-users`.
- `agent/src/server/mod.rs` doc comment (~line 20): "`miru-users` group". `agent/src/disk/device_api.rs::write` doc and `agent/src/disk/layout.rs::device_api` doc: the file's group is `miru-users`, inherited from the setgid `/run/miru`.
- `build/windows/README.md` (~line 211): "the Windows counterpart of the Linux `miru-users` group".
- `plans/active/20260910-windows-support.md` risk bullet "Linux configs are world-readable" (~line 332): replace with "**Linux configs were world-readable** (resolved by `plans/active/20261001-linux-permission-hardening.md`): `/srv/miru/configs` is now `2750 miru:miru-users`, setgid, and `miru-users` is the Linux counterpart of `Miru Agent Users`; existing `miru` members are migrated on upgrade."
- New `build/debian/README.md` (short, mirrors `build/windows/README.md`): a table of paths/modes/owners (the Validation table plus `/run/miru/miru.sock` `660 root miru-users` and `/run/miru/device-api.json` `640 miru miru-users`); "Access for local applications" (`sudo usermod -a -G miru-users <account>`, re-login or restart the app's service; never add accounts to `miru`); "Upgrading (breaking)" (what changed; the automatic one-time migration of `/etc/group` members; the two unmigrated cases from Purpose and how to add them, including `SupplementaryGroups=miru-users` for systemd services; configs outside `/srv/miru/configs` unchanged); "Uninstall" (`miru-users` kept; `sudo groupdel miru-users`).
- Release note: the M2 commit is `feat(debian)!: ...` with a `BREAKING CHANGE:` footer (goreleaser's changelog groups `feat!`), and the PR body carries the same note, naming both unmigrated cases from Purpose.

## Concrete Steps

M0 (from repo root):

    mkdir -p plans/active && git mv plans/backlog/20261001-linux-permission-hardening.md plans/active/
    git commit -m "docs(plans): activate linux permission hardening plan"

M1 (from repo root):

    cargo test -p miru-agent --test mod -- filesys:: disk:: cache:: events::
    # expect: all pass, including the new tests; then
    cargo test -p miru-agent --lib
    git add -A agent && git commit   # "feat(agent): write agent-private state with owner-only modes"

M2:

    chmod +x build/debian/tests/postinst-test.sh
    shellcheck -s sh build/debian/postinst build/debian/postrm
    shellcheck build/debian/tests/postinst-test.sh
    build/debian/tests/postinst-test.sh
    # expect: a PASS line per check, last line "all postinst checks passed", exit 0
    git add build/debian .github/workflows/ci.yml && git commit
    # "feat(debian)!: add miru-users group and tighten Linux permissions"
    # footer: "BREAKING CHANGE: apps must be in miru-users to use the socket or read /srv/miru/configs. /etc/group members of miru are migrated on upgrade; other config readers and services granted miru via systemd Group=/SupplementaryGroups= must be added to miru-users."

M3 (render from a scratch directory so the venv is not created in the repo):

    edit scripts/jinja/templates/partials/utils/activate.sh
    (cd "$(mktemp -d)" && /home/ben/miru/workbench5/repos/agent/scripts/jinja/render.sh)
    git diff --stat scripts/install     # 6 files, ~3 lines each
    git diff scripts/install | grep '^[-+][^-+]'
    # expect only: Build Timestamp, the comment, and "chown -R miru /srv/miru" lines.
    # If render.sh cannot install jinja2 (no network), apply the same two-line edit
    # by hand to all six files and leave timestamps unchanged; note it in Surprises.
    git add scripts && git commit   # "fix(scripts): keep /srv/miru groups when activating"

M4: make the doc edits, then `git add -A ARCHITECTURE.md agent/src build plans && git commit` ("docs: describe the miru-users group and Linux permission model").

M5: push and open a draft PR (`$pr`), then run `$preflight` until it reports `CLEAN`.

## Validation and Acceptance

Automated, in CI:

- `test` job: the new tests above pass; each fails before M1 (files are `0644`, `auth/` is `0755`) and passes after. Coverage gates hold (`agent/src/filesys/.covgate`, `agent/src/disk/.covgate`).
- `windows-check`: mode tests are `#[cfg(unix)]`/`#[cfg(target_os = "linux")]`; `create_private_if_absent::creates_missing_dir` runs on Windows.
- `debian-package`: after the upgrade scenario `stat -c '%a %U %G'` shows

        /var/lib/miru                       700  miru miru
        /var/lib/miru/auth                  700  miru miru
        /var/lib/miru/auth/token.json       600  miru miru
        /var/lib/miru/auth/private_key.pem  600  miru miru
        /var/lib/miru/auth/public_key.pem   640  miru miru
        /var/lib/miru/device.json           600  miru miru
        /var/log/miru                       750  miru miru
        /srv/miru                           755  miru miru
        /srv/miru/configs                   2750 miru miru-users
        /srv/miru/configs/v1                2750 miru miru-users
        /srv/miru/configs/v1/motion.json    640  miru miru-users
        /run/miru                           2750 miru miru-users

  plus the migration, setgid, idempotence, fresh-install, and `systemd-analyze verify` checks.
- `lint`, `tools`, and (triggered by `build/windows/README.md` and `ci.yml`) `windows-package` stay green.

Release gate (not a merge gate): the docs-repo update for `miru-users` ships with the agent release that contains this change.

Gate: preflight must report `CLEAN` — every CI job green on the pushed branch head — before the PR leaves draft or the task is reported complete. A local pass is not a substitute.

Manual, on a real Debian/Ubuntu device with an older agent installed (recommended before release, not a merge gate): install the new unit, socket, tmpfiles file, and binary over the old ones, run `sudo sh build/debian/postinst configure <old-version>`, and expect: the table above; `/run/miru/miru.sock` `660 root miru-users`; with `enable_tcp_server` on, `/run/miru/device-api.json` `640 miru miru-users`; the service active; deploys to `/srv/miru/configs/x.json` (group `miru-users`) and to a path outside `/srv/miru` both succeed; `sudo -u nobody cat /var/lib/miru/auth/token.json` and `sudo -u nobody ls /srv/miru/configs` fail with `Permission denied`; a re-logged-in `miru-users` member can `curl --unix-socket /run/miru/miru.sock http://localhost/v0.2/health` and read the config.

## Idempotence and Recovery

All code and doc steps are plain edits; re-running tests is safe. `postinst` is designed to be re-run: group creation and migration are skipped when `miru-users` exists, and `apply_permissions` sets absolute owners/modes. The container test runs in a throwaway container. `render.sh` can be re-run; only timestamps change. If a device ends up with an app locked out, `sudo usermod -a -G miru-users <account>` and restarting the app restores access. To roll back, reinstall the previous package (its unit and tmpfiles files restore `SocketGroup=miru` and `/run/miru` `0750 miru:miru`; migrated accounts are still in `miru`), then restore world-readable configs with `sudo chown -R miru:miru /srv/miru/configs && sudo chmod -R g-s,o+rX /srv/miru/configs && sudo chmod 0755 /srv/miru/configs`.
