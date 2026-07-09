# Use `filesys` Module In Tests (Replace Raw `std::fs` / `tokio::fs`)

**Status**: active
**Branch**: refactor/filesys-use-module-in-tests (off latest origin/main)
**Date**: 20260708

## Goal

Replace raw `std::fs` / `tokio::fs` calls with the repo's internal `filesys`
module (`filesys::Dir` / `filesys::File` plus the free functions in
`filesys::files::` and `filesys::dirs::`) at test/setup sites that already
operate on `filesys` values but bypass the abstraction. All target sites are
in test code. No production code changes; no behavior changes.

## Verified module API (read from origin/main `agent/src/filesys/`)

- `dirs::create(dir: &Dir) -> Result<(), FileSysErr>` — async; `create_dir_all` semantics (dir.rs/dirs.rs L106).
- `dirs::set_permissions(dir: &Dir, perms: std::fs::Permissions) -> Result<(), FileSysErr>` — async; REQUIRES a `std::fs::Permissions` (dirs.rs L234).
- `dirs::files(dir: &Dir) -> Result<Vec<File>, FileSysErr>` — async; returns regular files in `dir` (dirs.rs L145).
- `dirs::temp(prefix: &str) -> Result<TempDir, FileSysErr>` — sync, `#[cfg(feature="test")]`. `TempDir` impls `Deref<Target=Dir>` and exposes `.dir() -> &Dir`, `.to_dir() -> Dir`, `.path()` (dirs.rs L57-102).
- `files::write_string(file: &File, s: &str, opts: WriteOptions) -> Result<(), FileSysErr>` — async (files.rs L252).
- `files::delete(file: &File) -> Result<(), FileSysErr>` — async; NotFound is Ok (files.rs L272).
- `Dir::new<T: Into<PathBuf>>(path)`, `dir.subdir(rel) -> Dir`, `dir.file(name) -> File`, `dir.path() -> &PathBuf` (dir.rs; `path()` via `PathExt`).
- `File::new<T: Into<PathBuf>>(path)`, `file.name() -> Result<&str, FileSysErr>`, `file.path() -> &PathBuf` (file.rs).
- `WriteOptions::OVERWRITE_ATOMIC` — `{ overwrite: Allow, atomic: Yes }` (mod.rs L50).

Note: `Dir::new`/`File::new` take `Into<PathBuf>`. `&PathBuf` does NOT impl
`Into<PathBuf>`, so when building a `Dir`/`File` from another handle's
`.path()` (which returns `&PathBuf`), clone it: `.path().clone()`.

## Verified accessor return types (corrections to initial assumptions)

Read from `agent/src/disk/layout.rs`:
- `Layout::agent_version(&self) -> filesys::File` (L41) — **File, by value** (NOT a Dir).
- `Layout::device(&self) -> filesys::File` (L37) — **File, by value** (NOT a Dir).
- `Layout::auth(&self) -> AuthLayout` (L25); `AuthLayout::private_key(&self) -> filesys::File` (L97) — **File, by value**.

Consequence for `tests/app/upgrade.rs`: two of the three sites deliberately
create a *directory at a File path* (to force downstream read/write errors).
Because the accessor returns a `File`, we cannot pass it to `dirs::create`
directly — we must construct a `Dir` from the File's path. See File 3 below.

## Scope (three files, all in the `agent/` crate)

### File 1 — `agent/src/deploy/filesys.rs` (`#[cfg(test)] mod tests`)

Two `#[cfg(unix)] #[tokio::test]` tests, both already async and already
obtaining a temp dir via `filesys::dirs::temp(...)`:
- `rollback_returns_errors_when_restores_fail_synthetic` (L538)
- `remove_backups_continues_when_delete_fails` (L597)

Imports already in scope via `use super::*` (module-level L8:
`use crate::filesys::{self, errors::FileSysErr, files, PathExt, WriteOptions};`)
plus test-mod `use crate::filesys;`, `use std::os::unix::fs::PermissionsExt;`,
`use std::path::PathBuf;`. So `files`, `WriteOptions`, `filesys::{Dir,File}`
are all reachable; no new imports required.

Conversions (operations only — do NOT change any `Permissions::from_mode(...)`
expression; `dirs::set_permissions` still needs a `std::fs::Permissions`):

