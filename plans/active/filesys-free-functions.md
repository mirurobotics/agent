# ExecPlan: Refactor `filesys` to free-function seam (mirror core)

## Goal

Refactor `agent/src/filesys/` so that the **File/Dir seam** matches the core repo's convention: a *method* on `File`/`Dir` never touches disk. Every async / fallible-on-I/O operation becomes a **free function** in a sibling module (`files.rs` / `dirs.rs`) taking `&File` / `&Dir`. This is a **pure mechanical move** — no behavior, error variants, `trace!()` sites, TOCTOU comments, atomic-write, or move-rollback logic may change.

Reference implementation: `/home/ben/miru/workbench1/repos/core/pkg/filesys/{dir.go,file.go}` (pure methods), `/home/ben/miru/workbench1/repos/core/pkg/files/os.go` (`files.ReadJson`, `files.WriteBytes`, …), `/home/ben/miru/workbench1/repos/core/pkg/dirs/os.go` (`dirs.Create`, `dirs.Home`, `dirs.CreateTemp`, …).

**Note on paths:** the module physically lives at `agent/agent/src/filesys/` (nested crate dir). All paths below are under `/home/ben/miru/workbench1/repos/agent/agent/`.

## Context / current state (verified)

- Files: `agent/src/filesys/{mod.rs,file.rs,dir.rs,cached_file.rs,path.rs,errors.rs}` plus one `.covgate`.
- `File` and `Dir` each wrap a single `PathBuf`; both implement `PathExt` (path.rs) which provides `path`, `abs_path`, `exists`, `assert_exists`, `assert_doesnt_exist`. **These PathExt methods are out of scope and stay** (trait methods; `exists`/`assert_*` use sync `Path::exists`, not in the move list).
- `errors.rs` already defines every error struct used. **errors.rs needs no changes.**
- Call-site blast radius (measured):
  - Distinctive File I/O methods in `agent/src` outside filesys: ~20 sites across ~9 files.
  - Ambiguous `.delete()` / `.create()` / `.create_if_absent()` / `.move_to()` — receiver may be `File` **or** `Dir`; disambiguate per site.
  - Dir `.files()` in src: `agent/src/cache/dir.rs` (3 sites).
  - I/O constructors — **large**: `Dir::create_temp_dir` = **295 occurrences across 42 files** (dominated by tests), `Dir::new_home_dir` = 3, `Dir::new_current_dir` = 7.
- Tests: `agent/tests/filesys/{file.rs,dir.rs,cached_file.rs,path.rs,errors.rs}` plus many other `agent/tests/**` files that call `Dir::create_temp_dir` for fixtures.

## Naming decision (match core)

| Concept | Core name | New Rust free fn |
|---|---|---|
| module for File I/O | `files` package | `crate::filesys::files` |
| module for Dir I/O | `dirs` package | `crate::filesys::dirs` |
| temp dir ctor | `dirs.CreateTemp()` | `dirs::create_temp(prefix)` |
| home dir ctor | `dirs.Home()` | `dirs::home()` |
| current dir ctor | (core: none) | `dirs::current()` |

`create_temp` keeps the existing `prefix: &str` parameter (preserve behavior — do not drop the arg). `home`/`current` take no args and return `Result<Dir, FileSysErr>`.

---

## Method → free-function mapping

### File I/O → `agent/src/filesys/files.rs` (all take `&File` first arg)

| Old (method) | New (free fn) |
|---|---|
| `f.read_bytes().await` | `files::read_bytes(&f).await` |
| `f.read_string().await` | `files::read_string(&f).await` |
| `f.read_json::<T>().await` | `files::read_json::<T>(&f).await` |
| `f.read_secret_bytes().await` | `files::read_secret_bytes(&f).await` |
| `f.write_bytes(buf, opts).await` | `files::write_bytes(&f, buf, opts).await` |
| `f.write_string(s, opts).await` | `files::write_string(&f, s, opts).await` |
| `f.write_json(&obj, opts).await` | `files::write_json(&f, &obj, opts).await` |
| `f.append_bytes(buf, opts).await` | `files::append_bytes(&f, buf, opts).await` |
| `f.copy_to(&dst, opts).await` | `files::copy_to(&f, &dst, opts).await` |
| `f.move_to(&dst, ow).await` | `files::move_to(&f, &dst, ow).await` |
| `f.delete().await` | `files::delete(&f).await` |
| `f.create_symlink(&link, ow).await` | `files::create_symlink(&f, &link, ow).await` |
| `f.set_permissions(p).await` | `files::set_permissions(&f, p).await` |
| `f.permissions().await` | `files::permissions(&f).await` |
| `f.last_modified().await` | `files::last_modified(&f).await` |
| `f.size().await` | `files::size(&f).await` |
| `File::map_io_err_for_open` (private) | `map_io_err_for_open` (module-private free fn in files.rs) |
| `File::map_io_err_for_create` (private) | `map_io_err_for_create` (module-private free fn in files.rs) |
| `File::metadata` (private) | `metadata(&File)` (module-private free fn in files.rs) |

