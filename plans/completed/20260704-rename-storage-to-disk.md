# Rename the local on-disk persistence module `storage` → `disk` (and `StorageErr` → `DiskErr`)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Rename the module directory `agent/src/storage/` → `agent/src/disk/`, its test directory `agent/tests/storage/` → `agent/tests/disk/`, the `pub mod storage;` declarations in `agent/src/lib.rs` and `agent/tests/mod.rs`, every reference `crate::storage` / `miru_agent::storage` / `storage::` throughout the crate, and the error type `StorageErr` → `DiskErr` (including the five aggregating error enums that carry it). Pure mechanical rename, no behavior change. |
| `agent/libs/` | untouched | Generated API code does not reference this module. |

This plan lives in `agent/plans/backlog/` because all code changes are inside the `agent/` repo. Work happens on branch `refactor/rename-storage-to-disk` (base `main`).

## Purpose / Big Picture

`agent/src/storage/` is the agent's **local on-disk persistence** layer. Its `mod.rs` re-exports the persisted stores (`config_instances`, `deployments`, `device`, `releases`, `settings`, `upload_rules`, `git_commits`), the on-disk `Layout`, the `setup`/`agent_version` bootstrap helpers, and the module's error type `StorageErr`. Every store persists JSON under the device's state directory via `Layout` paths — this is disk state, nothing to do with object storage.

New `s3` / `gcs` object-storage modules are being introduced on other branches. The name "storage" is now ambiguous: it reads as "object storage" but means "the local disk state module." Renaming it to `disk` disambiguates it up front so the object-storage modules land cleanly. The error type `StorageErr` is renamed to `DiskErr` to match.

There is **no behavior change**: same files, same on-disk format, same logic. Acceptance is "the module is named `disk`, the error type is `DiskErr`, the crate compiles, the full test suite passes, and `scripts/preflight.sh` prints `Preflight clean`." Because it is a pure rename, coverage should be unchanged and the module's covgate threshold (94.83) moves with it.

## Progress

- [x] (2026-07-04) Read this plan end-to-end. Confirmed on branch `refactor/rename-storage-to-disk`.
- [x] `git mv` the 12 tracked files under `agent/src/storage/` (incl. `.covgate`) to `agent/src/disk/`.
- [x] `git mv` the 11 tracked files under `agent/tests/storage/` to `agent/tests/disk/`.
- [x] `agent/src/lib.rs`: `pub mod storage;` → `pub mod disk;`, re-sorted into alphabetical position (moved up, between `deploy` and `errors`).
- [x] `agent/tests/mod.rs`: `pub mod storage;` → `pub mod disk;`, re-sorted (moved up, between `deploy` and `errors`).
- [x] Renamed all module-path references `crate::storage` / `miru_agent::storage` / `storage::` → `disk` across the referencing src/test files (plus the intra-module uses).
- [x] Renamed the error type `StorageErr` → `DiskErr` everywhere (86 occurrences): the definition + `impl_error!` block + `From` impls in the module's `errors.rs`; the `pub use` and `use ... as StorErr` alias-target in the module's `mod.rs`; and the five aggregating enums that carry it (`ServiceErr`, `ProvisionErr`, `SyncErr`, `DeployErr`, `ServerErr`) — variant name, `From<StorageErr>` impl, and `impl_error!` entry each.
- [x] Verified doc/module comments: none named the module as an identifier (only generic English "storage" prose remains, left as-is per plan).
- [x] `cargo build -p miru-agent` clean; `./scripts/test.sh` full suite passes (1495 tests, unchanged); `./scripts/lint.sh` clean (import linter + audit); `./scripts/covgate.sh` clean.
- [x] `./scripts/preflight.sh` prints `Preflight clean`.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

- Decision: Rename the module `storage` → `disk` and its error `StorageErr` → `DiskErr`; nothing else.
  Rationale: Disambiguate local disk state from the incoming `s3`/`gcs` object-storage modules. Pure mechanical rename, no behavior change. Date/Author: 2026-07-04 / ben@miruml.com.

- Decision: Leave the public `Storage` struct and the internal type aliases (`StorErr`, `StorLayout`, `DeviceStorage`, `CfgInstStor`) as-is.
  Rationale: The task scope is the module path and the `StorageErr` type. The `Storage` struct is a distinct identifier and renaming it is out of scope for this mechanical pass; renaming it (and every `.storage` field / local `storage` binding) would balloon the diff and risk behavior-adjacent churn. If desired, that is a separate follow-up. Date/Author: 2026-07-04 / ben@miruml.com.

