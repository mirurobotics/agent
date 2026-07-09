# Create the device RSA private key at mode 0600 (no post-hoc chmod)

## Scope

| Repo | Path | Access |
|------|------|--------|
| agent | `/home/user/agent` | read-write |

No other repositories are read or modified. All edits live under `agent/src/` and
`agent/tests/` inside the `miru-agent` crate.

## Purpose / Big Picture

The agent generates a 4096-bit RSA key pair that is the device's cryptographic
identity to the Miru backend. Today `crypt::rsa::gen_key_pair` writes the private
key PEM to disk first and only *afterward* runs a separate `chmod 0600`. Between
the write and the chmod the file exists at the process default mode (typically
`0o644` — world-readable). Any local user can read the private key during that
window, and if the process is interrupted (crash, kill, power loss) between the
two syscalls the key is left permanently world-readable. A leaked private key lets
an attacker impersonate the device to the backend.

After this change the private key file is *created* at mode `0o600` and the public
key at `0o640` — the correct permissions exist from the first moment the file is
on disk. There is no widen-then-narrow window and no dependency on a follow-up
chmod succeeding.

Observable outcome: immediately after `gen_key_pair` returns, `stat` on the
private key shows `0600` and the public key shows `0640`, exactly as before — but
now that is the *create* mode, provable by the fact that the code no longer calls
`set_permissions` at all. A reader can verify by running the test suite (the
existing `gen_key_pair::file_permissions` test still passes with the chmod removed)
and by the new `filesys::write_bytes` mode tests.

## Progress

- [ ] Milestone 1 — filesys plumbing: add `WriteOptions.mode`, honor it in `write_bytes`, update all existing construction sites to `mode: None`.
- [ ] Milestone 2 — crypt hardening: create keys with `mode: Some(0o600)` / `Some(0o640)`, remove the two `set_permissions` calls and the now-unused import, update the doc comment.
- [ ] Milestone 3 — tests: add `filesys::write_bytes` mode tests for the atomic and non-atomic branches.
- [ ] Preflight gate: `scripts/preflight.sh` reports `Preflight clean`.

## Surprises & Discoveries

(Add entries as you go.)

- Pre-authoring note: the crypt-level permission assertion this plan needs
  already exists as `gen_key_pair::file_permissions` in `agent/tests/crypt/rsa.rs`
  (asserts private `& 0o777 == 0o600`, public `& 0o777 == 0o640`). We reuse it
  rather than add a duplicate; after Milestone 2 it guards the *create-time* mode.

## Decision Log

(Add entries as decisions are made.)

- **Decision (made):** Thread an optional Unix file mode through the shared
  `filesys` write path (`WriteOptions.mode: Option<u32>`) rather than fixing only
  the one call site in `crypt::rsa`. Rationale: secure-by-default belongs in the
  filesystem layer so every caller that needs a restrictive create mode gets it
  atomically, and the fix cannot be undone by a future refactor of `gen_key_pair`.
  `Option<u32>` defaults to `None`, which preserves today's behavior for every
  existing caller. Alternative considered and rejected: keep the write-then-chmod
  shape but shrink the window — rejected because it cannot close the crash window
  and leaves the anti-pattern in place.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

This section assumes no prior knowledge of the repo.

**The repo.** `/home/user/agent` is the Rust `agent` binary that runs on customer
devices. The crate is `miru-agent`. Tests run with a `test` cargo feature that
enables test-only helpers (`#[cfg(feature = "test")]`).

**Key files (full paths):**

- `/home/user/agent/agent/src/filesys/mod.rs` — defines the write-option types.
  `WriteOptions` (lines ~41-66) currently has two fields, `overwrite: Overwrite`
  and `atomic: Atomic`, derives `Clone, Copy, Debug, Default`, and exposes three
  `const` presets: `OVERWRITE_ATOMIC`, `OVERWRITE_NONATOMIC`, `ATOMIC`. `Overwrite`
  (`Deny`/`Allow`) and `Atomic` (`No`/`Yes`) are small enums defined in the same
  file.