`sanitize_filename` is already a free fn in `file.rs` and is pure — **leave it in file.rs**.

### Dir I/O → `agent/src/filesys/dirs.rs` (all take `&Dir` first arg)

| Old (method) | New (free fn) |
|---|---|
| `d.create().await` | `dirs::create(&d).await` |
| `d.create_if_absent().await` | `dirs::create_if_absent(&d).await` |
| `d.delete().await` | `dirs::delete(&d).await` |
| `d.files().await` | `dirs::files(&d).await` |
| `d.subdirs().await` | `dirs::subdirs(&d).await` |
| `d.is_empty().await` | `dirs::is_empty(&d).await` |
| `d.delete_if_empty_recursive().await` | `dirs::delete_if_empty_recursive(&d).await` |
| `d.move_to(&dest, ow).await` | `dirs::move_to(&d, &dest, ow).await` |
| `d.set_permissions(p).await` | `dirs::set_permissions(&d, p).await` |
| `d.permissions().await` | `dirs::permissions(&d).await` |
| `Dir::create_temp_dir(prefix).await` | `dirs::create_temp(prefix).await` |
| `Dir::new_home_dir()` | `dirs::home()` |
| `Dir::new_current_dir()` | `dirs::current()` |
| `Dir::metadata` (private) | `metadata(&Dir)` (module-private in dirs.rs) |
| `Dir::is_not_empty_err` (private) | `is_not_empty_err` (module-private in dirs.rs) |
| `Dir::move_to_no_overwrite` (private) | `move_to_no_overwrite(&Dir, &Dir)` (module-private) |
| `Dir::move_to_with_overwrite` (private) | `move_to_with_overwrite(&Dir, &Dir)` (module-private) |
| `Dir::rename_to` (private) | `rename_to(&Dir, &Dir)` (module-private) |
| `read_dir_err`/`create_dir_err`/`delete_dir_err`/`move_dir_err` (already free, in dir.rs) | move as-is into dirs.rs (module-private) |

### Stay as methods (pure / PathExt) — DO NOT MOVE

- `File`: `new`, `name` (returns `Result`, pure path op), `parent` (returns `Result`, pure), `is_absolute`, `Display`, `PathExt::path`.
- `Dir`: `new`, `name` (`Result`, pure), `parent` (`Result`, pure — calls `abs_path()`, a pure PathExt method), `subdir`, `file`, `is_valid_dir_name`, `assert_valid_dir_name`, `Display`, `PathExt::path`.
- `PathExt` (path.rs): `path`, `abs_path`, `exists`, `assert_exists`, `assert_doesnt_exist` — unchanged.

### Internal cross-calls to rewrite inside the moved code

- In `files::append_bytes` / `write_bytes` / `copy_to` / `move_to`: `self.parent()?.create_if_absent()` → `dirs::create_if_absent(&file.parent()?)` (and `&dst.parent()?` for copy/move destinations).
- In `files::create_symlink`: `link.delete().await?` → `files::delete(link).await?`.
- In `files::read_secret_bytes`: `self.size()` → `size(file)` (same module).
- In `files::read_string` / `read_json`: `self.read_bytes()` → `read_bytes(file)`.
- In `files::write_string` / `write_json`: `self.write_bytes(...)` → `write_bytes(file, ...)`.
- In `dirs::move_to`: `dest_dir.parent()?.create_if_absent()` → `create_if_absent(&dest_dir.parent()?)`; branch calls to `move_to_no_overwrite`/`move_to_with_overwrite` become free-fn calls.
- In `dirs::move_to_with_overwrite`: `self.parent()?.subdir(...)`, `trash_dir.delete()` → `delete(&trash_dir)`; keep the `FileSysErr::DeleteDirErr` matching and rollback logic byte-for-byte.
- In `dirs::delete_if_empty_recursive`: `item.dir.subdirs()` → `subdirs(&item.dir)`; keep the queue/`seen_before` loop and `remove_dir` semantics identical.
- In `dirs::create_if_absent`: still delegates to `create(dir)`.
- `self.clone()` in error construction becomes `file.clone()` / `dir.clone()`.