Build handles from the existing `tmp` guard (`tmp` derefs to `Dir`, so
`tmp.subdir(...)`, `tmp.file(...)` work; keep `filesys::File::new(...)` for the
`Snapshot { dst, backup }` construction to preserve the existing style there).

`rollback_returns_errors_when_restores_fail_synthetic`:

BEFORE (L551-557, L561, L580):
```rust
let existed_parent = tmp.path().join("existed_parent");
let dne_parent = tmp.path().join("dne_parent");
std::fs::create_dir_all(&existed_parent).unwrap();
std::fs::create_dir_all(&dne_parent).unwrap();

std::fs::write(existed_parent.join("backup.json"), "backup content").unwrap();
std::fs::write(dne_parent.join("dst.json"), "leftover").unwrap();

std::fs::set_permissions(&existed_parent, std::fs::Permissions::from_mode(0o555)).unwrap();
...
std::fs::set_permissions(&existed_parent, std::fs::Permissions::from_mode(0o755)).unwrap();
```
AFTER:
```rust
let existed_parent = tmp.subdir("existed_parent");
let dne_parent = tmp.subdir("dne_parent");
dirs::create(&existed_parent).await.unwrap();
dirs::create(&dne_parent).await.unwrap();

files::write_string(&existed_parent.file("backup.json"), "backup content", WriteOptions::OVERWRITE_ATOMIC).await.unwrap();
files::write_string(&dne_parent.file("dst.json"), "leftover", WriteOptions::OVERWRITE_ATOMIC).await.unwrap();

dirs::set_permissions(&existed_parent, std::fs::Permissions::from_mode(0o555)).await.unwrap();
...
dirs::set_permissions(&existed_parent, std::fs::Permissions::from_mode(0o755)).await.unwrap();
```
`existed_parent`/`dne_parent` are now `Dir` values. The `Snapshot` construction
below (L566-574) references paths via `.join(...)`; update those to the `Dir`
API for consistency, keeping `filesys::File::new(...)`:
- `filesys::File::new(dne_parent.join("dst.json"))` → `dne_parent.file("dst.json")` (returns `File`; drop the `filesys::File::new` wrapper).
- `filesys::File::new(existed_parent.join("dst.json"))` → `existed_parent.file("dst.json")`.
- `filesys::File::new(existed_parent.join("backup.json"))` → `existed_parent.file("backup.json")`.

Assertions (L582-593) use `existed_parent.join("backup.json").exists()` etc.
`existed_parent` is now a `Dir`, not a `PathBuf`, so `.join(...)` is gone.
Rewrite each assertion target to `existed_parent.file("backup.json").exists()`
(`File`/`Dir` impl `PathExt`, which provides `.exists()` — `PathExt` is in
scope via `use super::*`). Keep the assertion messages verbatim. Preserve the
load-bearing comments at L542-550, L559-560, L563-565, L578-579 unchanged.

`dirs::set_permissions` `.await` note: the pre-assertion permission restore
(L580) is now `.await`ed; it already runs before the assertions, so ordering is
preserved.

`remove_backups_continues_when_delete_fails` — same mechanical transform
(L602-612, L632): `tmp.subdir("writable")`, `tmp.subdir("locked")`;
`dirs::create(&...).await`; `files::write_string(&dir.file("dst.json"), "content", WriteOptions::OVERWRITE_ATOMIC).await`;
`dirs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).await`;
and rewrite the `Snapshot` `dst`/`backup` and the trailing `.exists()`
assertions (L634-644) to `<dir>.file("miru.backup.dst.json").exists()`.
Preserve comments L601, L607, L614-617, L631, L640.

### File 2 — `agent/tests/deploy/filesys.rs`

Imports at top already include `use miru_agent::filesys::{self, dirs, files, Overwrite, PathExt, WriteOptions};` (L9). No new imports needed. `File::name()` is available; `PathExt` (L9) provides `.path()`.

Whole file is effectively unix-only (`use std::os::unix::fs::PermissionsExt;` at L2, unconditional).

(a) `detect_backup_files` (L124-134) — make async, use `dirs::files`:

BEFORE:
```rust
fn detect_backup_files(dir: &filesys::Dir) -> Vec<filesys::File> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(BACKUP_FILE_PREFIX) {
            out.push(filesys::File::new(entry.path()));
        }
    }
    out
}
```
AFTER:
```rust
async fn detect_backup_files(dir: &filesys::Dir) -> Vec<filesys::File> {
    dirs::files(dir)
        .await
        .unwrap()
        .into_iter()
        .filter(|file| {
            file.name()
                .map(|name| name.starts_with(BACKUP_FILE_PREFIX))
                .unwrap_or(false)
        })
        .collect()
}
```
Signature stays `&filesys::Dir` (callers pass both `f.temp_dir.dir()` which is
`&Dir`, and `&locked_dir` where `locked_dir = f.temp_dir.subdir("locked")` is a
`Dir`). `File::name()` returns `Result<&str, FileSysErr>`; treat an unnamed
entry as "not a backup" (`.unwrap_or(false)`).

(b) All 8 call sites become `.await`. Every enclosing fn is `#[tokio::test]`
(verified). Add `.await` at each. Grep to confirm none are missed after editing:
`grep -n "detect_backup_files" agent/tests/deploy/filesys.rs` must show the
definition (now `async fn`) plus 8 `.await` call sites.

(c) `stale_backup_overwritten` (L330-331) — the one remaining `std::fs::write`:

BEFORE:
```rust
let stale_backup = f.temp_dir.path().join("miru.backup.a.json");
std::fs::write(&stale_backup, "stale_backup").unwrap();
```
AFTER (match the `files::write_string` already used for `a.json` a few lines up at L321-327):
```rust
let stale_backup = f.temp_dir.path().join("miru.backup.a.json");
files::write_string(
    &filesys::File::new(&stale_backup),
    "stale_backup",
    WriteOptions::OVERWRITE_ATOMIC,
)
.await
.unwrap();
```
Keep `assert!(stale_backup.exists())` at L332 (PathBuf method, unchanged).

(d) LEAVE `read_only()` (L114-116) and `writeable()` (L118-120) unchanged —
they return the exact `std::fs::Permissions` that `dirs::set_permissions`
requires.

### File 3 — `agent/tests/app/upgrade.rs`

Imports at top include `use miru_agent::filesys::{dirs, files, Overwrite, PathExt, WriteOptions};` (L12). `filesys::Dir` itself is NOT imported; reference it via the full path `miru_agent::filesys::Dir` (or add `Dir` to the L12 import — prefer the smaller diff / full path). `PathExt` (L12) provides `.path()`. All three enclosing fns are `#[tokio::test]` (verified).

(a) L272-273 `returns_true_when_read_errors` — creating a *directory* at the
`agent_version` marker File path:

BEFORE:
```rust
tokio::fs::create_dir_all(layout.agent_version().path())
    .await
    .unwrap();
```
AFTER:
```rust
dirs::create(&miru_agent::filesys::Dir::new(layout.agent_version().path().clone()))
    .await
    .unwrap();
```
(`agent_version()` returns `File`; `.path()` is `&PathBuf`; `.clone()` gives an
owned `PathBuf` for `Dir::new`.)

(b) L304-306 `returns_authn_err_when_private_key_missing` — deleting a File:

BEFORE:
```rust
tokio::fs::remove_file(layout.auth().private_key().path())
    .await
    .unwrap();
```
AFTER:
```rust
files::delete(&layout.auth().private_key())
    .await
    .unwrap();
```
(`private_key()` returns `File` by value; borrow it.) Note `files::delete`
treats NotFound as Ok; here the key exists (created in `prepare_layout`), so
behavior is unchanged.

(c) L343-345 `returns_storage_err_when_reset_fails` — creating a *directory* at
the `device` File path:

BEFORE:
```rust
tokio::fs::create_dir_all(layout.device().path())
    .await
    .unwrap();
```
AFTER:
```rust
dirs::create(&miru_agent::filesys::Dir::new(layout.device().path().clone()))
    .await
    .unwrap();
```

## Explicitly OUT of scope (do NOT touch)