- `/home/user/agent/agent/src/filesys/files.rs` — the shared file I/O functions.
  `write_bytes` (lines ~196-250) is the single choke point all writes flow through
  (`write_string`, `write_json`, and the test-only `seed` all call it). It has two
  branches:
  - **Atomic branch** (`opts.atomic == Atomic::Yes`): uses the `atomicwrites`
    crate. It builds an `AtomicFile` and calls `af.write(|f| f.write_all(buf))`.
    `atomicwrites` writes to a temp file inside a `.atomicwrite` subdirectory with
    default `OpenOptions` (`write+create+truncate`, mode `0o666 & !umask` ⇒ usually
    `0o644`) then renames it into place; the rename preserves that permissive mode.
    This is the root cause of the exposure window.
  - **Non-atomic branch** (`Atomic::No`): for `Overwrite::Deny` it uses
    `tokio::fs::OpenOptions::new().write(true).create_new(true)`; for
    `Overwrite::Allow` it uses `TokioFile::create` (which is
    `write+create+truncate`). Neither sets a mode.
- `/home/user/agent/agent/src/crypt/rsa.rs` — `gen_key_pair` (lines ~40-91). Writes
  the private key via `write_bytes(..., WriteOptions { overwrite, atomic: Atomic::Yes })`
  then `set_permissions(0o600)`; same for the public key with `0o640`. Imports
  `use std::os::unix::fs::PermissionsExt;` (line 2) solely for
  `Permissions::from_mode`.
- `/home/user/agent/agent/src/disk/setup.rs` — `bootstrap` (lines ~51-65) moves the
  generated keys into the `auth/` directory with `files::move_to`, which is
  `tokio::fs::rename` (a `rename(2)`). **rename preserves the inode's mode**, so the
  `0o600`/`0o640` created at generation time survive the move — the end-state on
  disk under `auth/` is correct. No change needed here; called out so a reviewer
  knows the on-disk result is not re-widened later.

**Terms of art:**

- *Atomic write*: write to a temp file, then `rename` it over the target so readers
  never see a half-written file. Provided here by the `atomicwrites` crate
  (version `0.4.4`, pinned in `/home/user/agent/Cargo.lock`).
- *umask*: a per-process mask that clears permission bits at file-creation time; the
  effective create mode is `requested_mode & !umask`. The agent sets no umask, so it
  inherits its launcher's (systemd default is commonly `0o022`).
- *covgate*: a per-directory minimum coverage percentage in a `.covgate` file,
  enforced by `scripts/covgate.sh`. Current floors: `agent/src/filesys/.covgate` =
  `81.69`, `agent/src/crypt/.covgate` = `95.16`.

**Why `mode` at create time is correct and safe.** Create mode is `requested & !umask`.
`0o600 & !0o022 == 0o600` and `0o640 & !0o022 == 0o640` — a typical umask strips
nothing, so the resulting mode equals the request. A *more* aggressive umask (e.g.
`0o077`) would only make the mode *more* restrictive (`0o640 & !0o077 == 0o600`),
which is still safe. The tests assert `mode & 0o777 == 0o600` / `== 0o640`, which is
robust because neither `0o600` nor `0o640` carries any bit a normal umask removes.

**`atomicwrites` 0.4.4 API (verified against the crate source).**

    pub fn write<T, E, F>(&self, f: F) -> Result<T, Error<E>>
    pub fn write_with_options<T, E, F>(&self, f: F, options: std::fs::OpenOptions) -> Result<T, Error<E>>

`write` internally builds `OpenOptions::new().write(true).create(true).truncate(true)`
and delegates to `write_with_options`. So to control the temp file's mode we call
`write_with_options` ourselves with a `std::fs::OpenOptions` that additionally sets
`.mode(m)`. Both methods take `&self` and return the same `Result` type, so the two
call paths unify in a `match`.

**tokio `OpenOptions` mode.** `tokio::fs::OpenOptions` exposes an inherent unix
`pub fn mode(&mut self, mode: u32) -> &mut OpenOptions` (verified in tokio 1.52.3),
so the non-atomic branch needs no extra import for `.mode()`. The atomic branch uses
`std::fs::OpenOptions`, which requires `use std::os::unix::fs::OpenOptionsExt;` in
scope for `.mode()`.

## Plan of Work

Edits are grouped into three milestones; each compiles and keeps the suite green.

**Milestone 1 — filesys plumbing.**

