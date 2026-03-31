# Import Linter: Normalize Internal Imports and Forbid `super::` in Source

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Extend `tools/lint/` and wire the new rule into the existing agent lint workflow |

This plan lives in `agent/.ai/exec-plans/completed/` because implementation and validation are complete.

## Purpose / Big Picture

The agent repository already has a custom import linter in `tools/lint/` that enforces import group ordering and header comments. It does not currently enforce two repository-specific style rules:

1. Internal imports should be minimized by grouping same-parent imports together instead of scattering multiple `use crate::<module>::...` lines.
2. Production source code should not use `super::...` imports; absolute `crate::...` imports are preferred. `#[cfg(test)]` code is explicitly exempt.

After this work:

- `./scripts/lint.sh` will detect and auto-fix split internal imports such as separate `crate::filesys::dir::Dir`, `crate::filesys::path::PathExt`, and `crate::filesys::{Overwrite, WriteOptions}` lines into a single grouped `crate::filesys::{...}` import.
- `./scripts/lint.sh` will rewrite top-level production `super::...` imports under `agent/src/` to absolute `crate::...` imports based on the file's module path.
- The fixer will remain idempotent, and the resulting imports will stay compatible with the repository's existing header-group convention.

## Progress

- [x] (2026-03-06) Read all top-level `.ai/rules/*.mdc` files, `agent/AGENTS.md`, `agent/ARCHITECTURE.md`, and `agent/TECH_DEBT.md`.
- [x] (2026-03-06) Inspected the existing import linter implementation in `tools/lint/` and the shared lint scripts in `scripts/lib/lint.sh`.
- [x] (2026-03-06) Sampled current import patterns in `agent/src/` and `agent/tests/` to scope the rule safely.
- [x] (2026-03-06) Drafted implementation and validation steps for the new normalization rule in this backlog ExecPlan.
- [x] (2026-03-06) Implemented the linter changes, migrated `agent/src/` and `agent/tests/`, and verified the resulting import layout with formatter, clippy, and test passes.

## Surprises & Discoveries

- The existing linter only parses the leading import block at the top of each file. That is acceptable for this change because all current production `use super::...` imports in `agent/src/` occur in top-level import blocks; the many nested `use super::*` occurrences are inside test modules and should remain allowed.
- Current production code has `37` top-level `use super::...` imports across `19` files under `agent/src/`.
- Current production code has at least `11` files with repeated top-level `crate::<module>` prefixes that are obvious merge candidates even before converting `super::...` imports.
- The current lint script runs `cargo fmt` before the custom import linter. That order was fine when the linter only regrouped whole statements, but import-tree rewriting may require either canonical rendering in the fixer or a script-order adjustment so `cargo fmt` runs after fixes.
- The repository test wrapper really does require unsandboxed local socket access. In this environment, `./scripts/test.sh` failed under the sandbox with `PermissionDenied` on `/tmp/miru.sock` and passed once rerun outside the sandbox.

## Decision Log

- Decision: limit the first normalization pass to internal imports rooted at `crate::...` plus absolute rewrites of `super::...`.
  Rationale: this directly addresses the requested repository style without expanding churn to external imports such as `tokio` or `serde`. External import merging can be added later if desired.
  Date/Author: 2026-03-06 / Codex

- Decision: keep `#[cfg(test)]` and nested test-module `use super::*` patterns out of scope for the first pass.
  Rationale: the user explicitly allows `super::` in test code, and the existing linter's top-of-file parser already cleanly separates production top-level imports from nested test-only imports.
  Date/Author: 2026-03-06 / Codex

- Decision: derive `crate::...` replacements for `super::...` from the file path instead of from string heuristics alone.
  Rationale: correct rewriting depends on the file's module path, especially for `mod.rs` files and multi-segment paths such as `agent/src/http/deployments.rs`.
  Date/Author: 2026-03-06 / Codex

## Outcomes & Retrospective

- [x] New linter rule exists for grouped internal imports and non-test `super::` imports. `tools/lint` now parses `use` trees with `syn`, normalizes mergeable `crate::...` imports, and rewrites top-level source `super::...` imports to absolute `crate::...`.
- [x] Fix mode rewrote the current repository violations in `agent/src/` and the remaining top-level test-side split imports in `agent/tests/`.
- [x] The linter passes its own formatting, unit tests, and clippy checks.
- [x] Validation completed successfully with:
  - `cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src --config .lint-imports.toml`
  - `cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/tests --config .lint-imports.toml`
  - `cargo fmt -p miru-agent -- --check`
  - `cargo clippy --package miru-agent --no-deps --all-targets --all-features -- -D warnings`
  - `./scripts/test.sh` (outside the sandbox because the suite binds `/tmp/miru.sock`)

