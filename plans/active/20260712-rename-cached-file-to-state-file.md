# Rename the `CachedFile` abstraction to `StateFile`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Rename `agent/src/filesys/cached_file.rs` → `agent/src/filesys/state_file.rs`, its mirror test file `agent/tests/filesys/cached_file.rs` → `agent/tests/filesys/state_file.rs`, the types `SingleThreadCachedFile` → `SingleThreadStateFile` and `ConcurrentCachedFile` → `ConcurrentStateFile`, and every reference (module declarations, imports, type aliases, local bindings, comments, one `info!` log message). Pure mechanical rename, zero behavior change. |
| `libs/` | untouched | Generated OpenAPI code; contains no reference to `cached_file` (verified by grep). |

This plan lives in `plans/backlog/` of the agent repo because all changes are inside this repo. Work happens on branch `claude/scanner-state-persistence-4gbd9c` (already created and pushed, identical to `main` at `dd4940d`).

## Purpose / Big Picture

`agent/src/filesys/cached_file.rs` defines a small persistence abstraction: a typed JSON file on disk whose current content is kept in memory. `SingleThreadCachedFile<ContentT, PatchT>` is the direct owner variant; `ConcurrentCachedFile<ContentT, PatchT>` wraps it in a tokio actor (`Worker` + `Command` channel) for shared use. Its two consumers use it to persist agent state: the auth token (`TokenFile` in `agent/src/authn/token_mngr.rs`) and the device record (`Device` in `agent/src/disk/device.rs`). The name "cached file" undersells what it is — it is the durable state file, not a cache that can be dropped. This rename makes the intent explicit: `StateFile`.

There is no behavior change: same files on disk, same JSON format, same actor logic, same public API shape (only names change). Acceptance is: the crate compiles, the full test suite passes unchanged via `./scripts/test.sh`, lint is clean, the verification grep returns zero hits, and CI is green on the pushed branch head.

## Progress

- [ ] Milestone 1: rename files, symbols, references; commit.
- [ ] Milestone 2: local validation (build, test, lint, grep gate) and CI green on pushed head.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Rename `CachedFile` → `StateFile` (both variants), the module file, the mirror test file, and every textual reference to "cached file" in `agent/`; also rename the internal field `cache: Arc<ContentT>` → `state` since the change is trivial and mechanical. Keep the traits `ConcurrentContentT` / `ConcurrentPatchT`, the `Command` enum, the `Worker` struct, and the `dispatch!` macro as-is.
  Rationale: The abstraction is durable state, not a discardable cache. Field rename keeps the module internally consistent; the traits/actor plumbing do not embed the old name. Date/Author: 2026-07-12 / agents@miruml.com.
- Decision: Leave the two test comments "should still be able to read the file since it's cached in memory" (`agent/tests/filesys/cached_file.rs` lines 204 and 586 pre-rename) untouched.
  Rationale: They describe real behavior (content held in memory), do not embed the type name, and do not match the verification grep patterns. Date/Author: 2026-07-12 / agents@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (a Rust workspace; the binary crate is `miru-agent` under `agent/`). All paths below are relative to the repo root. All commands run from the repo root.

Complete inventory of references (verified with `grep -rni "cachedfile\|cached_file\|cached file"` across the repo; nothing else references these names — not `agent/src/lib.rs`, not `ARCHITECTURE.md`, not `.lint-imports.toml`, not `libs/`):

- `agent/src/filesys/cached_file.rs` — the module itself (279 lines):
  - line 29 `pub struct SingleThreadCachedFile<ContentT, PatchT>` and line 38 its `impl` block
  - line 34 field `cache: Arc<ContentT>`; line 43 local `let cache = ...`; uses of `self.cache` at lines 48, 87, 92, 97, 98
  - lines 46, 51, 60 local bindings named `cached_file`
  - line 146 `Worker` field `pub file: SingleThreadCachedFile<ContentT, PatchT>`
  - line 186 `pub struct ConcurrentCachedFile<ContentT, PatchT>` and line 194 its `impl` block
  - lines 205, 219 `SingleThreadCachedFile::new(...)` / `::new_with_default(...)` calls inside `spawn` / `spawn_with_default`
  - line 252 `info!("{} cached file shutdown complete", ...)` in `ConcurrentCachedFile::shutdown`