1. `agent/src/filesys/mod.rs`, `struct WriteOptions`: add a third field
   `pub mode: Option<u32>`. Keep the existing derives (`Option<u32>::default()` is
   `None`, so `WriteOptions::default()` is unchanged). Document it: `Some(0o600)`
   means the file is created with those Unix permissions from the start; `None`
   (default) keeps today's behavior. Add `mode: None` to each of the three `const`
   presets (`OVERWRITE_ATOMIC`, `OVERWRITE_NONATOMIC`, `ATOMIC`) — const struct
   literals must list every field.

2. `agent/src/filesys/files.rs`, imports: add
   `use std::os::unix::fs::OpenOptionsExt;` to the `// standard crates` group
   (alphabetically between `std::io::Write` and `std::time::SystemTime`).

3. `agent/src/filesys/files.rs`, `write_bytes`, **atomic branch**: when
   `opts.mode` is `Some(m)`, build a `std::fs::OpenOptions` set to
   `.write(true).create(true).truncate(true).mode(m)` and call
   `af.write_with_options(|f| f.write_all(buf), open_opts)`; when `None`, keep the
   existing `af.write(|f| f.write_all(buf))`. Feed the result into the existing
   `.map_err(|e| e.into())` / error-mapping code unchanged.

4. `agent/src/filesys/files.rs`, `write_bytes`, **non-atomic branch**: build the
   `tokio::fs::OpenOptions` explicitly for both overwrite modes and apply
   `.mode(m)` when `opts.mode` is `Some(m)`. For `Overwrite::Deny` keep
   `.write(true).create_new(true)`; for `Overwrite::Allow` use
   `.write(true).create(true).truncate(true)` (identical to the previous
   `TokioFile::create`). Behavior is unchanged when `mode` is `None`.

5. Update the two other source construction sites of the `WriteOptions { .. }`
   struct literal to add `mode: None`:
   `agent/src/filesys/cached_file.rs` (~line 76) and
   `agent/src/cache/dir.rs` (~line 88). All other callers use the `const` presets
   or `WriteOptions::default()` and need no change.

6. Update the six test construction sites of the literal in
   `agent/tests/filesys/files.rs` (lines ~605, 625, 713, 733, 820, 840 — the
   `write_bytes_atomic` / `write_bytes_non_atomic` helpers inside the `write_bytes`,
   `write_string`, and `write_json` test modules) to add `mode: None`, so the test
   crate still compiles.

**Milestone 2 — crypt hardening.**

7. `agent/src/crypt/rsa.rs`, `gen_key_pair`: change the private-key write to
   `WriteOptions { overwrite, atomic: Atomic::Yes, mode: Some(0o600) }` and delete
   the two lines that build `Permissions::from_mode(0o600)` and call
   `files::set_permissions(private_key_file, ...)`. Do the same for the public key
   with `mode: Some(0o640)`, deleting its `set_permissions(0o640)` block.

8. `agent/src/crypt/rsa.rs`: remove `use std::os::unix::fs::PermissionsExt;`
   (line 2) — it is used only by the two deleted `from_mode` calls, so it becomes
   an unused import that `clippy`/`cargo build` warnings (and `-D warnings` in lint)
   would flag. Confirm no other `PermissionsExt` use remains in the file before
   deleting.

9. `agent/src/crypt/rsa.rs`: update the `gen_key_pair` doc comment so it states the
   private key is created with `0600` and the public key with `0640` from the start
   (no post-hoc chmod). Drop the wording implying a separate permission step.

**Milestone 3 — tests.**

10. `agent/tests/filesys/files.rs`, `pub mod write_bytes`: add a test that writes a
    file with `WriteOptions { overwrite: Overwrite::Allow, atomic: Atomic::Yes,
    mode: Some(0o600) }` and asserts `files::permissions(&file).await.unwrap().mode()
    & 0o777 == 0o600`, then repeats with `atomic: Atomic::No` to cover the
    non-atomic branch. `use std::os::unix::fs::PermissionsExt;` is already imported
    at the top of that test file (line 3), and `dirs::temp(..)` + `dir.file(..)` is
    the established pattern in this file.

11. Leave `agent/tests/crypt/rsa.rs`'s `gen_key_pair::file_permissions` test as is —
    it already asserts `0o600`/`0o640` and now proves the create-time mode is
    correct after the chmod removal.