## Outcomes & Retrospective

Completed 2026-07-04. Pure mechanical rename, no behavior change.

- Moved 13 tracked files `agent/src/storage/*` → `agent/src/disk/*` (12 `.rs` + `.covgate`) and 11 `agent/tests/storage/*` → `agent/tests/disk/*` via `git mv` (renames preserved history; `.covgate` gate of 94.83 now lives at `agent/src/disk/.covgate`).
- Registered `pub mod disk;` in `agent/src/lib.rs` and `agent/tests/mod.rs`, re-sorted between `deploy` and `errors`; removed the old `storage` line.
- Renamed module-path references (`crate::storage` ×30, `miru_agent::storage` ×34, bare `storage::` module segments) → `disk` equivalents, and the error type `StorageErr` → `DiskErr` (86 occurrences), including the five aggregating enums' variant/`From`/`impl_error!` entries.
- Total diff vs `main`: 75 files changed, 269 insertions / 269 deletions — perfectly balanced, confirming a 1-for-1 token rename with no line additions or removals.
- Watch-outs honored: the `filesys` module path (`From<filesys::FileSysErr>`) unchanged; the `gcs` test var in `tests/sync/deployments.rs` untouched; local `storage` bindings/fields, the `Storage` struct, the `StorageOptions` type, and the internal aliases `StorErr`/`StorLayout`/`DeviceStorage`/`CfgInstStor` all left as-is.
- Validation: `cargo build` clean; `./scripts/test.sh` 1495 tests pass (unchanged); `./scripts/lint.sh` clean; `./scripts/covgate.sh` clean; `./scripts/preflight.sh` prints `Preflight clean`.
- No deviations from plan. Committed as a single commit (a "move-only" first commit would not build, since the reference updates are inseparable from a buildable state — the plan explicitly allowed a single commit in that case).

## Context and Orientation

- `agent/src/storage/mod.rs` — module root. Re-exports the stores and `pub use self::errors::{DeviceNotActivatedErr, StorageErr};`. Also has three internal aliases at the top of the file: `use self::errors::StorageErr as StorErr;`, `use self::layout::Layout as StorLayout;`, `use self::device::Device as DeviceStorage;`. Only the `StorageErr` symbol is renamed to `DiskErr`; the `as StorErr` alias target changes (`use self::errors::DiskErr as StorErr;`) but the alias name `StorErr` itself may stay (it is not `StorageErr`).
- `agent/src/storage/errors.rs` — defines `pub enum StorageErr`, its `From<CacheErr>`/`From<CryptErr>`/`From<FileSysErr>` impls, and the `crate::impl_error!(StorageErr { ... });` registration. All of these `StorageErr` tokens → `DiskErr`. The sibling error structs (`DeviceNotActivatedErr`, `JoinHandleErr`, `PruneCacheErrs`, `ResolveDeviceIDErr`) are **not** renamed.
- `agent/src/lib.rs` — the crate's module list (kept alphabetical). `pub mod storage;` currently sits between `services` and `sync`; `pub mod disk;` sorts between `deploy` and `errors`, so the line **moves position** — do not just rename in place, or the import linter / a human reviewer will flag the ordering.
- `agent/tests/mod.rs` — same story, `pub mod storage;` → `pub mod disk;` and re-sorted (between `deploy` and `errors`).
- The **five aggregating error enums** each carry `StorageErr(StorageErr)`: `agent/src/services/errors.rs:27`, `agent/src/provisioning/errors.rs:45`, `agent/src/deploy/errors.rs:113`, `agent/src/sync/errors.rs:94`, `agent/src/server/errors.rs:110`. Each also has a `From<StorageErr> for <Enum>` impl and a bare `StorageErr,` line inside its own `impl_error!` block. All three sites per enum become `DiskErr`.

### Blast radius (measured on `refactor/rename-storage-to-disk`)

Reference counts across `agent/src` + `agent/tests`:

| Pattern | Count |
|---|---|
| `crate::storage` | 30 |
| `miru_agent::storage` | 34 |
| `storage::` (all) | 179 |
| `use …storage` (module import lines) | 63 |
| `StorageErr` | 86 |