- Any `std::fs::Permissions::from_mode(0o...)` construction anywhere
  (crypt/rsa.rs, tests/deploy/apply.rs, tests/filesys/{dir,file}.rs, and the
  in-scope files' Permissions expressions).
- `agent/src/filesys/` module internals.
- `tools/lint/` (separate workspace).
- `read_only()` / `writeable()` helpers in tests/deploy/filesys.rs.

## Test steps

All commands run from within the agent repo (`/home/ben/miru/workbench1/repos/agent`).
`--features test` is mandatory (test helpers/mocks are gated). The integration
test binary is `mod` (single entry `agent/tests/mod.rs`); filter by module path.

1. Compile (fast feedback that async fan-out + borrows line up):
   ```bash
   cargo test --package miru-agent --features test --no-run
   ```
2. File 1 — the two `src/deploy/filesys.rs` unit tests (`--lib`), both `#[cfg(unix)]`:
   ```bash
   RUST_LOG=off cargo test --package miru-agent --features test --lib -- deploy::filesys::tests::rollback_returns_errors_when_restores_fail_synthetic deploy::filesys::tests::remove_backups_continues_when_delete_fails
   ```
3. File 2 — the `tests/deploy/filesys.rs` integration tests (all callers of `detect_backup_files` + `stale_backup_overwritten`):
   ```bash
   RUST_LOG=off cargo test --package miru-agent --features test --test mod -- deploy::filesys
   ```
4. File 3 — the `tests/app/upgrade.rs` tests:
   ```bash
   RUST_LOG=off cargo test --package miru-agent --features test --test mod -- app::upgrade
   ```
5. Full affected surface at once (sanity):
   ```bash
   RUST_LOG=off cargo test --package miru-agent --features test -- deploy::filesys app::upgrade
   ```

Environment / CI notes:
- The two File 1 unit tests are `#[cfg(unix)]` and permission-based
  (chmod 0o555 → expect EACCES). They run locally on this Linux host. They are
  skipped on non-unix; permission semantics require NOT running as root (root
  bypasses mode bits), which is the normal local/CI case.
- `tests/deploy/filesys.rs` is unix-only by construction (`PermissionsExt`
  import, no `#[cfg]` guard). The permission-based deploy tests run locally on
  Linux as non-root. These are pure-filesystem tests with NO network/credentials
  — none are credentialed integration tests, so all run locally.
- `tests/app/upgrade.rs` uses a `MockClient` (no real backend/network); runs
  locally. RSA keypair generation in `prepare_layout` is local CPU only.
- Everything in scope runs locally; nothing here depends on CI-only credentials.

## Validation

- Run the full local gate before publishing:
  ```bash
  ./scripts/preflight.sh
  ```
  It fans out lint (`scripts/lint.sh`: import linter + `cargo fmt --check` +
  machete/diet + audit + clippy `-D warnings`), tests-with-coverage-gates
  (`scripts/covgate.sh`), and the `tools/lint` workspace's own lint + tests.
  **Preflight MUST print `Preflight clean` (exit 0) — lint, format, and all
  tests passing — before the changes are pushed/published.** A nonzero exit or
  any "Preflight FAILED" line blocks publishing.
- Because `--features test` is required, never validate with a bare
  `cargo test`; use the scripts or the explicit `--features test` invocations
  above.
- Coverage: these are test-only edits that preserve every assertion, so the
  `agent/src/deploy/filesys.rs` `.covgate` threshold is unaffected; `covgate.sh`
  (invoked by preflight) confirms.

## Git / publishing constraints

- All commits are made from within `/home/ben/miru/workbench1/repos/agent`
  (the agent repo's own git context), never the workbench root.
- Branch `refactor/filesys-use-module-in-tests` is already checked out off
  latest `origin/main`. Commit there; do not commit to `main`.

## Risk / gotchas checklist

- `detect_backup_files` async fan-out: exactly 8 `.await` call sites; verify
  with grep after editing (definition + 8 callers).
- `dirs::files` returns only regular files (skips subdirs); `detect_backup_files`
  previously iterated all `read_dir` entries but filtered by the `miru.backup.`
  filename prefix, and backups are always files — behavior preserved.
- File-vs-Dir: `agent_version()`/`device()` return `File`; must wrap
  `.path().clone()` in `Dir::new` for `dirs::create`. Do NOT pass the `File`.
- Permissions type: `dirs::set_permissions` keeps taking `std::fs::Permissions`;
  the `from_mode(...)` expressions and `read_only()`/`writeable()` helpers are
  untouched.
- `.exists()` in File 1 assertions comes from `PathExt` (in scope via
  `use super::*`), replacing the former `PathBuf::exists`.