---

## Milestones (ordered)

### M1 — Create `agent/src/filesys/files.rs`
- Module header comment block matching repo import ordering (`// standard crates` / `// internal crates` / `// external crates`).
- Move the 16 public File I/O methods + 3 private helpers (`metadata`, `map_io_err_for_open`, `map_io_err_for_create`) out of `file.rs` verbatim, converting `impl File { pub async fn foo(&self, …) }` → `pub async fn foo(file: &File, …)`. Replace every `self` with `file`. Preserve `trace!()`, TOCTOU comments, atomicwrites logic, `SecretBox` handling, and `Sync`/`Atomic`/`Overwrite` option checks exactly.
- Cross-calls to `dirs::create_if_absent` and intra-module fns as listed above.

### M2 — Create `agent/src/filesys/dirs.rs`
- Move the 10 public Dir I/O methods + 3 I/O constructors (`create_temp`, `home`, `current`) + private helpers (`metadata`, `is_not_empty_err`, `move_to_no_overwrite`, `move_to_with_overwrite`, `rename_to`) + the 4 module-level error helpers verbatim, converting to free fns on `&Dir`.
- Constructors: `home`/`current` return `Result<Dir, FileSysErr>`; `create_temp` async returns `Result<Dir, FileSysErr>`. Preserve `tempfile::Builder`, `std::env::var("HOME")`, `std::env::current_dir()` logic and error variants.

### M3 — Slim `file.rs`
- Leave only: struct `File`, `Display`, `PathExt` impl, and pure methods `new`, `is_absolute`, `name`, `parent`, plus free `sanitize_filename`.
- Remove now-unused imports (`atomicwrites`, `secrecy`, `tokio::io`, `SystemTime`, options structs if unreferenced). `scripts/lint.sh` runs machete + clippy `-D warnings`, so unused imports fail — remove them.

### M4 — Slim `dir.rs`
- Leave only: struct `Dir`, `Display`, `PathExt` impl, and pure methods `new`, `name`, `parent`, `is_valid_dir_name`, `assert_valid_dir_name`, `subdir`, `file`.
- Remove the 4 error-helper free fns (moved to dirs.rs) and now-unused error imports.

### M5 — Update `mod.rs`
- Add `pub mod files;` and `pub mod dirs;`.
- Keep existing `pub use` of `Dir`/`File`/`FileSysErr`/`PathExt` and all option structs (`Overwrite`, `Atomic`, `Sync`, `WriteOptions`, `AppendOptions`, `CopyOptions`) unchanged.

### M6 — Migrate `cached_file.rs`
- `file.read_json::<ContentT>().await` → `files::read_json::<ContentT>(&file).await`.
- `file.write_json(data, …).await` → `files::write_json(&file, data, …).await`.
- `self.file.write_json(&data, WriteOptions::OVERWRITE_ATOMIC).await` → `files::write_json(&self.file, &data, …).await`.
- Add `use crate::filesys::files;`.

### M7 — Migrate `agent/src` call sites (non-test)
Rewrite each measured site; add module import per file honoring import-ordering. Confirmed src sites:
- `agent/src/main.rs`: 2× `Dir::create_temp_dir` → `dirs::create_temp`; 2× `tmp_dir.delete()` (Dir) → `dirs::delete(&tmp_dir)`.
- `agent/src/storage/agent_version.rs`: `read_string`, `write_string`.
- `agent/src/storage/setup.rs`: 3× `write_json`; 2× `move_to` (**File**); `create_if_absent` (Dir), `delete` (Dir).
- `agent/src/provisioning/{provision.rs,reprovision.rs,shared.rs}`: `read_string`; `temp_dir.delete()` (Dir).
- `agent/src/events/store.rs`: `append_bytes`, `write_string`, `read_string`; `move_to` (**File**).
- `agent/src/deploy/filesys.rs`: `write_string`, `copy_to` (File), `move_to` (File and Dir — disambiguate per receiver); `delete` (check each receiver type).
- `agent/src/crypt/rsa.rs`: 2× `write_bytes`, 2× `read_secret_bytes`, 2× `set_permissions` (File).
- `agent/src/cache/file.rs`: 2× `write_json`.
- `agent/src/cache/dir.rs`: `create_if_absent` (Dir), `write_json` (File), `delete` (File), 3× `files()` (Dir → `dirs::files`).
- `agent/src/server/serve.rs`: `socket_file.delete()` (**File**).
- `agent/src/cache/single_thread.rs`: verify any File I/O and migrate.

