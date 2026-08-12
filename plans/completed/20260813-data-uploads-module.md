# Restructure — move scan/ and upload/ under a data_uploads/ parent module

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | All changes land here (crate `miru-agent` under agent/) |

Base branch `main` (HEAD e07b6f5); working branch `refactor/data-uploads-module` (already created, clean, == main). No backend, spec, or generated-code (libs/) changes. No behavior changes of any kind — this is a pure `git mv` + path-rewrite, provably behavior-preserving.

## Purpose / Big Picture

The data-uploads feature currently spans two top-level crate modules, `crate::scan` (watch directories, detect stable files) and `crate::upload` (queue and execute uploads), connected by the `StableFileSink` seam (scan/sink.rs defines the trait, upload/sink.rs implements it). A third sibling is inbound: the pending delete-module PR (#201) will be rebased into `data_uploads::retention` by a future PR. This restructure gives the feature one parent module so the sibling has a natural home:

- `agent/src/scan/` → `agent/src/data_uploads/scan/` (`crate::scan` → `crate::data_uploads::scan`)
- `agent/src/upload/` → `agent/src/data_uploads/upload/` (`crate::upload` → `crate::data_uploads::upload`)

**No compatibility re-exports.** There is no `pub use data_uploads::scan as scan;` (or similar) at the crate root — this is a real move, and every call site updates. The owner's intent is structural clarity, not aliasing (decided by the repo owner — do not relitigate).

Verification of behavior preservation: `./scripts/test.sh` passes with test counts EQUAL to main's, and every `.covgate` gate passes with thresholds carried over unchanged.

## Progress

- [x] M0: Record baselines on main HEAD (test counts, covgate output)
- [x] M1: `git mv` both src dirs; new `data_uploads/mod.rs`; lib.rs swap; path rewrite across agent/src
- [x] M2: Tests mirror — `git mv agent/tests/upload` under `agent/tests/data_uploads/`; path rewrite across agent/tests
- [x] M3: Docs — ARCHITECTURE.md codemap entry for `data_uploads/`
- [x] M4: Validation — test counts vs baseline, covgate, lint, fmt, clippy, no-features check; preflight CLEAN; push; CI green; draft PR

## Surprises & Discoveries

(Discovered during planning; executor appends its own here.)

- **`agent/tests/scan/` does not exist on main.** All scan tests are in-source (`#[cfg(test)]` in agent/src/scan/scanner.rs etc. — see plans/completed/20260812-stable-file-sinks.md M3, which reworked them in-source). Only `agent/tests/upload/` exists and moves. The tests mirror therefore gains `agent/tests/data_uploads/upload/` but no `scan/` — the mirror stays exact w.r.t. what exists.
- **ARCHITECTURE.md's codemap never mentions scan or upload at all** (`grep -ci "scan\|upload" ARCHITECTURE.md` → 0; the codemap at ARCHITECTURE.md:16 predates the feature — it also lacks disk/, gcs/, s3/). There are no stale path references to fix; instead M3 adds one small codemap entry for the new parent module.
- **AGENTS.md:11 says lib.rs lists "all 22 public modules" — already stale on main** (29 today, 28 after this change). Left alone; not this PR's drift to fix.
- **(executor) The import linter's normalizer enforces same-root merging, not just sorting**: after the mechanical rewrite it emitted `split-internal-imports` ("merge `crate::data_uploads` imports into a single grouped use") for app/state.rs, data_uploads/upload/sink.rs, and server/errors.rs — the two anticipated `use crate::data_uploads::scan...` / `use crate::data_uploads::upload...` lines in each file had to merge into one nested grouped `use crate::data_uploads::{scan..., upload...};`. The plan's "may merge if the linter prefers it" hedge for server/errors.rs turned out to be mandatory, and applied to all three files. Short names `scan`/`upload` stay bound, so interior qualified usages still needed no edits.
- **(executor) Within-group sort ordering is delegated to `cargo fmt`** (tools/lint/src/checker/mod.rs comment: "Sorting within groups is left to `cargo fmt` (reorder_imports)"), so no manual re-sorting was needed — `cargo fmt -p miru-agent` repositioned every rewritten `use` line.
- **(executor) `cargo clippy --package miru-agent --all-features -- -D warnings` fails on main and on this branch alike** with two `manual_map` errors in generated `libs/backend-api` code (verified by running it on a clean main checkout). AGENTS.md:96 documents that clippy warnings in generated code are expected; `scripts/lint.sh` (which scopes clippy correctly) and `scripts/preflight.sh` both pass clean. Not this PR's issue.

## Decision Log

- **(owner) One parent module, real move, no re-exports**: `crate::data_uploads::{scan, upload}`; every call site updates; the parent must be a natural home for the future `data_uploads::retention` sibling (PR #201 rebase, out of scope here).
- **Rewrite absolute paths mechanically, keep short names bound (planner)**: every occurrence of `crate::scan` → `crate::data_uploads::scan`, `crate::upload` → `crate::data_uploads::upload`, `miru_agent::scan` → `miru_agent::data_uploads::scan`, `miru_agent::upload` → `miru_agent::data_uploads::upload` — including self-references inside the moved dirs (e.g. agent/src/scan/scanner.rs:7 `pub use crate::scan::state::...`), which are absolute crate paths and equally stale after the move. Files that import the module itself (`use crate::scan;` at app/run.rs:15 and server/errors.rs:10, `use crate::scan::{self, ...}` at app/state.rs:14) keep the short name `scan`/`upload` bound after the rewrite (`use crate::data_uploads::scan;` still binds `scan`), so the many qualified interior usages (15 in app/state.rs, 4 in app/run.rs, `scan::ScanErr`/`upload::UploadErr` at server/errors.rs:88,92,151-152,163-164) need no edits. This keeps the diff at ~52 reference lines + structural files.
- **Import-group re-sorting is required (planner)**: the custom import linter enforces alphabetical order within each comment-headed group via a full-path sort key (tools/lint/src/parser/mod.rs:10-11, checker/mod.rs:47). Rewritten `use` lines change sort position: `crate::data_uploads::...` sorts between `crate::crypt`/`crate::cooldown` and `crate::deploy`/`crate::disk`, so e.g. app/state.rs's line 14 moves up from its `crate::scan` slot, and the former `crate::upload` line at state.rs:17 moves adjacent to it. Every touched file's internal-crates group must be re-sorted; the linter is the arbiter (`scripts/lint.sh` runs it).
- **`.lint-imports.toml` needs no change (planner)**: it names only internal crate roots (`backend_api`, `device_api`, `miru_agent`) and the three group-label comments — no module paths (.lint-imports.toml:1-6).
- **No parent `.covgate` for `data_uploads/` (planner)**: `scripts/lib/covgate.sh` discovers modules purely by `find "$SRC_DIR" -name '.covgate'` (line 67) and matches files by directory-prefix `startswith` (line 84) — the moved gates (`scan/.covgate` 98.83, `upload/.covgate` 96.00) travel with `git mv` and keep gating their subtrees at the new nested paths; nested gates are precedented (services/ has a parent gate plus five nested ones). `data_uploads/mod.rs` will contain only `pub mod` declarations — no coverage regions — so a parent gate would be either vacuous or redundant with the children. When `retention` lands it brings its own `data_uploads/retention/.covgate` (per the AGENTS.md new-module checklist). If a reviewer wants a parent aggregate gate later, `scripts/update-covgates.sh` can ratchet one in — not this PR.
- **(owner) workers/scan.rs and workers/sync_scan_bridge.rs STAY at agent/src/workers/**: they are drivers of the scanner actor, not members of the data-uploads feature. Only their `use` paths (and the doc comment at workers/scan.rs:1 that names `crate::scan`) update. Do not move or rename them.
- **Out of scope (owner)**: renaming anything else; touching `delete/`/`retention` (does not exist on main); crate-root re-exports; any behavior change; fixing AGENTS.md's stale module count; back-filling the rest of ARCHITECTURE.md's stale codemap. A diff under libs/ or api/specs/ is a mistake — revert it.

## Context and Orientation

All paths relative to /home/ben/miru/workbench4/repos/agent. Read AGENTS.md first: three-group comment-headed imports (custom linter, alphabetical within group), tests mirror src layout, per-directory `.covgate` files, `./scripts/test.sh` (never bare `cargo test` — `--features test` is required), fmt/clippy scoped `--package miru-agent`.

### What moves

- `agent/src/scan/` — 6 files: errors.rs, mod.rs, rule.rs, scanner.rs, sink.rs, state.rs, plus `.covgate` (98.83). `mod.rs` declares `pub mod errors; pub(crate) mod rule; pub mod scanner; pub mod sink; pub(crate) mod state;` with re-exports — content unchanged by the move (`pub(crate)` visibility is crate-wide, unaffected by nesting).
- `agent/src/upload/` — 8 files: errors.rs, executor.rs, job.rs, mod.rs, queue.rs, sink.rs, transfer.rs, uploader.rs, plus `.covgate` (96.00). `mod.rs` content unchanged.
- `agent/tests/upload/` — 6 files: executor.rs, mod.rs, queue.rs, sink.rs, transfer.rs, uploader.rs.

### Structural files that change

- `agent/src/lib.rs`: drop `pub mod scan;` (line 22) and `pub mod upload;` (line 27); add `pub mod data_uploads;` in alphabetical position (between `pub mod crypt;` and `pub mod deploy;`). 29 modules → 28.
- NEW `agent/src/data_uploads/mod.rs`: exactly `pub mod scan;` + `pub mod upload;` (alphabetical; `retention` slots in later). No re-exports at this level — call sites use full paths, matching the no-aliasing decision.
- `agent/tests/mod.rs`: drop `pub mod upload;`; add `pub mod data_uploads;` (between `crypt` and `deploy`).
- NEW `agent/tests/data_uploads/mod.rs`: exactly `pub mod upload;` (no scan/ — see Surprises).

### Call-site sweep (measured on main HEAD e07b6f5)

`grep -rn "crate::scan\|miru_agent::scan" agent/src agent/tests` → 32 lines; `grep -rn "crate::upload\|miru_agent::upload"` → 20 lines. **52 reference lines across 25 files**, the expected path-rewrite diff size:

Inside the moved dirs (self-references, 19 lines): scan/scanner.rs (11), scan/rule.rs (2), scan/sink.rs (1), upload/sink.rs (2 — one `crate::scan::{scanner::StableFile, sink::StableFileSink}`, one `crate::upload`), upload/executor.rs (1), upload/queue.rs (1), upload/transfer.rs (1), upload/uploader.rs (1).

agent/src consumers (10 lines): app/state.rs (2: lines 14, 17), app/run.rs (1: line 15), server/errors.rs (2: lines 10, 13), workers/scan.rs (2: doc comment line 1 + `use` line 11), workers/sync_scan_bridge.rs (2: lines 8, 105).

agent/tests (23 lines): upload/uploader.rs (2), upload/executor.rs (2), upload/transfer.rs (2), upload/queue.rs (1), upload/sink.rs (2 — imports both `miru_agent::scan` and `miru_agent::upload`), workers/scan.rs (2), workers/sync_scan_bridge.rs (3: lines 11, 482, 509 — the latter two are fully-qualified expression paths, not `use` lines), server/errors.rs (2), app/state.rs (2), mocks/scanner.rs (1), mocks/upload_executor.rs (2), mocks/object_transfer.rs (2).

No other reference kinds exist: no string-literal module names, no log-filter/tracing-target configs naming `scan`/`upload` module paths, no path references in scripts/ or .github/workflows/ (`grep -rn "src/scan\|src/upload\|tests/scan\|tests/upload" .github scripts tools` → empty). Tracing log targets derive from module paths and will change (`miru_agent::scan::scanner` → `miru_agent::data_uploads::scan::scanner`) — nothing in the repo filters on the old targets.

## Plan of Work

### M0 — Baselines (on the clean branch, == main)

1. Run `./scripts/test.sh 2>&1 | grep "test result"` and record every per-binary `X passed; Y failed; Z ignored` line — this is the equality target for M4. (Known flake: `deploy::fsm::tests::next_action_fn::deployed_activity` has a wall-clock drift flake — see 20260812-stable-file-sinks.md; rerun if it trips.)
2. Run `./scripts/covgate.sh` and record the per-module pass lines for scan and upload.

### M1 — src move + rewrite

3. `mkdir agent/src/data_uploads && git mv agent/src/scan agent/src/data_uploads/scan && git mv agent/src/upload agent/src/data_uploads/upload` (`.covgate` files travel with the moves — verify with `git status`).
4. Create `agent/src/data_uploads/mod.rs` (two `pub mod` lines, per Context).
5. Edit `agent/src/lib.rs` per Context.
6. Path rewrite across agent/src (the 29 src lines from the sweep): `crate::scan` → `crate::data_uploads::scan`, `crate::upload` → `crate::data_uploads::upload`. A `grep -rl ... | xargs sed -i` over agent/src is safe — the tokens are unambiguous (no `crate::scanner`-style near-misses exist).
7. Re-sort the internal-crates import group in every touched file (app/state.rs, app/run.rs, server/errors.rs, workers/scan.rs, workers/sync_scan_bridge.rs, and the moved files whose `use crate::...` lines changed position — notably upload/sink.rs where `crate::data_uploads::scan...` and `crate::data_uploads::upload...` now sort adjacently). In server/errors.rs the two module imports may merge to `use crate::data_uploads::{scan, upload};` if the linter prefers it; otherwise keep two sorted lines.
8. Gate: `cargo check --package miru-agent` (NO features — recent regression source) AND `cargo check --package miru-agent --features test` both clean.

### M2 — tests mirror + rewrite

9. `mkdir agent/tests/data_uploads && git mv agent/tests/upload agent/tests/data_uploads/upload`.
10. Create `agent/tests/data_uploads/mod.rs` (`pub mod upload;`); edit `agent/tests/mod.rs` per Context.
11. Path rewrite across agent/tests (23 lines): `miru_agent::scan` → `miru_agent::data_uploads::scan`, `miru_agent::upload` → `miru_agent::data_uploads::upload`; re-sort import groups in touched test files.
12. Gate: `./scripts/test.sh` — all pass; compare `test result` lines against the M0 baseline: **counts must be EQUAL** (same passed/failed/ignored per binary; test names may re-sort since Rust test names embed module paths, e.g. in-source scanner tests become `data_uploads::scan::scanner::tests::...`).

### M3 — docs

13. ARCHITECTURE.md: add a short codemap entry (under "Business logic", after `deploy`) for `data_uploads/` — parent module for the file-upload pipeline; submodules `scan` (watches rule-configured directories, emits stable files to sinks) and `upload` (queues and executes uploads to object storage); note that drivers live in `workers/` (scan.rs, sync_scan_bridge.rs). Three or four sentences; do not back-fill the rest of the stale codemap.

### M4 — validation, push, CI, PR

14. Full local gate battery (see Validation). `./scripts/update-deps.sh` first only if Cargo.lock is stale; this change should not touch Cargo.lock or Cargo.toml at all.
15. `./scripts/preflight.sh` → must print CLEAN.
16. Commit from within this repo's git context. One commit is honest for a pure restructure: `refactor(agent)!: move scan/ and upload/ under a data_uploads/ parent module` (breaking `!` because `miru_agent::scan`/`miru_agent::upload` public paths move). Verify the commit shows renames (`git show --stat` lists `{scan => data_uploads/scan}/...`), proving git tracked the moves. Signed commits (commit.gpgsign is on — never disable).
17. Push `refactor/data-uploads-module`; open a DRAFT PR onto main (`gh pr create` with full body — `gh pr edit` is broken on mirurobotics repos, use `gh api PATCH` if edits are needed); watch CI on the branch head.

## Validation and Acceptance

Behavioral acceptance criteria:

1. **Pure restructure**: `./scripts/test.sh` green with per-binary test counts EQUAL to the M0 baseline from main — zero tests added, removed, or newly ignored.
2. **Covgate carried over**: `./scripts/covgate.sh` — every module passes; scan and upload appear as `data_uploads/scan` (requires 98.83) and `data_uploads/upload` (requires 96.00) with no threshold edits anywhere in the diff (`git diff main --stat -- '**/.covgate'` shows only renames).
3. **No stale paths**: grep proof —

       grep -rn "crate::scan\b\|crate::upload\b" agent/src agent/tests        # expect: no matches
       grep -rn "miru_agent::scan\b\|miru_agent::upload\b" agent/src agent/tests  # expect: no matches
       grep -rn "pub use" agent/src/lib.rs agent/src/data_uploads/mod.rs     # expect: no matches (no aliasing re-exports)

4. **Diff shape**: only renames under agent/src/{scan,upload} → agent/src/data_uploads/ and agent/tests/upload → agent/tests/data_uploads/upload, path/import-sort line edits in the 25 swept files, the four structural mod-listing files, and ARCHITECTURE.md. Nothing under libs/, api/, scripts/, .github/, Cargo.*.
5. **Retention-ready**: adding `data_uploads::retention` later is one `pub mod retention;` line in data_uploads/mod.rs plus its own directory — sanity-check by reading the final mod.rs, not by building anything.

Exact commands and expected results:

    ./scripts/test.sh                        # all pass; counts == M0 baseline
    ./scripts/covgate.sh                     # every gate met, thresholds unchanged
    scripts/lint.sh                          # import linter + fmt + machete/diet + audit + clippy clean
    cargo fmt -p miru-agent -- --check       # clean
    cargo clippy --package miru-agent --all-features -- -D warnings   # clean
    cargo check --package miru-agent         # NO features — must pass
    ./scripts/preflight.sh                   # reports CLEAN, exit 0

CI: after pushing, the workflow run on the branch head must conclude green. **The PR must not leave draft, and the task must not be reported complete, until preflight reports CLEAN and CI is green on the pushed branch head.**

## Idempotence and Recovery

- All edits are `git mv` + text edits on an existing branch; re-running a milestone is safe. Roll back with `git reset --hard main` (branch has no unique commits until M4).
- If a `sed` sweep misfires, `git checkout -- <paths>` restores; the moves themselves survive in the index.
- `cargo check` / `test.sh` / `covgate.sh` / `preflight.sh` are safe to repeat. Force-push is acceptable before review starts.