- `agent/src/filesys/mod.rs` — line 1 `pub mod cached_file;` (module list is alphabetical: cached_file, dir, dirs, errors, file, files, path). There are no re-exports of the module's items.
- `agent/src/authn/token_mngr.rs` — line 6 `use crate::filesys::{cached_file::SingleThreadCachedFile, file::File, path::PathExt};` and line 25 `pub type TokenFile = SingleThreadCachedFile<Token, token::Updates>;`
- `agent/src/disk/device.rs` — line 8 `use crate::filesys::{cached_file::ConcurrentCachedFile, files, PathExt};` and line 12 `pub type Device = ConcurrentCachedFile<models::Device, device::Updates>;`
- `agent/tests/filesys/cached_file.rs` — mirror test file (736 lines, per the repo rule that `agent/tests/` mirrors `agent/src/`): line 4 imports both types from `cached_file::`, line 14 `type SingleThreadTokenFile = SingleThreadCachedFile<Token, Updates>;`, line 341 `type ConcurrentTokenFile = ConcurrentCachedFile<Token, Updates>;`, banner comments at lines 13 (`SINGLE THREADED CACHED FILE`) and 340 (`MULTI THREADED CACHED FILE`), and ~60 local bindings named `cached_file`.
- `agent/tests/filesys/mod.rs` — line 1 `pub mod cached_file;` (alphabetical list, same as src).

Out of scope, intentionally unchanged: `ConcurrentPatchT` / `ConcurrentContentT` traits, `Command` enum, `Worker` struct, `dispatch!` macro, `agent/src/filesys/.covgate` (per-directory coverage gate — a file rename within the same module directory does not affect it), and historical documents under `plans/` that mention `cached_file`.

## Plan of Work

Milestone 1 — the rename (single commit; a split "move-only" commit would not compile because the module declaration and references must change together):

1. `git mv agent/src/filesys/cached_file.rs agent/src/filesys/state_file.rs` and `git mv agent/tests/filesys/cached_file.rs agent/tests/filesys/state_file.rs`.
2. In `agent/src/filesys/state_file.rs`: rename `SingleThreadCachedFile` → `SingleThreadStateFile` and `ConcurrentCachedFile` → `ConcurrentStateFile` (all occurrences listed above); rename the three local `cached_file` bindings → `state_file`; rename the field `cache` → `state` (field declaration line 34, local at line 43, and the `self.cache` uses — whole-word only, so "cached" text is never touched); change the log string at line 252 to `"{} state file shutdown complete"`.
3. In `agent/src/filesys/mod.rs`: replace `pub mod cached_file;` with `pub mod state_file;` and move it to keep the module list alphabetical (after `pub mod path;`).
4. In `agent/src/authn/token_mngr.rs`: update the import to `state_file::SingleThreadStateFile` and the alias to `pub type TokenFile = SingleThreadStateFile<Token, token::Updates>;`.
5. In `agent/src/disk/device.rs`: update the import to `state_file::ConcurrentStateFile` and the alias to `pub type Device = ConcurrentStateFile<models::Device, device::Updates>;`.
6. In `agent/tests/filesys/mod.rs`: replace `pub mod cached_file;` with `pub mod state_file;`, re-sorted after `pub mod path;`.
7. In `agent/tests/filesys/state_file.rs`: update the line-4 import path and both type names; rename both type-name occurrences in the alias definitions; rename all local `cached_file` bindings → `state_file`; update the two banner comments to `SINGLE THREADED STATE FILE` / `MULTI THREADED STATE FILE`. Leave the two "cached in memory" comments as-is (see Decision Log).
8. Run `cargo fmt -p miru-agent` so rustfmt settles brace-group ordering in the touched `use` statements.
9. Commit: `refactor: rename CachedFile to StateFile` (Conventional Commits, imperative, lower-case description, under 72 chars).

Milestone 2 — validation: run the verification grep, build, full test suite, lint; push; watch CI to green (details in Validation and Acceptance). Produces no commit unless fixes are needed.

## Concrete Steps

All commands run from the repo root, on branch `claude/scanner-state-persistence-4gbd9c`.