Files to move: **12** under `agent/src/storage/` (`agent_version.rs`, `config_instances.rs`, `deployments.rs`, `device.rs`, `errors.rs`, `git_commits.rs`, `layout.rs`, `mod.rs`, `releases.rs`, `settings.rs`, `setup.rs`, `upload_rules.rs`) **plus `.covgate`**; **11** under `agent/tests/storage/` (`agent_version.rs`, `caches.rs`, `deployments.rs`, `device.rs`, `errors.rs`, `init.rs`, `layout.rs`, `mod.rs`, `settings.rs`, `setup.rs`, `upload_rules.rs`).

Distinct files edited (references, outside the moved dirs): **28** in `agent/src`, **23** in `agent/tests`. Total distinct files touched by the rename (moves + edits): **66**.

### Watch-outs (do NOT touch these)

- **`filesys` module** (`agent/src/filesys/`) — low-level FS primitives (`FileSysErr`, `File`, `Overwrite`, etc.). Unrelated to this rename. Leave untouched. `StorageErr` has a `From<filesys::FileSysErr>` impl — keep that path as `filesys`.
- **No `s3`/`gcs` module on `main`** — they are on other branches. There is no collision to worry about here. The only literal `gcs` token in the tree is a **local test variable** `let gcs = f.git_commit_stor.values()...` in `agent/tests/sync/deployments.rs:1455` (git-commit-stor, not the module). Do not rename it.
- **Local bindings / fields literally named `storage`** — e.g. `let storage = Arc::new(stor);` in `agent/src/app/state.rs:49`, `self.storage.shutdown()` in `app/state.rs:117`, `args.storage` / `storage_ref` / `sync_storage` in `agent/src/sync/syncer.rs` and `agent/src/sync/deployments.rs`, and the `pub storage: &'a Storage<'a>` struct fields. These are **not** the module path and must **not** be renamed. Prefer precise replacements of the module path (`crate::storage`, `miru_agent::storage`, `storage::`) and the type (`StorageErr`) over a blind `sed 's/storage/disk/g'`.
- **Generic English "storage"** in prose comments (e.g. "shutdown storage", "storage layout stuff") need not change; only comments that name the module as an identifier (e.g. a rustdoc link or a `// see storage::setup`) should update.

## Plan of Work

1. **Move the source module.** `git mv` each of the 12 files (including `.covgate`) from `agent/src/storage/` to `agent/src/disk/`. Using per-file `git mv` preserves history and keeps the `.covgate` gate with the module. Remove the now-empty `agent/src/storage/` directory.

2. **Move the test module.** `git mv` each of the 11 files from `agent/tests/storage/` to `agent/tests/disk/`. Remove the empty directory.

3. **Update module declarations.** In `agent/src/lib.rs` and `agent/tests/mod.rs`, rename `pub mod storage;` → `pub mod disk;` and **move the line into alphabetical position** (between `deploy` and `errors`).

4. **Rename module-path references.** Replace `crate::storage` → `crate::disk`, `miru_agent::storage` → `miru_agent::disk`, and the leading `storage::` path segment → `disk::` across all 51 referencing files, plus the intra-module `self::`/`super::` uses inside the moved files. Be surgical: match the path segment, not bare `storage` substrings inside other identifiers or in prose.

5. **Rename the error type.** Replace `StorageErr` → `DiskErr` everywhere (86 occurrences). This covers:
   - the definition, `From` impls, and `impl_error!(StorageErr { … })` block in `agent/src/disk/errors.rs`;
   - the `pub use self::errors::{DeviceNotActivatedErr, StorageErr};` and the `use self::errors::StorageErr as StorErr;` alias-target in `agent/src/disk/mod.rs` (`Result<…, StorErr>` return types unchanged; only the `StorageErr` token they alias becomes `DiskErr`);
   - each of the five aggregating enums (`ServiceErr`, `ProvisionErr`, `SyncErr`, `DeployErr`, `ServerErr`): the `StorageErr(StorageErr)` variant → `DiskErr(DiskErr)`, the `From<StorageErr> for <Enum>` impl, and the bare `StorageErr,` line in each enum's `impl_error!` block;
   - test helpers named `fn storage_err() -> StorageErr` and `matches!(…, XErr::StorageErr(_))` assertions in `agent/tests/**`.

6. **Doc/comment sweep.** Update any doc comment or `//` comment that names the module as an identifier. Skip generic English uses of the word "storage."

7. **Validate.** Build, then run the test / lint / covgate / preflight gates (Concrete Steps below). Iterate until `Preflight clean`.

## Concrete Steps

All commands run from `/home/ben/miru/workbench5/repos/agent/` (the agent repo root).

