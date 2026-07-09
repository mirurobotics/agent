# Track root `Cargo.lock` and pin `crossbeam-epoch` (fix RUSTSEC-2026-0204)

**Status**: active
**Branch**: build/commit-cargo-lock (off latest origin/main)
**Date**: 20260708

## Goal

Start version-tracking the **root workspace** `Cargo.lock` so the shipped
`miru-agent` binary has a reproducible dependency graph, and so the freshly
published advisory **RUSTSEC-2026-0204** (`crossbeam-epoch` invalid pointer
deref in the `fmt::Pointer` impl) is durably pinned to the patched
`>= 0.9.20`.

Context / why this is the right vehicle:
- `Cargo.lock` is currently gitignored (`.gitignore:8`, pattern `Cargo.lock`,
  which matches every workspace). The `.gitignore` comment itself says: *"Remove
  Cargo.lock from gitignore if creating an executable, leave it for libraries."*
  `miru-agent` is an executable, so tracking its lock is the intended policy.
- Because the lock is untracked, CI resolves fresh each run and already picks
  `crossbeam-epoch 0.9.20` (verified: `cargo update -p crossbeam-epoch --dry-run`
  → `v0.9.18 -> v0.9.20`; `0.9.20` is the latest `0.9.x` and nothing caps it).
  PR #120's `lint` (audit) check is green. The advisory only appears in *local*
  preflight because a stale on-disk lock pins `0.9.18`.
- Committing a fresh lock makes that patched resolution durable and identical
  for every developer and CI run — no more stale-lock false positives.

## Scope

Two tracked files change; no source code changes.

### 1. `.gitignore`
Anchor the ignore to the root workspace only, so `./Cargo.lock` becomes tracked
while the separate dev-tool workspace lock (`tools/lint/Cargo.lock`) stays
ignored:
- Change line 8 from `Cargo.lock` to `/Cargo.lock`.
- Update the adjacent comment (lines 6-7) to reflect the decision (agent is an
  executable → its lock is tracked; the leading `/` keeps it scoped to the root
  workspace).

### 2. `Cargo.lock` (root workspace, `/home/ben/miru/workbench1/repos/agent/Cargo.lock`)
- Refresh to a current, fully-resolved lock that mirrors what CI resolves today
  (`cargo update` — latest semver-compatible across the graph), then `git add`
  the file (newly tracked).
- Requirement: the committed lock MUST contain `crossbeam-epoch` at `>= 0.9.20`
  and `cargo audit` MUST report 0 vulnerabilities.

## Explicitly OUT of scope
- `tools/lint/Cargo.lock` — separate dev-tool workspace, no advisory in its
  graph (clap/toml/walkdir/serde/syn/proc-macro2). Leave it ignored. A follow-up
  can track it too if the team wants full consistency.
- Any source / test code. No `.covgate` changes (coverage is unaffected by a
  lockfile).
- The two pre-existing **allowed** audit warnings (`paste` unmaintained,
  `num-bigint` yanked) — `lint.sh` already allow-lists these; do not touch them.

## Test steps

Run from the repo root (`/home/ben/miru/workbench1/repos/agent`):
1. Refresh + confirm the patched crate:
   ```bash
   cargo update
   grep -A1 'name = "crossbeam-epoch"' Cargo.lock   # expect version = "0.9.20" (or higher 0.9.x)
   ```
2. Audit is clean (this is the exact gate CI's `lint` job runs):
   ```bash
   LINT_FIX=0 ./scripts/lint.sh
   ```
   Expect the `cargo audit` step to report `0 vulnerabilities` (the 2 allowed
   warnings may remain).
3. Workspace still builds and the affected tests still pass with the refreshed
   lock:
   ```bash
   cargo test --package miru-agent --features test --no-run
   RUST_LOG=off cargo test --package miru-agent --features test -- deploy::filesys app::upgrade
   ```

## Validation

- `./scripts/preflight.sh` MUST print `Preflight clean` (exit 0) — lint
  (including `cargo audit` now reporting no vulnerabilities), all tests, and all
  coverage gates passing — before publishing.
- Confirm `git status` shows exactly two changed/added paths: `.gitignore`
  (modified) and `Cargo.lock` (new/tracked). No source files, no
  `tools/lint/Cargo.lock`.

## Git / publishing constraints
- All commits from within `/home/ben/miru/workbench1/repos/agent` (the agent
  repo's own git context). Branch `build/commit-cargo-lock` off latest
  `origin/main`. Do not commit to `main`.

## Risk / gotchas
- The root `Cargo.lock` is ~94k lines; committing it is a large *addition* but
  not a diff (new file), so review load is low — reviewers check the
  `crossbeam-epoch` version and that no source changed.
- Do NOT accidentally track `tools/lint/Cargo.lock`; the anchored `/Cargo.lock`
  pattern prevents this — verify with `git status` after `git add`.
- `cargo update` may bump other transitive crates to latest-compatible; that is
  the same resolution CI already uses (and is green), so it is expected and
  safe. Verify with the build + test run above.