For every ambiguous `.delete()` / `.create()` / `.create_if_absent()` / `.move_to()`: **inspect the receiver's declared type** to route to `files::` vs `dirs::`. A `grep`-only replace is unsafe.

### M8 — Migrate I/O constructors crate-wide
- `Dir::create_temp_dir(x)` → `dirs::create_temp(x)` : **295 occurrences / 42 files** (mostly `agent/tests/**` fixtures; also `filesys::Dir::create_temp_dir` fully-qualified). Distinctive token, unchanged args ⇒ scripted find/replace is safe; add `use` path where missing. Verify with `cargo build --tests`.
- `Dir::new_home_dir()` → `dirs::home()` : 3 sites.
- `Dir::new_current_dir()` → `dirs::current()` : 7 sites.

### M9 — Migrate remaining test call sites
- `agent/tests/filesys/{file.rs,dir.rs,cached_file.rs}` contain the bulk of File/Dir I/O method calls — migrate all to the free-fn form.
- Keep test filenames as-is; only migrate call bodies. Update `agent/tests/filesys/mod.rs` only if files are added/renamed.
- Other `agent/tests/**`: `create_temp` swaps (M8) plus any stray File/Dir I/O calls surfaced by the compiler.

### M10 — Validate (see Validation section)

---

## Test steps

Run from repo root `/home/ben/miru/workbench1/repos/agent`:

1. **Refresh lockfile before lint** (per AGENTS.md): `./scripts/update-deps.sh`.
2. **Build (incl. tests) to catch missed call sites**:
   - `cargo build --package miru-agent --all-features`
   - `cargo build --package miru-agent --features test --tests`
3. **Full test suite** (the `--features test` flag is mandatory): `./scripts/test.sh`.
   - Targeted while iterating: `RUST_LOG=off cargo test --features test --package miru-agent --test filesys`.
4. **Lint**: `./scripts/lint.sh` (import linter, `cargo fmt --check`, machete, diet, clippy `--all-features -D warnings`, security audit). Fix unused imports left by slimming file.rs/dir.rs.
5. **Coverage gate**: `./scripts/covgate.sh` — enforces `agent/src/filesys/.covgate`. Logic only moves; coverage should hold. Do **not** lower the gate.

## Validation

- Source of truth for "ready to publish" is `./scripts/preflight.sh`. It prints exactly **`Preflight clean`** on success.
- **Requirement: preflight must report `clean` before the change is published.** Do not push or open a PR while preflight is failing.
- Treat green `cargo build --tests` as the gate that all constructor renames landed, and green `./scripts/test.sh` as the gate that no behavior drifted.

---

## Risks

1. **Pure-but-fallible methods stay methods.** `File::name`/`File::parent`/`Dir::name`/`Dir::parent` return `Result<_, FileSysErr>` yet do no disk I/O — they **stay as methods**. Do not move them. Reviewers may wrongly flag them as "fallible ⇒ should be free fns" — call this out in the PR.
2. **`subdirs`/`files` are I/O returning path types.** They perform `read_dir` (I/O ⇒ free fn) but return `Vec<Dir>` / `Vec<File>`. They belong in `dirs.rs` — don't mistake the return type for a reason to keep them as methods.
3. **Ambiguous `.delete()` / `.create()` / `.create_if_absent()` / `.move_to()` receivers.** These names exist on both `File` and `Dir`. Routing requires per-site type inspection; a blind grep-replace will mis-route. Let the compiler catch mis-routes; budget manual review for `deploy/filesys.rs`, `storage/setup.rs`, `cache/dir.rs`.
4. **`create_temp_dir` blast radius (295 sites / 42 files).** Dominates effort. Distinctive token ⇒ scriptable, but each edited file needs the correct `use` path. Missing imports surface only at `cargo build --tests`.
5. **Coverage gate.** Moving code should preserve line coverage. If coverage dips, **do not ratchet the covgate value down** — ensure tests still exercise the moved paths. Flag any covgate change before proceeding.
6. **Unused-import fallout.** Slimming file.rs/dir.rs leaves dangling imports; clippy `-D warnings` + machete fail until cleaned. Expected.
7. **Import-ordering lint.** The custom import linter (`.lint-imports.toml`) enforces std/internal/external grouping with comment separators; new files and `use` lines must follow it.

---

### Critical Files
- `agent/src/filesys/file.rs`, `dir.rs`, `mod.rs`, `cached_file.rs`
- `agent/tests/filesys/{dir.rs,file.rs,cached_file.rs}` (largest test-migration surface)