1. Confirm branch and move the module dirs preserving history:

       git rev-parse --abbrev-ref HEAD   # expect refactor/rename-storage-to-disk
       git mv agent/src/storage agent/src/disk
       git mv agent/tests/storage agent/tests/disk

   (`git mv` on the directory moves all tracked files including `.covgate` in one shot; if it complains, fall back to per-file `git mv`.)

2. Rename the module declarations by hand (they need re-sorting, not just a substring swap): edit `agent/src/lib.rs` and `agent/tests/mod.rs` so `pub mod disk;` sits between `pub mod deploy;` and `pub mod errors;`, and the old `storage` line is gone.

3. Rename module-path references. Do the three unambiguous path forms first:

       grep -rl 'crate::storage'     agent/src agent/tests | xargs sed -i 's/crate::storage/crate::disk/g'
       grep -rl 'miru_agent::storage' agent/src agent/tests | xargs sed -i 's/miru_agent::storage/miru_agent::disk/g'

   Then the bare `storage::` path segment — guard the left boundary so you never touch `something_storage::` or a field access. Inspect the hits first:

       grep -rnE '(^|[^_A-Za-z0-9])storage::' agent/src agent/tests

   and apply `sed -i -E 's/(^|[^_A-Za-z0-9])storage::/\1disk::/g'` only to those files. Re-grep to confirm zero remaining `storage::` that are the module.

4. Rename the error type (surgical — `StorageErr` is a unique token, safe to replace globally):

       grep -rl 'StorageErr' agent/src agent/tests | xargs sed -i 's/StorageErr/DiskErr/g'

   This handles the definition, the five aggregating enums' variant/`From`/`impl_error!` entries, and all test helpers/assertions at once.

5. Sweep for stragglers and false negatives:

       grep -rnE '\bstorage\b' agent/src agent/tests   # review: any left should be local vars / prose only
       grep -rn 'StorageErr' agent/src agent/tests      # expect: 0
       grep -rn 'pub mod storage' agent/src agent/tests # expect: 0

6. Build the library and tests:

       cargo build -p miru-agent
       cargo test -p miru-agent --no-run --features test

   Expected: clean. A leftover `storage::` path or a missed `DiskErr` variant surfaces here as an unresolved-path / unknown-variant error.

7. Run the project gates:

       ./scripts/test.sh        # RUST_LOG=off cargo test --features test — full suite passes
       ./scripts/update-deps.sh # refresh Cargo.lock before linting
       ./scripts/lint.sh        # custom import linter (import-group name changes → may reorder), fmt, machete, cargo audit, clippy
       ./scripts/covgate.sh     # renamed module keeps its .covgate gate (94.83); coverage unchanged

8. Run the full preflight gate and confirm the final line:

       ./scripts/preflight.sh   # expect final line: "Preflight clean"

9. Only after `Preflight clean` prints, open the PR (base `main`).

## Validation and Acceptance

Acceptance is `./scripts/preflight.sh` printing `Preflight clean` on branch `refactor/rename-storage-to-disk`. Along the way:

- `cargo build -p miru-agent` and `cargo test -p miru-agent --no-run --features test` succeed — confirms every module-path and error-type reference resolved.
- `grep -rn 'StorageErr\|storage::\|pub mod storage' agent/src agent/tests` returns nothing that is the module/type (only local `storage` bindings and prose remain).
- `./scripts/test.sh` reports the **same** passing-test count as `main` — a pure rename changes no test outcomes.
- `./scripts/lint.sh` passes. The custom import-order linter is the most likely to complain: the import group for the module changes name (`disk` sorts before `storage`), so import lines that reference it may need reordering within their group. `cargo audit` is unaffected (no dependency change).
- `./scripts/covgate.sh` passes with the module's `.covgate` (94.83) now living under `agent/src/disk/`. Coverage is unchanged because no lines were added or removed.

Operator-visible behavior: none. Same on-disk format, same paths, same logic.

## Idempotence and Recovery

All steps are local and re-runnable.

- The `sed` passes are idempotent: `crate::storage`/`StorageErr` tokens are gone after the first run, so re-running is a no-op.
- If a build error points at a missed site, fix it and re-run `cargo build -p miru-agent`; grep the error output for `storage` to locate the next site.
- `git restore <path>` (or `git mv` back) backs out any individual file; `git checkout -- .` / `git reset --hard` resets the whole working tree to the branch tip.
- `./scripts/preflight.sh` is idempotent — running it twice yields the same result.

There is nothing destructive here: no on-disk format change, no wire-format change, no data migration. It is a rename of an internal module and its error type.