## Context and Orientation

### Current linter architecture

The agent import linter lives in `agent/tools/lint/` and currently has five modules:

- `parser.rs` parses the leading import block into `UseStatement`s, comments, and blank lines.
- `classifier.rs` classifies each `use` as standard, internal, or external.
- `checker.rs` validates group ordering and header labels.
- `fixer.rs` rewrites the import block to restore group order and required headers.
- `config.rs` loads `.lint-imports.toml`.

It does not currently understand the structure of a `use` tree beyond the first root segment (`crate`, `tokio`, `std`, etc.).

### Current production patterns to normalize

Representative examples already in `agent/src/`:

- `agent/src/filesys/file.rs` splits `crate::filesys` imports across four lines that should become one grouped import.
- `agent/src/server/handlers.rs` imports four `crate::services::<module> as ...` entries that should become one grouped `crate::services::{...}` import.
- `agent/src/deploy/apply.rs` mixes `super::errors`, `super::{filesys as dpl_filesys, fsm}`, and separate `crate::filesys` imports; this file exercises both new rules at once.
- `agent/src/http/deployments.rs` uses four `super::...` imports that should become a single grouped absolute import under `crate::http::{...}`.

### Boundaries

- Generated code under `libs/backend-api/` and `libs/device-api/` is out of scope.
- Test-only nested `use super::*` inside `#[cfg(test)]` modules are out of scope for the rewrite rule.
- The first pass only normalizes top-of-file import blocks, matching the current linter's operating model.

## Plan of Work

### Milestone 1: Add structured import-tree normalization support

Files:

- `tools/lint/Cargo.toml`
- `tools/lint/src/parser.rs`
- `tools/lint/src/checker.rs`
- `tools/lint/src/fixer.rs`
- possibly a new helper module such as `tools/lint/src/normalizer.rs`

Implementation outline:

- Add a real parser for individual `use` statements using `syn::ItemUse` so the linter can reason about nested import trees, renames, globs, and `self`.
- Introduce a canonical internal representation for mergeable imports rooted at `crate::...`.
- Keep the existing top-of-file block parser for comments and blank-line preservation, but augment each `UseStatement` with enough structure to compare and rewrite internal imports canonically.
- Treat attributed imports conservatively: if a `use` statement has attributes, leave it as a standalone statement unless a safe merge path is explicitly proven.

Deliverable:

- The linter can build a normalized view of a file's internal imports rather than only classifying whole lines.

### Milestone 2: Rewrite production `super::...` imports to absolute `crate::...`

Files:

- `tools/lint/src/checker.rs`
- `tools/lint/src/fixer.rs`
- helper module if added

Implementation outline:

- Compute the module path for each scanned file from its path relative to the crate root:
  - `agent/src/http/deployments.rs` => module path `http::deployments`
  - `agent/src/storage/mod.rs` => module path `storage`
  - `agent/src/main.rs` => crate root
- For top-level imports in production source files, rewrite:
  - `super::errors::HTTPErr` => `crate::http::errors::HTTPErr`
  - `super::{request, ClientI}` => `crate::http::{request, ClientI}`
- Emit a dedicated diagnostic for non-test `super::` imports so check mode clearly explains why the file is non-canonical.
- Do not touch nested `use super::*` inside `#[cfg(test)]` modules because they are outside the linter's top-of-file scope and are explicitly allowed.

Deliverable:

- The fixer can eliminate current production `super::...` imports in `agent/src/` automatically.

### Milestone 3: Merge same-parent internal imports into grouped `crate::...::{...}` trees

Files:

- `tools/lint/src/checker.rs`
- `tools/lint/src/fixer.rs`
- helper module if added

Implementation outline:

- Normalize mergeable internal imports by parent path:
  - `use crate::filesys::dir::Dir;`
  - `use crate::filesys::path::PathExt;`
  - `use crate::filesys::{Atomic, Overwrite, WriteOptions};`
  becomes:
  - `use crate::filesys::{dir::Dir, path::PathExt, Atomic, Overwrite, WriteOptions};`
- Support merges that require `self`, for example:
  - `use crate::filesys;`
  - `use crate::filesys::Overwrite;`
  becomes:
  - `use crate::filesys::{self, Overwrite};`
- Preserve aliases during merge:
  - `use crate::services::deployment as dpl_svc;`
  - `use crate::services::device as dvc_svc;`
  becomes:
  - `use crate::services::{deployment as dpl_svc, device as dvc_svc};`