Step 1 — file moves:

    git mv agent/src/filesys/cached_file.rs agent/src/filesys/state_file.rs
    git mv agent/tests/filesys/cached_file.rs agent/tests/filesys/state_file.rs

Step 2 — mechanical replacements (then hand-verify with `git diff`):

    sed -i \
      -e 's/SingleThreadCachedFile/SingleThreadStateFile/g' \
      -e 's/ConcurrentCachedFile/ConcurrentStateFile/g' \
      -e 's/\bcached_file\b/state_file/g' \
      -e 's/CACHED FILE/STATE FILE/g' \
      -e 's/cached file/state file/g' \
      agent/src/filesys/state_file.rs \
      agent/src/filesys/mod.rs \
      agent/src/authn/token_mngr.rs \
      agent/src/disk/device.rs \
      agent/tests/filesys/state_file.rs \
      agent/tests/filesys/mod.rs

    # field rename, src module only (word-boundary so "cached" is never matched)
    sed -i 's/\bcache\b/state/g' agent/src/filesys/state_file.rs

Step 3 — restore alphabetical module ordering by hand in `agent/src/filesys/mod.rs` and `agent/tests/filesys/mod.rs` (move the `pub mod state_file;` line after `pub mod path;`), then:

    cargo fmt -p miru-agent

Step 4 — verification grep (must print nothing and exit non-zero):

    grep -rni "cachedfile\|cached_file\|cached file" agent/ libs/

Expected: zero hits. (`plans/` history docs live outside `agent/` and `libs/` and are intentionally excluded.)

Step 5 — build and test:

    cargo build -p miru-agent
    ./scripts/test.sh

Expected: build clean; test suite passes with the same test count as `main` (this is a pure rename — no tests added, removed, or modified in behavior). `./scripts/test.sh` wraps `RUST_LOG=off cargo test --features test`; the `--features test` flag is mandatory, never invoke `cargo test` bare.

Step 6 — lint:

    ./scripts/update-deps.sh   # refresh Cargo.lock first, per AGENTS.md
    ./scripts/lint.sh

Expected: import linter, `cargo fmt --check`, machete/diet, audit, and clippy all clean. (Optionally `./scripts/preflight.sh` for the full local gate.)

Step 7 — commit (end of Milestone 1):

    git add -A
    git commit -m "refactor: rename CachedFile to StateFile"

Step 8 — push and watch CI (Milestone 2):

    git push origin claude/scanner-state-persistence-4gbd9c

Then watch the `CI` workflow (jobs: `lint`, `test`, `tools`) on the pushed head. Note: the `gh` CLI is unavailable in this environment — use the GitHub MCP tools (`mcp__github__actions_list`, `mcp__github__actions_get`, `mcp__github__get_job_logs`, `mcp__github__pull_request_read`) against `mirurobotics/agent`.

## Validation and Acceptance

Acceptance is all of the following, in order:

1. `grep -rni "cachedfile\|cached_file\|cached file" agent/ libs/` returns zero hits.
2. `cargo build -p miru-agent` succeeds.
3. `./scripts/test.sh` passes with zero failures and an unchanged test count relative to `main`. The full suite is the regression gate — no new tests are needed for a pure rename; any failure means the rename broke something and must be fixed before proceeding.
4. `./scripts/lint.sh` is clean.
5. `git diff main --stat` shows insertions equal to deletions (a 1-for-1 token rename adds no lines), and the two file moves are recorded as renames.
6. Preflight reports CLEAN: the `CI` workflow (lint, test, tools jobs) is green on the pushed head of `claude/scanner-state-persistence-4gbd9c`. The PR must not leave draft, and the task must not be reported complete, until CI is green on that head. CI status is checked via the GitHub MCP tools (see Step 8) since `gh` is unavailable.

## Idempotence and Recovery

Every step is safe to re-run: `git mv` on an already-moved file fails harmlessly; the sed replacements are no-ops once applied; fmt, grep, build, test, and lint are read-only or idempotent. If a sed pass over-matches (verify with `git diff` before committing), recover with `git checkout -- <file>` (or `git reset --hard main` before anything is committed) and redo that step by hand. The branch exists only for this work, so `git reset --hard dd4940d` is always a safe full rollback prior to push; after push, force-push the reset branch if a restart is needed (no one else consumes this branch).
