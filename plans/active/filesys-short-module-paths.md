# ExecPlan: Normalize filesys call sites to short `files::` / `dirs::` paths

## Goal

Follow-up to the filesys free-function refactor (PR #106). Replace the
fully-qualified call-site convention `filesys::files::…` / `filesys::dirs::…`
(and any `crate::filesys::…` / `miru_agent::filesys::…` prefixed forms) with the
short `files::…` / `dirs::…` form, importing the modules directly — matching the
files that already use the short form (e.g. `use miru_agent::filesys::{dirs, PathExt};`).

Pure mechanical convention change; no behavior changes.

## Scope (measured on the base branch)

- 799 call sites across 40 files (373 `files::`, 426 `dirs::`); zero `filesys::dir::`.
- Dominant callers are test files; `filesys::dirs::create_temp` alone is ~288 sites.

## Approach

1. **Call-site substitution (prefix-aware):** rewrite `crate::filesys::`, `miru_agent::filesys::`, and bare `filesys::` prefixes on `files::`/`dirs::` to the short form. Prefix-awareness matters: a naive `filesys::files::`→`files::` replace corrupts fully-qualified paths into `crate::files::` / `miru_agent::files::`.
2. **Import fix (per file):** ensure `files`/`dirs` are in scope in the carrier `use …::filesys::{…}` statement. Nested test mods use `use super::*`, so fixing the top-level filesys import propagates. Drop a now-unused `self` where the module is no longer referenced; keep it where `filesys::Type` is still used inline.
3. **Multiline imports:** 3 files (`filesys/files.rs`, `tests/filesys/{file,cached_file}.rs`) have multiline `use …filesys::{` blocks handled individually.
4. **Name collision:** `tests/filesys/dir.rs` has a test submodule `mod files` (tests `dirs::files()`) that collides with importing the `files` module → rename it to `mod list_files`. Only collision in the tree.

## Test steps
- `cargo build --package miru-agent --features test --tests` — 0 errors, 0 warnings (unused-import/self)
- `RUST_LOG=off cargo test --features test` — full suite green

## Validation
- `./scripts/preflight.sh` must report **`Preflight clean`** before the change is published. (Note: `workers` covgate is a pre-existing local-vs-CI gap unrelated to this change — see memory `agent-covgate-workers-local-gap`.)

## Risks
- Fully-qualified path corruption if replacement isn't prefix-aware (handled).
- `mod files` collision in dir.rs (renamed to `list_files`; no path refs to the mod elsewhere, uses `super::*`).
- Unused `self`/`files`/`dirs` imports → clippy `-D warnings`; caught by build warnings pass.