- Preserve globs when present:
  - `use crate::sync::{errors::*, syncer::SyncerExt};`

Diagnostics:

- Add a dedicated diagnostic when multiple mergeable internal imports share the same parent prefix and should be grouped.
- Ensure fix mode produces a single canonical grouped import per mergeable internal parent path.

Deliverable:

- Repeated `crate::<module>::...` imports in a file collapse into the minimal grouped form automatically.

### Milestone 4: Make fix mode formatting-safe

Files:

- `scripts/lib/lint.sh`
- possibly `tools/lint/src/fixer.rs`

Implementation outline:

- Choose one of two approaches and document it in the implementation:
  - render canonical imports in a format that already matches `cargo fmt` output, or
  - run the custom import linter before `cargo fmt` in fix mode so formatter cleanup happens after import rewriting
- Keep CI check mode behavior explicit: `cargo fmt --check` and the custom import linter should still fail independently with actionable output.

Deliverable:

- Fix mode leaves the working tree both style-correct and formatter-clean.

### Milestone 5: Add regression coverage in the linter itself

Files:

- `tools/lint/src/checker.rs`
- `tools/lint/src/fixer.rs`
- `tools/lint/src/parser.rs`
- optional fixture-based tests under `tools/lint/tests/`

Test plan:

- Happy path:
  - already-grouped internal imports remain unchanged
  - already-absolute `crate::...` imports remain unchanged
- Invalid input or error path:
  - split internal imports produce diagnostics
  - production `super::...` imports produce diagnostics
- Boundary and edge cases:
  - `mod.rs` path resolution for `super::...`
  - `crate::module::{self, Item}` merges
  - alias-preserving merges
  - glob-preserving merges
  - attributed imports remain separate if unsafe to merge
- Side-effect/dependency behavior:
  - fix mode is idempotent across repeated runs
  - script ordering still yields formatter-clean output

Deliverable:

- The linter can prove the new rule works before it rewrites the repository.

### Milestone 6: Run the fixer on agent source and verify clean lint/test results

Files:

- affected `agent/src/**/*.rs` files

Implementation outline:

- Run the custom linter in fix mode over `agent/src`.
- Review the resulting import churn with focus on:
  - `filesys/*`
  - `http/*`
  - `server/*`
  - `deploy/*`
  - `storage/*`
- Re-run the full agent lint path and the linter tool's own tests.

Deliverable:

- Repository source is migrated to the new canonical import style and the standard lint commands pass.

## Concrete Steps

1. Add structured import parsing support to `tools/lint/` and teach it how to render canonical grouped internal imports.
2. Add file-path-to-module-path resolution so the fixer can convert top-level production `super::...` imports into absolute `crate::...` paths.
3. Update checker diagnostics to report both split internal imports and forbidden production `super::...` imports.
4. Update fix mode or shared lint-script ordering so formatter output remains clean after rewrites.
5. Add focused tests that cover `foo.rs`, `mod.rs`, aliases, globs, `self`, and idempotence.
6. Run the fixer over `agent/src`, inspect the diff, and re-run lint/test commands.

## Validation and Acceptance

Acceptance criteria:

- `agent/src/filesys/file.rs` and similar files collapse repeated `crate::filesys::...` imports into a single grouped import.
- `agent/src/http/*.rs`, `agent/src/deploy/*.rs`, `agent/src/server/*.rs`, and similar files no longer use top-level `super::...` imports in production code.
- Running the fixer twice produces no additional changes.
- `cargo test` from `tools/lint/` passes with coverage for the new normalization logic.
- `LINT_FIX=0 ./scripts/lint.sh` from `agent/` passes after the migration.

Suggested commands:

- From `agent/tools/lint/`: `cargo test`
- From `agent/tools/lint/`: `cargo clippy --all-targets -- -D warnings`
- From `agent/`: `cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src --fix --config .lint-imports.toml`
- From `agent/`: `LINT_FIX=0 ./scripts/lint.sh`
- From `agent/`: `./scripts/test.sh`

## Idempotence and Recovery

- The tool changes are idempotent if canonical rendering is stable; re-running fix mode should yield zero diff after the first pass.
- Repository migration is recoverable by reverting only the rewritten import blocks in touched files if a rendering bug is discovered.
- If the `super::` path-resolution logic proves incorrect for `mod.rs` or similar layouts, disable only that rewrite path and keep grouped-import normalization isolated until the module-path bug is fixed.
