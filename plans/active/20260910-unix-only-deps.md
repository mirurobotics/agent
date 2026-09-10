# Unix-only dependency hygiene: target-gate `nix`, remove unused `users`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.


## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench5/repos/agent) | read/write | Rust workspace for the Miru device agent. Both manifest edits, all validation, and the commit happen here. |

The plan lives at `plans/backlog/20260910-unix-only-deps.md` in this repo because the repo owns the change; when implementation starts it moves to `plans/active/` and travels in this PR via its own `docs(plans):` commit (step 1). Branch: `build/unix-only-deps` (base `main`, currently at 84f8463).


## Purpose / Big Picture

This is PR 1 ("dependency hygiene") of the Windows-support roadmap in `plans/active/20260910-windows-support.md` (section "PR roadmap", line 64; PR 1 at lines 68–70). Two Cargo metadata changes, no behavior change:

1. Declare `nix` (a Unix-syscall wrapper crate) as a target-specific dependency of the `miru-agent` crate so it is only a dependency when compiling for Unix targets.
2. Remove the `users` workspace dependency, which nothing consumes.

After this PR, Linux builds and tests are byte-for-byte equivalent in behavior: Linux matches `cfg(unix)`, so `nix` still resolves, compiles, and links exactly as before. The payoff is groundwork: a future Windows `cargo check` (added in PR 3) will not try to build `nix`. Explicitly out of scope: openssl/TLS work (PR 2), any `#[cfg(unix)]` gating of source code, the privilege module, or CI target checks (PR 3), and the dead `config-agent` entry in the root manifest. The `privilege` module remains unconditionally compiled.


## Progress

- [ ] Activate plan (move to `plans/active/`, `docs(plans):` commit); confirm branch and clean tree
- [ ] Root `Cargo.toml`: remove `users = "0.11.0"` from `[workspace.dependencies]`
- [ ] `agent/Cargo.toml`: move `nix` from `[dependencies]` to new `[target.'cfg(unix)'.dependencies]` table
- [ ] Run `./scripts/update-deps.sh`; confirm no Cargo.lock drift from this change
- [ ] Run `./scripts/lint.sh` — clean
- [ ] Run `./scripts/test.sh` — all tests pass, including `agent/tests/privilege/mod.rs`
- [ ] Commit the two manifest edits (Conventional Commits)
- [ ] Push; preflight CLEAN (CI green on the pushed head) before the PR leaves draft


## Surprises & Discoveries

(Add entries as work proceeds.)


## Decision Log

