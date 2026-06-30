# Sync vendored OpenAPI spec and regenerate Rust models from openapi main


## Scope

| Repo | Path | Access |
| --- | --- | --- |
| agent | /home/ben/miru/workbench4/repos/agent | read-write (branch `chore/sync-openapi-spec-models`) |
| openapi | /home/ben/miru/workbench4/repos/openapi | read-only (branch `main`, HEAD 316936e, clean) |

This plan file lives in the agent repo at `plans/backlog/20260630-sync-openapi-spec-models.md`.

Git note: the orchestrator owns branching. The agent repo is already on branch `chore/sync-openapi-spec-models`. Do NOT create or switch branches. Commit from inside the agent repo's own git context (cwd `/home/ben/miru/workbench4/repos/agent`), never from the workbench root.


## Purpose / Big Picture

The agent's vendored backend OpenAPI spec is stale (pinned to openapi commit 97809d8). Current openapi `main` (HEAD 316936e) has two schema deltas the agent has not picked up:

1. Removed the per-rule `poll_interval_secs` field from `UploadRuleSource` (openapi PR #166).
2. Added an optional `content` string property on `BaseUploadRule` (openapi PR #152).

After this task the agent's generated Rust models match current openapi `main`: `poll_interval_secs` is gone everywhere, the `content` field is present, the agent's hand-written source no longer references the removed field, and `cargo check`, `./scripts/test.sh`, and `./scripts/preflight.sh` all report clean. This is a focused delta re-vendor + regenerate + small source fixup. No new behavior is surfaced to users; the `content` field is left carried by the generated model only (not promoted into the agent's domain model).


## Progress

- [x] M1: Re-vendor backend spec from openapi main + regenerate models
- [ ] M2: Fix agent source/tests for removed `poll_interval_secs`
- [ ] M3: Validate (cargo check/test, lint, preflight clean)


## Surprises & Discoveries

- 2026-06-30 (M1): Strategy B (copy `build/dist/agent.yaml` wholesale) was attempted but rejected in favor of Strategy A (surgical edit). `build/build-release.sh` skips the agent target because HEAD (316936e) is not the `agent/v0.4.0` tag commit, so the snapshot build (`build/build-snapshot.sh`) was used instead. The snapshot artifact differs from the existing vendored `v04.yaml` in three ways that are NOT real spec changes: (a) a different YAML dumper style (list items at column 0 vs the vendored 2-space indent, plus long-description line-wrapping) yielding a ~1181-line noise diff in the body; (b) a version regression `v0.5.0-pre` -> `v0.4` (the version derives from the last git tag); (c) the verbose `x-git-commit.message` body, which itself contains the literal string `poll_interval_secs` and would have failed the acceptance grep over `api/specs/`. Both strategies are documented as converging on identical generated Rust models, so Strategy A was applied: removed `poll_interval_secs` from `UploadRuleSource` (required list, property def, and 3 example occurrences) and added the optional `content: string` property to `BaseUploadRule`, mirroring `apis/apps/backend-server/agent/openapi.gen.yaml` exactly (including its `content` description + example). `api/regen.sh` then modified only `upload_rule_source.rs` (dropped field, updated `new()` signature) and `base_upload_rule.rs` (added `content: Option<String>`); device models untouched, no files added/removed.


## Decision Log

- 2026-06-30 (author): Re-vendor via Strategy B (authoritative full rebuild of the spec from openapi `main` using its release build tool, then copy into the agent) as the primary path, because it keeps spec provenance correct and avoids hand-editing generated YAML. Strategy A (surgical edit of the vendored `v04.yaml`) is documented as a fallback for when the openapi build toolchain (Python/Node/Java) is unavailable. Both converge on the same two schema deltas and identical generated Rust models.


## Outcomes & Retrospective

(to be filled in at completion)


## Context and Orientation

Read this section assuming no prior knowledge of either repo.

Terms:

- "Vendored spec": a copy of an OpenAPI YAML file checked into the agent repo under `api/specs/`. The agent does not fetch specs at build time; a human/agent copies them in and regenerates code.
- "Regen" / "codegen": running the OpenAPI generator to turn a spec YAML into Rust source files (model structs). Output is copied into the `libs/backend-api` and `libs/device-api` crates.
- "openapi-generator-cli": a Java-based tool (run via `npx`) that reads a spec and emits Rust. Requires Node/npx and a Java JRE. The version is pinned to 7.12.0 in `api/openapitools.json`.

Agent repo tooling (all paths relative to `/home/ben/miru/workbench4/repos/agent`):

- `api/specs/backend/v04.yaml` — vendored backend spec. Currently pinned to openapi 97809d8; still contains `poll_interval_secs` (5 occurrences: a required-list entry around line 835, the property definition around lines 843-846, and three example values around lines 853, 958, 991) and lacks the `content` property on `BaseUploadRule`.
- `api/specs/device/v02.yaml` — vendored device spec ("Miru Agent API", a device-local Unix-socket API). No `poll_interval` and no upload schemas. Unaffected by this task.
- `api/Makefile` — `make gen` runs clean, then `gen-backend` and `gen-device`. Each target runs `npx --yes @openapitools/openapi-generator-cli generate -i <SPEC> -g rust -t templates/rust -o <CODEGEN_DIR> --additional-properties=packageName=<backend-api|device-api>`. Vars: `BACKEND_FILE := specs/backend/v04.yaml`, `DEVICE_FILE := specs/device/v02.yaml`, `BACKEND_CODEGEN_DIR := codegen/backend`, `DEVICE_CODEGEN_DIR := codegen/device`, `RUST_TEMPLATES_DIR := templates/rust`.
- `api/regen.sh` — resolves the git root, `cd`s into `api`, runs `make gen` (generates into `api/codegen/{backend,device}`), then for backend does `rm -rf libs/backend-api/src/models/*` followed by `cp -r api/codegen/backend/src/models/* libs/backend-api/src/models`, and the same for device into `libs/device-api/src/models`. It copies only the `src/models` subtree.
- `api/templates/rust/` — custom Rust templates used by the generator.
- `libs/backend-api/src/models/` — 50 generated files, including the upload set: `base_upload_rule.rs`, `upload_rule_source.rs`, `upload.rs`, `create_upload_request.rs`, etc. `upload_rule_source.rs` currently has `poll_interval_secs` (a `#[serde(rename = "poll_interval_secs")]` line, a `pub poll_interval_secs: i32` field, a `new(...)` arg, and a struct-init line). `base_upload_rule.rs` currently has no `content` field.
- `libs/device-api/src/models/` — 21 generated files, no upload models (correct, unchanged by this task).
- `AGENTS.md` — states that `libs/backend-api` and `libs/device-api` are auto-generated from the specs in `api/specs/`, must not be edited by hand, and are regenerated via `make -C api` or `api/regen.sh`. It does not prescribe how to obtain the updated YAML.

openapi repo sources (all paths relative to `/home/ben/miru/workbench4/repos/openapi`):

- `apis/apps/backend-server/agent/openapi.gen.yaml` — the agent backend bundle source on `main`. Has 0 `poll_interval_secs` matches (removed), has the upload paths (`/uploads`, `/uploads/{upload_id}/confirm`) and the `UploadRuleSource` schema (around line 825), and has the new `content` property on `BaseUploadRule` (source lines ~939-949). This maps to the agent's `api/specs/backend/v04.yaml`.
- `apis/apps/device-server/openapi.gen.yaml` — the device bundle source (title "Miru Agent API"). Maps to the agent's `api/specs/device/v02.yaml`.
- `build/build-release.sh` — wrapper that requires a clean git tree and runs the release build (`python -m tools.release build`), writing `build/dist/agent.yaml` and `build/dist/device.yaml`. Release builds inject `info.version`, `x-release-version`, and `x-git-commit`, and resolve `$API_VERSION$` / `$RELEASE_VERSION$` placeholders.
- `build/build-snapshot.sh` — same build with `--snapshot`; does not require a clean tree and appends a snapshot suffix to the version.
- The raw `*.gen.yaml` sources are NOT byte-identical to the vendored files: sources carry `info.version 0.0.0` and `$API_VERSION$` placeholders, while vendored files carry a concrete version plus injected `x-release-version` / `x-git-commit`. These header differences do NOT affect the generated Rust models — the generator only reads `info.title`, `info.description`, and the `paths`/`schemas`.
- Caveat: the `build/dist/agent.yaml` committed in the openapi repo is STALE (v0.4, commit 1e48465, still has `poll_interval_secs`). Do NOT copy that committed artifact as-is; it must be rebuilt from `main` first.

Source impact in the agent (hand-written code that references the field being removed):

- `agent/src/models/upload_rule.rs` — line ~42 `pub poll_interval_secs: i32,` in the domain `UploadRuleSource` struct; line ~50 `poll_interval_secs: source.poll_interval_secs,` inside `From<backend_client::UploadRuleSource>`.
- `agent/tests/storage/upload_rules.rs` — line ~17 `poll_interval_secs: 60,`.
- `agent/tests/models/upload_rule.rs` — line ~37 JSON `"poll_interval_secs": 60`; lines ~104 and ~132 struct literals `poll_interval_secs: ...`.
- Do NOT touch `agent/.../workers/poller.rs` — its `poll_interval` is the unrelated sync poller interval, not the upload-rule field.
- The new `content` field on `BaseUploadRule` is additive and optional. The agent's `From<BaseUploadRule>` impl (in `upload_rule.rs`, around lines 106-131) does not read it, so it does not break compilation. Default decision: do NOT surface `content` into the agent domain model — it is out of scope. The generated model simply carries it.

Validation tooling (agent repo):

- `./scripts/preflight.sh` — runs four checks in parallel: `scripts/lint.sh`, `scripts/covgate.sh` (tests + coverage gates), `tools/lint/scripts/lint.sh`, `tools/lint/scripts/covgate.sh`. "Clean" = all four exit 0.
- `./scripts/test.sh` — runs `RUST_LOG=off cargo test --features test`. The `--features test` flag is MANDATORY (mocks/helpers are behind `#[cfg(feature = "test")]`).
- `cargo check --workspace` / `cargo build --workspace` — `libs/backend-api` and `libs/device-api` are workspace members.
- `./scripts/lint.sh` — a separate gate (custom import linter, `cargo fmt`, machete/diet unused-dep checks, security audit, clippy). Run `./scripts/update-deps.sh` first to refresh `Cargo.lock`. Clippy warnings inside generated code are expected and ignorable.


## Plan of Work

Three milestones, one commit each.

M1 — Re-vendor backend spec from openapi main and regenerate models.

Primary path (Strategy B, authoritative): in the openapi repo, build the release (or snapshot) artifacts from `main`, then copy `build/dist/agent.yaml` over the agent's `api/specs/backend/v04.yaml`. The device spec is functionally unchanged; copying `build/dist/device.yaml` over `api/specs/device/v02.yaml` is optional and produces no model changes — skip it unless you want to keep both vendored files exactly in sync with the same build. Then run `api/regen.sh` in the agent repo, which regenerates and copies the model files. Optionally normalize the new `v04.yaml` header to match the existing trimmed style (the release build embeds a fuller `x-git-commit` block with author/branch/dirty/`x-build.built_at` and the full commit-message body); header content does not affect generated models, so this is cosmetic.

Fallback path (Strategy A, surgical) — use only if the openapi repo's Python release-build toolchain (`python -m tools.release`) is unavailable. Note this does NOT remove the Node/Java dependency: `api/regen.sh` runs the openapi-generator-cli (Node/npx + Java JRE) under both strategies, so the generator must be available either way. Edit `api/specs/backend/v04.yaml` directly: remove the `poll_interval_secs` entry from the `UploadRuleSource` `required` list (around line 835), remove its property definition (around lines 843-846), and remove the three example occurrences (around lines 853, 958, 991). Then add a `content` property to `BaseUploadRule`, mirroring source lines ~939-949 of `apis/apps/backend-server/agent/openapi.gen.yaml` (an optional `type: string`). Then run `api/regen.sh`. Verify the result with the same greps below; both strategies must yield identical generated models.

Regen effect either way: `api/regen.sh` MODIFIES `libs/backend-api/src/models/upload_rule_source.rs` (drops the field) and `libs/backend-api/src/models/base_upload_rule.rs` (adds `content: Option<String>`). No model files are added or removed. Device models are unchanged.

M2 — Fix agent source/tests for the removed field. Remove the `poll_interval_secs` references from the three files listed under Source impact above. Leave `workers/poller.rs` untouched. Do not add a `content` field to the domain model.

M3 — Validate. Run `cargo check --workspace`, `./scripts/test.sh`, then `./scripts/update-deps.sh` + `./scripts/lint.sh`, then `./scripts/preflight.sh`. If covgate or fmt force small changes, commit them; otherwise M3 has no code commit.


## Concrete Steps

All commands list their working directory explicitly.

### M1 — Re-vendor + regenerate

Primary (Strategy B). Build the spec in the openapi repo:

    cd /home/ben/miru/workbench4/repos/openapi
    git rev-parse HEAD            # expect 316936e... and a clean tree (git status --short -> empty)
    ./build/build-release.sh      # or ./build/build-snapshot.sh if a clean tree is not guaranteed
    ls build/dist/agent.yaml      # expect the freshly built artifact

Copy the rebuilt agent spec into the agent repo and regenerate:

    cp /home/ben/miru/workbench4/repos/openapi/build/dist/agent.yaml \
       /home/ben/miru/workbench4/repos/agent/api/specs/backend/v04.yaml
    cd /home/ben/miru/workbench4/repos/agent
    ./api/regen.sh

Expected `regen.sh` transcript (abbreviated): `make gen` invokes the generator twice (backend, device), prints generator progress, finishes without error; the script then clears and repopulates `libs/backend-api/src/models` and `libs/device-api/src/models`.

Verify the deltas landed:

    cd /home/ben/miru/workbench4/repos/agent
    grep -rn poll_interval_secs api/specs/                       # expect: no output
    grep -rn poll_interval_secs libs/backend-api/src/models/     # expect: no output
    grep -n content libs/backend-api/src/models/base_upload_rule.rs   # expect: a `content` field line

Inspect the model diff to confirm only the two expected files changed:

    git -C /home/ben/miru/workbench4/repos/agent status --short libs/

Expected: `libs/backend-api/src/models/upload_rule_source.rs` and `libs/backend-api/src/models/base_upload_rule.rs` modified; no other model files added/removed; device models untouched.

Commit M1 (from the agent repo root):

    cd /home/ben/miru/workbench4/repos/agent
    git add api/specs/backend/v04.yaml libs/backend-api/src/models/
    git commit -m "chore(api): sync vendored backend openapi spec and regen models from openapi main" \
      -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M2 — Fix agent source/tests

Edit `agent/src/models/upload_rule.rs`: delete the `pub poll_interval_secs: i32,` field from the domain `UploadRuleSource` struct (around line 42) and the `poll_interval_secs: source.poll_interval_secs,` line in the `From<backend_client::UploadRuleSource>` impl (around line 50).

Edit `agent/tests/storage/upload_rules.rs`: delete the `poll_interval_secs: 60,` line (around line 17).

Edit `agent/tests/models/upload_rule.rs`: delete the JSON line `"poll_interval_secs": 60` (around line 37; also remove the trailing comma on the now-last preceding line if needed to keep valid JSON) and the two struct-literal lines `poll_interval_secs: ...` (around lines 104 and 132).

Confirm only the intended references remain:

    cd /home/ben/miru/workbench4/repos/agent
    grep -rn poll_interval_secs agent/      # expect: only matches in workers/poller.rs

Build to confirm the source compiles:

    cd /home/ben/miru/workbench4/repos/agent
    cargo check --workspace                 # expect: Finished, no errors

Commit M2 (from the agent repo root):

    cd /home/ben/miru/workbench4/repos/agent
    git add agent/src/models/upload_rule.rs agent/tests/storage/upload_rules.rs agent/tests/models/upload_rule.rs
    git commit -m "refactor(uploads): drop poll_interval_secs from agent upload-rule model and tests" \
      -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M3 — Validate

    cd /home/ben/miru/workbench4/repos/agent
    cargo check --workspace      # expect: Finished, no errors
    ./scripts/test.sh            # expect: all tests pass (note: runs cargo test --features test)
    ./scripts/update-deps.sh     # refresh Cargo.lock
    ./scripts/lint.sh            # expect: pass; generated-code clippy warnings are ignorable
    ./scripts/preflight.sh       # expect: clean (all four sub-checks exit 0)

If `cargo fmt` (via lint) or covgate modifies files, review and commit them:

    cd /home/ben/miru/workbench4/repos/agent
    git add -A
    git commit -m "chore(uploads): apply formatting/coverage gate fixups" \
      -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

Otherwise M3 produces no commit.


## Validation and Acceptance

Acceptance is verifiable behavior, not just compiling code:

1. Spec is current: `grep -rn poll_interval_secs api/specs/` (cwd `/home/ben/miru/workbench4/repos/agent`) returns no output.
2. Backend models are current: `grep -rn poll_interval_secs libs/backend-api/src/models/` returns no output, and `grep -n content libs/backend-api/src/models/base_upload_rule.rs` shows the new `content` field. Before this task the first grep matched `upload_rule_source.rs` and the second showed no `content` field; after, those are reversed — demonstrating the change.
3. No stray references: `grep -rn poll_interval_secs agent/` returns only matches inside `workers/poller.rs`.
4. Workspace compiles: `cargo check --workspace` finishes with no errors. (Before M2 this fails because the source references the removed model field; after M2 it passes.)
5. Tests pass: `./scripts/test.sh` (which runs `RUST_LOG=off cargo test --features test`) reports all tests passed. The upload-rule tests fail before M2 (they reference `poll_interval_secs`) and pass after.
6. Preflight clean: `./scripts/preflight.sh` exits 0 (all four parallel sub-checks pass). Preflight MUST report clean before the changes are published.
7. Lint clean: `./scripts/lint.sh` (run after `./scripts/update-deps.sh`) passes as a separate gate; clippy warnings emitted inside generated model code are expected and ignorable.


## Idempotence and Recovery

- Re-vendoring and `api/regen.sh` are idempotent: `regen.sh` clears `libs/*/src/models` before copying, so re-running reproduces the same output. Re-running the openapi build overwrites `build/dist/*.yaml` cleanly.
- If regen produces unexpected model changes (e.g. files added/removed, or device models changed), discard and retry: `git -C /home/ben/miru/workbench4/repos/agent checkout -- libs/` and re-run `api/regen.sh`. If the openapi repo's Python release-build toolchain is unavailable, switch to Strategy A (surgical edit) described in Plan of Work — it converges on the same models.
- The spec copy is a single-file overwrite; to roll it back: `git -C /home/ben/miru/workbench4/repos/agent checkout -- api/specs/backend/v04.yaml`.
- Source edits in M2 are small and reversible via `git checkout -- <file>`. If `cargo check` still fails after M2, re-grep `agent/` for `poll_interval_secs` to find a missed reference (excluding `workers/poller.rs`).
- All commits are per-milestone; if a milestone's validation fails, fix forward within that milestone before committing, or `git reset --soft HEAD~1` to amend the last commit. Do not switch or create branches — stay on `chore/sync-openapi-spec-models`.