## Concrete Steps

Work from the repo root unless noted:

    cd /home/user/agent

**Milestone 1.** Apply edits 1-6. The `struct WriteOptions` becomes:

    /// Options for file write operations.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct WriteOptions {
        pub overwrite: Overwrite,
        pub atomic: Atomic,
        /// Unix permission bits to create the file with, e.g. `Some(0o600)`.
        /// `None` (default) uses the process default (mode `0o666 & !umask`,
        /// typically `0o644`). `Some(m)` creates the file at `m & !umask` from
        /// the first byte, so the file is never briefly more permissive than
        /// intended and no follow-up `set_permissions` is required.
        pub mode: Option<u32>,
    }

Each preset gains `mode: None`, e.g.:

    pub const OVERWRITE_ATOMIC: Self = Self {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
        mode: None,
    };

The atomic branch of `write_bytes` becomes:

    let write_res = match opts.mode {
        Some(m) => {
            let mut open_opts = std::fs::OpenOptions::new();
            open_opts.write(true).create(true).truncate(true).mode(m);
            af.write_with_options(|f| f.write_all(buf), open_opts)
        }
        None => af.write(|f| f.write_all(buf)),
    };
    let io_err: Result<(), std::io::Error> = write_res.map_err(|e| e.into());

The non-atomic branch becomes:

    let mut f = match opts.overwrite {
        Overwrite::Deny => {
            let mut open_opts = tokio::fs::OpenOptions::new();
            open_opts.write(true).create_new(true);
            if let Some(m) = opts.mode {
                open_opts.mode(m);
            }
            open_opts.open(file.path()).await
        }
        Overwrite::Allow => {
            let mut open_opts = tokio::fs::OpenOptions::new();
            open_opts.write(true).create(true).truncate(true);
            if let Some(m) = opts.mode {
                open_opts.mode(m);
            }
            open_opts.open(file.path()).await
        }
    }
    .map_err(|e| map_io_err_for_create(e, file, opts.overwrite))?;

Verify the crate and tests compile with behavior unchanged:

    ./scripts/test.sh

Expect: all tests pass (no behavior change yet — every existing site is `mode: None`).
The `gen_key_pair::file_permissions` test still passes (chmod still present).

Commit the milestone (work stays on the current designated branch
`claude/agent-security-hunt-lwnvzy`; do not create a new branch):

    git add agent/src/filesys/mod.rs agent/src/filesys/files.rs \
        agent/src/filesys/cached_file.rs agent/src/cache/dir.rs \
        agent/tests/filesys/files.rs
    git commit -m "feat(filesys): add optional create-time mode to WriteOptions"

**Milestone 2.** Apply edits 7-9 in `agent/src/crypt/rsa.rs`. The private-key write
becomes:

    files::write_bytes(
        private_key_file,
        &private_key_pem,
        WriteOptions {
            overwrite,
            atomic: Atomic::Yes,
            mode: Some(0o600),
        },
    )
    .await?;

and the two lines building `Permissions::from_mode(0o600)` + `set_permissions` are
deleted (public key analogous with `mode: Some(0o640)`). Remove the now-unused
`use std::os::unix::fs::PermissionsExt;`. Then:

    ./scripts/test.sh

Expect: all tests pass. Critically, `gen_key_pair::file_permissions` still passes —
the `0o600`/`0o640` now come from the create mode, not a chmod. Commit:

    git add agent/src/crypt/rsa.rs
    git commit -m "fix(crypt): create device RSA keys at 0600/0640 without chmod"

**Milestone 3.** Add the `filesys::write_bytes` mode tests (edit 10). Sketch:

    #[tokio::test]
    async fn honors_mode_atomic() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("keyfile");
        files::write_bytes(
            &file,
            b"secret",
            WriteOptions { overwrite: Overwrite::Allow, atomic: Atomic::Yes, mode: Some(0o600) },
        )
        .await
        .unwrap();
        let mode = files::permissions(&file).await.unwrap().mode() & 0o777;
        assert_eq!(0o600, mode, "atomic write must create file at 0600");
    }

    #[tokio::test]
    async fn honors_mode_non_atomic() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("keyfile");
        files::write_bytes(
            &file,
            b"secret",
            WriteOptions { overwrite: Overwrite::Allow, atomic: Atomic::No, mode: Some(0o600) },
        )
        .await
        .unwrap();
        let mode = files::permissions(&file).await.unwrap().mode() & 0o777;
        assert_eq!(0o600, mode, "non-atomic write must create file at 0600");
    }