- 2026-09-10 (authoring): `users` is verified unused and safe to remove. Evidence: its only declaration is root `Cargo.toml:75` (no member crate's manifest references it); zero `use users` / `users::` / `extern crate users` hits in any `.rs` file; and `grep 'name = "users"' Cargo.lock` matches nothing — cargo never resolved it because no member consumes it. Removal is therefore a manifest-only diff with no lockfile impact.

(Add further entries as work proceeds.)


## Outcomes & Retrospective

(Fill in when the work is complete.)


## Context and Orientation

Terms: a *workspace dependency* is declared once under `[workspace.dependencies]` in the root manifest and inherited by member crates via `{ workspace = true }`. A *target-specific dependency table* (`[target.'cfg(unix)'.dependencies]`) makes a dependency apply only when the compilation target matches the cfg expression; Linux matches `cfg(unix)`. Target-specific dependencies are also available to that target's tests, so the privilege integration test keeps compiling on Linux even though `nix` is a normal (non-dev) dependency.

Files:

- Root manifest `Cargo.toml`: members are `agent`, `libs/device-api`, `libs/backend-api`. Sections: `[workspace]` (line 1), `[workspace.package]` (9–13), `[workspace.metadata]` (15–16), `[workspace.dependencies]` (18–76), `[profile.release]` (78–80). Line 42 `nix = { version = "0.31.2", default-features = false, features = ["user"] }` **stays** (it defines version/features). Line 75 `users = "0.11.0"` is **removed**. No `[target.*]` table exists anywhere in the workspace today.
- `agent/Cargo.toml`: `[package]` (1–7, has `build = "build.rs"`), `[features]` `test = []` (9–10), `[lints.clippy]` (12–13), `[dependencies]` (15–47) with `nix = { workspace = true }` at line 27, `[dev-dependencies]` (49–64). The new target table goes between `[dependencies]` and `[dev-dependencies]`.
- `nix` consumers (all Unix-only in nature, all still compile on Linux): `agent/src/privilege/mod.rs` (lines 7–8 import `nix::errno::Errno` and `nix::unistd::{Gid, Uid, User}`; `geteuid`/`getegid` at 16–17), `agent/src/privilege/errors.rs` (line 5), and integration test `agent/tests/privilege/mod.rs` (lines 29, 49–50, 95). The module is wired unconditionally at `agent/src/lib.rs:20` (`pub mod privilege;`) and stays that way in this PR. No other member manifest declares `nix`.
- `Cargo.lock`: `nix` is present (0.31.3 resolved from ^0.31.2) and stays — moving the dep does not change resolution on Linux. `users` is absent and stays absent. Expected lockfile diff: none. `agent/build.rs` uses only `std::process::Command` — unaffected.

Tooling (all run from repo root /home/ben/miru/workbench5/repos/agent):

- `./scripts/update-deps.sh` — refreshes Cargo.lock; AGENTS.md requires running it before linting.
- `./scripts/lint.sh` — custom import/funclen/assert linter (`cargo run --manifest-path tools/lint/Cargo.toml`), `cargo fmt --package miru-agent`, `cargo machete`, `cargo diet`, `cargo audit`, `cargo clippy --package miru-agent --no-deps --all-targets --all-features -- -D warnings`. Locally it defaults to fix-mode (`LINT_FIX=1`: fmt rewrites files, clippy adds a `--fix --allow-dirty` pass); CI runs check-only via `LINT_FIX=0`. Pass = exit 0.
- `./scripts/test.sh` — `RUST_LOG=off cargo test --package miru-agent --features test`. The `--features test` flag is mandatory (test helpers are behind `#[cfg(feature = "test")]`).
- `./scripts/preflight.sh` — lint + covgate + tools lint + tools covgate in parallel; prints "Preflight clean" on success.
- CI: `.github/workflows/ci.yml`, workflow name `CI`, triggers on push/PR to `main` and `release/*`. Jobs: `lint` (runs `LINT_FIX=0 ./scripts/lint.sh`), `test` (cargo-llvm-cov via `./scripts/covgate.sh`), `tools` (tools/lint's own lint+covgate). This is the only PR-gating workflow.

Known non-risk: `cargo machete` is green today even with `users` declared (it never flagged the workspace-level entry), so this PR fixes latent hygiene rather than a red gate; the `nix` move must not introduce a new machete finding — `privilege` keeps it "used" on Unix.


## Plan of Work

Single milestone. Edit the two manifests, refresh the lockfile (expecting no change), run lint and tests as the regression net, and commit. No new tests: this is a metadata-only change, and the existing suite is the coverage that proves the moved dependency still compiles and links on Linux.


## Concrete Steps

All commands run from /home/ben/miru/workbench5/repos/agent. The commit step happens during implementation, not at authoring time.

1. Activate the plan and confirm starting state — the only untracked entry is this plan (`?? plans/backlog/`); commit it so it travels in the PR:

       git branch --show-current   # expect: build/unix-only-deps
       mv plans/backlog/20260910-unix-only-deps.md plans/active/
       git add plans/active/20260910-unix-only-deps.md
       git commit -m "docs(plans): activate unix-only deps plan"
       git status --short          # expect: empty (clean tree)

2. In root `Cargo.toml`, delete line 75 inside `[workspace.dependencies]`:

       users = "0.11.0"

   Leave line 42 (`nix = ...`) untouched.

3. In `agent/Cargo.toml`, delete line 27 (`nix = { workspace = true }`) from `[dependencies]`, then add a new table after `[dependencies]` ends (line 47) and before `[dev-dependencies]` (line 49):

       [target.'cfg(unix)'.dependencies]
       nix = { workspace = true }

4. Refresh the lockfile, then check for drift:

       ./scripts/update-deps.sh
       git diff --stat Cargo.lock   # expect: no output

   Any Cargo.lock churn is unrelated dependency drift, not caused by this change — leave it out of the PR (`git restore Cargo.lock`).

5. Lint:

       ./scripts/lint.sh            # expect: exit 0; fmt/machete/diet/audit/clippy all clean
       git status --short           # expect: only the two manifests (plus this plan, if updated) — fix-mode must not have touched anything else

6. Test:

       ./scripts/test.sh            # expect: every suite ends "test result: ok. ... 0 failed"

7. Commit the milestone (Conventional Commits; only the two manifests should be staged):

       git add Cargo.toml agent/Cargo.toml
       git diff --cached --stat     # expect: exactly 2 files changed
       git commit -m "build(deps): make nix unix-only, drop unused users workspace dep"

8. Commit any plan updates (`docs(plans):`), push, and open the draft PR — CI runs on `pull_request` only for this branch, so the draft PR is what triggers the run:

       git status --short          # expect: empty, or only this plan modified
       git add plans/active/20260910-unix-only-deps.md
       git commit -m "docs(plans): record unix-only deps progress"   # skip if the tree was clean
       git push -u origin build/unix-only-deps   # expect: new remote branch, upstream set
       gh pr create --draft --base main --fill   # expect: prints the draft PR URL


## Validation and Acceptance

Acceptance is "no observable change on Linux, cleaner manifests":

- `git diff main --stat` shows only `Cargo.toml`, `agent/Cargo.toml`, and this plan under `plans/active/`. No Cargo.lock diff.
- `cargo tree --package miru-agent | grep nix` still lists `nix v0.31.3` (resolution unchanged because Linux matches `cfg(unix)`).
- `./scripts/lint.sh` exits 0 — in particular `cargo machete` reports no unused dependencies and clippy `--all-targets --all-features -D warnings` is clean.
- `./scripts/test.sh` passes with zero failures. The existing suite is the regression net for this metadata-only change; `agent/tests/privilege/mod.rs` is the direct nix-consumer coverage that must still compile and pass.
- Preflight must report CLEAN — meaning the `CI` workflow (lint, test, tools jobs) is green on the pushed head of `build/unix-only-deps` — before the PR leaves draft or the task is reported complete. Locally `./scripts/preflight.sh` prints "Preflight clean"; on GitHub verify with `gh run list --branch build/unix-only-deps` or `gh pr checks`.


## Idempotence and Recovery

Both edits are exact-text deletions/insertions: re-applying them to an already-edited file is a no-op (the text to delete is gone; do not add the target table twice). All scripts are safe to re-run any number of times. If a step fails midway, `git status` and `git diff` show exactly what changed; `git restore Cargo.toml agent/Cargo.toml` (or `git restore Cargo.lock` for unrelated lock churn) returns to the clean baseline, after which the steps can be repeated from step 2. Nothing here is destructive or migratory — the only persistent side effects are the local commits, undoable pre-push with `git reset --soft`.