Run and commit:

    ./scripts/test.sh
    git add agent/tests/filesys/files.rs
    git commit -m "test(filesys): assert write_bytes honors create-time mode"

**Preflight gate (before publishing).**

    ./scripts/preflight.sh

Expect the final line `Preflight clean`. This runs lint (`scripts/lint.sh`, clippy
`--all-features -D warnings`, import-order and assert-style checks) and the coverage
gate (`scripts/covgate.sh`) in parallel for both the agent crate and the tools crate.
Do not open a PR until this prints `Preflight clean`.

## Validation and Acceptance

Acceptance is behavioral, verified by the test suite and by the create-mode guard.

1. **Suite green.** From `/home/user/agent`, `./scripts/test.sh` (which runs
   `RUST_LOG=off cargo test --features test`) passes with no failures after each
   milestone.

2. **Create-time mode is honored (fails before, passes after).** The new
   `filesys::write_bytes` tests `honors_mode_atomic` / `honors_mode_non_atomic`
   assert the file is created at `0o600`. These exercise the new `mode` field, which
   does not exist before the change. To confirm they genuinely guard the behavior:
   temporarily make `write_bytes` ignore `opts.mode` (i.e. always take the old
   `af.write(...)` / `TokioFile::create` path); re-run `./scripts/test.sh` and
   observe `honors_mode_atomic` FAIL with the created mode reported as `0o644`
   (`assertion left == right: 0o600 != 0o644`). Restore the wiring and it passes.

3. **Private/public key end-state unchanged, now chmod-free.** The existing
   `agent/tests/crypt/rsa.rs` test `gen_key_pair::file_permissions` calls
   `gen_key_pair` into a temp dir and asserts private `mode & 0o777 == 0o600` and
   public `== 0o640`. It passes after Milestone 2 even though both `set_permissions`
   calls are gone — proving the correct mode now comes from file creation. As a
   sanity check that this test guards the fix: if you delete the `set_permissions`
   calls *without* threading `mode` through (i.e. skip Milestone 1's wiring for the
   rsa call sites), `file_permissions` FAILS with the private key at `0o644`.

4. **On-disk result after bootstrap.** `disk::setup::bootstrap` moves the keys into
   `auth/` via `files::move_to` (`rename(2)`), which preserves the inode mode. No
   test change needed; noted so reviewers know the final `auth/` files remain
   `0o600`/`0o640`.

5. **Preflight clean.** `./scripts/preflight.sh` prints `Preflight clean`. Coverage
   stays above the floors: `agent/src/filesys/.covgate` = `81.69` (the new
   `write_bytes` mode branches are covered by the Milestone 3 tests) and
   `agent/src/crypt/.covgate` = `95.16` (removing the two chmod calls removes covered
   lines; the remaining `gen_key_pair` body stays covered by existing tests). If a
   floor regresses, add coverage rather than lowering the gate.

## Idempotence and Recovery

- All edits are source edits; re-running the build/test/preflight commands is safe
  and repeatable. `./scripts/test.sh`, `./scripts/lint.sh`, and
  `./scripts/preflight.sh` have no side effects beyond build artifacts and temp
  files (tests use `dirs::temp` / `files::temp`, which self-clean on drop).
- Each milestone is a self-contained commit; roll back a milestone with
  `git revert <sha>` or, before committing, `git restore <file>`.
- Lowest-risk-first ordering: Milestone 1 adds a defaulted field and changes no
  behavior (every site is `mode: None`), so it is safe to land alone. Milestone 2 is
  the only behavior change; if anything regresses, reverting just that commit
  restores the write-then-chmod path while keeping the (harmless) `mode` field.
- The single external-API assumption is `atomicwrites 0.4.4`'s
  `write_with_options(&self, f, options: std::fs::OpenOptions)` (verified against the
  crate source). If a future dependency bump changes that signature, only the atomic
  branch of `write_bytes` needs adjustment; the `mode` field and all call sites are
  unaffected.
