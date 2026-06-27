# Upload-rules spec-sync follow-up: rename interval fields to `_secs` and retype as integer

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** MUST be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench4/repos/agent` (the Rust device agent — the only repo edited) | read-write | Re-vendor `api/specs/backend/v04.yaml` from the openapi `main` bundle at `fe6e9ca`, regenerate `libs/backend-api` models, and update the hand-rolled `agent/src/models/upload_rule.rs` + its tests/fixtures for the renamed/retyped upload-rule source fields. |
| `/home/ben/miru/workbench4/repos/openapi` | read-only | Source of the vendored bundle. Read `apis/apps/backend-server/agent/openapi.gen.yaml` at `origin/main` (`fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f`). No edits. |

This plan lives in `plans/backlog/` per the agent repo's plan conventions (`plans/{active,archived,backlog,completed}/`). Branch: `feat/uploads-read-path` (the PR #89 branch). Commit all changes from inside the agent repo's own git context (see workbench `CLAUDE.md`), never from the workbench root.

**This plan is a pure contract-shape sync.** It re-vendors the agent bundle from openapi `4c92b71` → `fe6e9ca` and propagates the single field-shape change into the agent. It adds NO new behavior.

### The change being synced

openapi commit `fe6e9ca` (#146, "refactor(upload-rule): rename interval fields to `_secs` and type as integer") is the ONLY change in the agent bundle between `4c92b71` (vendored by #89) and `fe6e9ca` (verified by `git diff`). It renames and retypes the two `UploadRuleSource` duration fields:

- `poll_interval` (string, pattern `^\d+(s|m|h)$`, e.g. `"60s"`) → `poll_interval_secs` (integer, e.g. `60`)
- `stability_window` (string, e.g. `"60s"`) → `stability_window_secs` (integer, e.g. `60`)

Both become required `type: integer` (no `format:` → the generator maps bare `integer` to Rust `i32`, same as `limit`/`offset` in `libs/backend-api/src/models/paginated_list.rs`).

### Explicitly OUT OF SCOPE

Everything deferred from #89's M2–M5 remains out of scope and MUST NOT be implemented here:

- File discovery / glob matching on `source.glob` (M2).
- Per-rule poll loop honoring `poll_interval_secs`; stability/finalization detection via `stability_window_secs` (M2). This plan stores the integers; nothing consumes them.
- Streaming sha256 digest + size computation (M3).
- `POST /uploads` (`createUpload`) + presigned `PUT` with `required_headers` (M3).
- `POST /uploads/{upload_id}/confirm` (`confirmUpload`) (M3).
- Local uploads ledger / idempotency / retry state (M4).
- `delete_policy` enforcement (M5).
- Any background upload worker or `app/run.rs` integration (M5).

No interpretation of the new integers as durations is in scope — they are stored as plain integers, exactly as the string fields were stored as plain strings in #89.

## Purpose / Big Picture

#89 (this branch) vendored the openapi agent bundle at `4c92b71` as a `v0.5.0-pre` pre-release snapshot and added the upload-rules READ path (`models/upload_rule.rs`, `http/upload_rules.rs`, `storage/upload_rules.rs`, sync fetch+cache). The completed plan is `plans/completed/20260626-agent-uploads-read-path.md`.

Since then, openapi `main` advanced one commit (`fe6e9ca`) that reshapes `UploadRuleSource`. This plan re-vendors the bundle to that commit and propagates the field rename/retype so the agent's domain model matches the current contract. It is intentionally tiny and self-contained: re-vendor → regen → rename two fields (`String` → `i32`) in the hand-rolled model and the From conversion → fix every test/fixture that names the old fields.

**Observable outcome at completion:** `api/specs/backend/v04.yaml` carries `poll_interval_secs`/`stability_window_secs` as integers and `x-git-commit.sha = fe6e9ca...`; `libs/backend-api/src/models/upload_rule_source.rs` exposes the two fields as `i32`; `agent/src/models/upload_rule.rs::UploadRuleSource` holds `poll_interval_secs: i32` and `stability_window_secs: i32`; all upload-rule unit tests pass against the new shape; `scripts/preflight.sh` prints `Preflight clean` with `models` coverage still at 100%.

## Progress

- [x] **S0** Preflight the generator (`openapi-generator-cli` reachable) — gate; STOP/report if unavailable. (2026-06-26: `7.12.0` available.)
- [x] **S1** Re-vendor `api/specs/backend/v04.yaml` from openapi `fe6e9ca` (keep #89 stamping shape; bump `x-git-commit` only). (2026-06-26: diff vs previous vendor is exactly x-git-commit + the two `*_secs` integer fields/examples; nothing else.)
- [x] **S2** Regenerate models via `api/regen.sh`; confirm `upload_rule_source.rs` fields are `i32` named `*_secs`. (2026-06-26: only `upload_rule_source.rs` changed; both fields render `i32`.)
- [x] **S3** Update `agent/src/models/upload_rule.rs` (`UploadRuleSource` fields + `From` conversion). (2026-06-26: fields now `poll_interval_secs: i32` / `stability_window_secs: i32`; derived `Default` still valid.)
- [x] **S4** Update tests/fixtures: `tests/models/upload_rule.rs`, `tests/storage/upload_rules.rs`, and re-verify `tests/http/upload_rules.rs`, `tests/mocks/http_client.rs`, `tests/sync/deployments.rs`. (2026-06-26: model JSON fixture + both builders updated to integers; http/mocks/sync use `..default()` and needed no edit — all compile and pass.)
- [~] **V** `scripts/preflight.sh` reports `Preflight clean` — DEFERRED to a later step per orchestration. (2026-06-26: validated the touched surface instead — `cargo build -p miru-agent` succeeds and all 28 upload-rule tests pass via `cargo test --features test`. Full preflight/coverage gate not run here.)

Use timestamps when completing steps. Split partially-completed work into "done" / "remaining".

## Surprises & Discoveries

(Seed findings; add entries as work proceeds.)

- **2026-06-26 (execution): every prediction held; no surprises.** The bundle at `origin/main` was exactly `fe6e9ca`. The raw-bundle-vs-current-vendor diff was precisely the predicted set: stamping (`version`, `x-release-version`+`x-git-commit` block, two `$API_VERSION$` placeholders) plus the `UploadRuleSource` field rename/retype and example blocks — nothing else. Generated regen touched only `upload_rule_source.rs` (`codegen/` is gitignored, so it never appeared in `git status`). Both fields rendered `i32` as predicted. The `..default()` http/mocks/sync tests needed no edit and pass. `cargo machete`/full preflight intentionally not run (deferred).

- **The diff is verified to be exactly the two-field rename/retype.** `git -C repos/openapi diff 4c92b71 fe6e9ca -- apis/apps/backend-server/agent/openapi.gen.yaml` touches ONLY the `UploadRuleSource` schema (required list, the two property definitions, and the inline `example:` blocks) plus the `BaseUploadRule` example. No other schema, path, or enum changes. So regen should diff only `upload_rule_source.rs` (and the version/commit stamp is the only `info:`-block change in the spec).
- **Bare `integer` (no `format:`) → `i32`** under this generator + the repo's `api/templates/rust/model.mustache`. Evidence: `paginated_list.rs` renders `pub limit: i32` / `pub offset: i32` for the bundle's unformatted `integer` page fields, while `int64`-formatted `total_count` renders `Option<i64>`. The two new fields have no `format:`, so expect `i32`. **Confirm after S2** and match the domain-model type to whatever regen actually produces (S3).
- **The `Miru-Version` header is unchanged.** #89 substituted the `$API_VERSION$` placeholder (the `APIVersion` enum value and the `MiruVersion` parameter `example:`) with `v0.5.0-pre` to reproduce the openapi release-pipeline stamping. The raw `fe6e9ca` bundle still carries those `$API_VERSION$` placeholders, so the re-vendor MUST re-apply the same substitution; the resulting header stays `v0.5.0-pre` (no version bump in this sync). See Decision Log.
- **Most upload-rule tests do NOT name the renamed fields.** `tests/http/upload_rules.rs`, `tests/mocks/http_client.rs`, and `tests/sync/deployments.rs` build `BaseUploadRule` via `..BaseUploadRule::default()` (grep-verified), so the renamed `UploadRuleSource` fields are filled by `Default` and these files likely need no edit. They MUST still compile and pass after regen; re-verify in S4 (a stale field name would surface as a compile error).
- **Files that DO name the old fields** (grep-verified, repo-wide, excluding `libs/`): `api/specs/backend/v04.yaml`, `agent/src/models/upload_rule.rs` (lines 74–75, 82–83), `agent/tests/models/upload_rule.rs` (JSON fixture + `backend_rule` builder + `from_backend` expected), `agent/tests/storage/upload_rules.rs` (the `rule()` builder). NOTE: `poll_interval`/`stability_window` matches in `agent/src/app/options.rs`, `agent/src/workers/poller.rs`, `agent/src/app/run.rs`, and their tests are UNRELATED (the sync poller's own interval, `idle_timeout_poll_interval`, `poll_interval_secs: i64`) — DO NOT touch them.
- **`agent/src/models/.covgate` requires 100%.** Changing `String` → `i32` removes no branches but the `From`/`Deserialize`/`Default`/serde-roundtrip coverage for `UploadRuleSource` must stay complete. The existing serde harness in `tests/models/upload_rule.rs` already exercises the source struct via the `UploadRule` fixture; updating the fixture values (string → integer JSON) keeps coverage intact.

## Decision Log

- **Decision: re-vendor from openapi `fe6e9ca` keeping the #89 `v0.5.0-pre` stamping shape; bump only `x-git-commit`.** Keep `info.version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, and the `$API_VERSION$` → `v0.5.0-pre` substitution exactly as #89 did. Update `x-git-commit.sha` to `fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f`, `x-git-commit.url` to `https://github.com/mirurobotics/openapi/commit/fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f`, and `x-git-commit.message` to the commit subject (`refactor(upload-rule): rename interval fields to _secs and type as integer (#146)`). Rationale: this is a contract-shape follow-up to the same `-pre` snapshot, not a release bump; the outbound `Miru-Version` header stays `v0.5.0-pre`, so no new backend-acceptance risk beyond what #89 already flagged.
  Date/Author: 2026-06-26 / plan author.
- **Decision: keep the source fields as plain integers, not parsed durations.** Mirror #89's choice to keep the duration fields as raw stored values (it stored `String`; this stores `i32`). Parsing/interpreting `poll_interval_secs`/`stability_window_secs` as a polling/stability window is an M2 concern and is out of scope. Rationale: preserve the "pure contract-shape sync, no new behavior" boundary.
  Date/Author: 2026-06-26 / plan author.
- **Decision: match the domain-model integer type to the generated type.** Use whatever `api/regen.sh` produces for `UploadRuleSource` (expected `i32`); if the generator emits a different integer width, adopt that exact type in `agent/src/models/upload_rule.rs` rather than forcing `i32`. Rationale: the `From<backend_client::UploadRuleSource>` conversion is a plain field move and must not introduce a lossy/implicit cast.
  Date/Author: 2026-06-26 / plan author.

## Context and Orientation

The agent is a Rust workspace rooted at `repos/agent/Cargo.toml`: `agent/` (binary crate — all logic), `libs/backend-api/` and `libs/device-api/` (OpenAPI-generated models; do NOT hand-edit — regenerate via `api/regen.sh`). Repo conventions: `repos/agent/AGENTS.md` (import ordering, `thiserror` errors, `#[cfg(feature = "test")]` gating, `scripts/test.sh` with `--features test`, per-module `.covgate`).

### Spec vendoring & codegen

- `api/specs/backend/v04.yaml` — the vendored agent bundle. After #89 its `info:` block carries `version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, and an `x-git-commit:` block pointing at `4c92b71`. The `UploadRuleSource` schema is around lines 1048–1077 and a `BaseUploadRule` example references the fields around lines 1181–1182.
- `api/regen.sh` — runs `make gen`, wipes `libs/backend-api/src/models/*` and `libs/device-api/src/models/*`, and copies the freshly generated models in. Safe to re-run.
- `api/templates/rust/model.mustache` — custom model template (forward-compatible enums). Codegen uses it; do not bypass.
- `api/openapitools.json` — pins the generator CLI version.

### Domain model & tests (the surface this plan edits)

- `agent/src/models/upload_rule.rs` — hand-rolled domain model. `UploadRuleSource` (lines 71–86) currently holds `poll_interval: String` / `stability_window: String` with a field-move `From<backend_client::UploadRuleSource>`. This is the only production file to edit.
- `agent/tests/models/upload_rule.rs` — serde harness fixture (`RequiredField` "source" JSON), the `backend_rule(...)` `BaseUploadRule` builder, and the `from_backend` expected `UploadRuleSource`.
- `agent/tests/storage/upload_rules.rs` — the `rule(...)` `UploadRule` builder.
- `agent/tests/http/upload_rules.rs`, `agent/tests/mocks/http_client.rs`, `agent/tests/sync/deployments.rs` — build `BaseUploadRule` via `..default()`; re-verify they compile/pass (likely no edit).

### Validation tooling

- `scripts/preflight.sh` runs, in parallel: `scripts/lint.sh`, `scripts/covgate.sh` (tests + coverage gate), and the `tools/lint` lint + covgate. Prints `Preflight clean` on success, `Preflight FAILED (...)` and exits non-zero otherwise.
- `scripts/covgate.sh` runs `cargo test --features test` with coverage and enforces each module's `.covgate` minimum. Relevant thresholds: `agent/src/models/.covgate` = **100**, `agent/src/http/.covgate`, `agent/src/storage/.covgate`, `agent/src/sync/.covgate` (do not lower any threshold — add tests instead).
- `scripts/test.sh` = `RUST_LOG=off cargo test --features test`. The `--features test` flag is REQUIRED (test helpers/mocks are behind `#[cfg(feature = "test")]`).
- `scripts/lint.sh` runs the custom import linter, `cargo fmt`, `cargo machete`/diet unused-dep checks, security audit, and clippy. Run `scripts/update-deps.sh` first to refresh `Cargo.lock`.

## Plan of Work

### S0 — Preflight the generator (gate)

Confirm `openapi-generator-cli` can run before changing anything:

    cd /home/ben/miru/workbench4/repos/agent/api
    npx --yes @openapitools/openapi-generator-cli version

If this fails (no network / npx / Java unavailable), **STOP and report**. Do NOT hand-write or hand-edit generated models in `libs/backend-api/src/models/` — `regen.sh` regenerates the crate wholesale and will silently destroy hand-edits, diverging from the contract. Reporting the blocker is the correct outcome.

### S1 — Re-vendor `api/specs/backend/v04.yaml` from `fe6e9ca`

1. Extract the source bundle (read-only) from openapi:

       cd /home/ben/miru/workbench4/repos/openapi
       git show origin/main:apis/apps/backend-server/agent/openapi.gen.yaml > /tmp/claude-1000/-home-ben-miru-workbench4/0b7d4b5d-b8dc-472f-a9dd-ca87bdaacaa6/scratchpad/agent-bundle-fe6e9ca.yaml
       # sanity: origin/main is at fe6e9ca for this file
       git -C /home/ben/miru/workbench4/repos/openapi show -s --format='%H%n%s' fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f

2. Re-apply the #89 stamping to the extracted bundle, producing the new `repos/agent/api/specs/backend/v04.yaml`. The stamp is identical to #89 EXCEPT the `x-git-commit` block:
   - `info.version: 0.0.0` → `version: v0.5.0-pre`
   - keep `license:` (already matches)
   - `x-release-version: v0.5.0-pre`
   - `x-git-commit:` block: `sha: fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f`, `url: https://github.com/mirurobotics/openapi/commit/fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f`, `message: refactor(upload-rule): rename interval fields to _secs and type as integer (#146)` (keep the same three keys / YAML shape as the existing block).
   - Substitute BOTH `$API_VERSION$` placeholders → `v0.5.0-pre` (the `components.schemas.APIVersion` enum value and the `MiruVersion` parameter `example:`), exactly as #89 did, so regen produces a valid `Miru-Version` header. **Do not skip this** — the raw `fe6e9ca` bundle still carries the placeholders.
   - Leave everything else (`servers:`, `security:`, `paths:`, `components:`) exactly as the `fe6e9ca` source bundle has it — this is the new contract surface, which now contains the `*_secs` integer fields.

   The simplest reliable approach: diff the extracted bundle's `info:` block + `APIVersion` enum + `MiruVersion` example against the current vendored `v04.yaml`, then apply the same hand-stamp #89 used (only the `x-git-commit` values differ). The schema body changes (the two renamed/retyped fields) come for free from the source bundle.

3. Write the stamped result to `repos/agent/api/specs/backend/v04.yaml` (overwrite). Sanity-check:

       cd /home/ben/miru/workbench4/repos/agent
       grep -nE '^  version:|x-release-version:|sha:' api/specs/backend/v04.yaml | head
       grep -nE 'poll_interval_secs|stability_window_secs' api/specs/backend/v04.yaml
       grep -nE 'poll_interval:|stability_window:|\$API_VERSION\$' api/specs/backend/v04.yaml   # expect NO matches

   Expect: version/release lines show `v0.5.0-pre`; `sha:` shows `fe6e9ca...`; the `*_secs` integer fields present; the old `poll_interval:`/`stability_window:` string fields and `$API_VERSION$` placeholders ABSENT.

### S2 — Regenerate models

    cd /home/ben/miru/workbench4/repos/agent
    api/regen.sh
    cargo build -p backend-api 2>&1 | tail -20

Confirm the source model retyped/renamed:

    grep -nE 'poll_interval_secs|stability_window_secs|i32|i64' libs/backend-api/src/models/upload_rule_source.rs

Expect `pub poll_interval_secs: i32` and `pub stability_window_secs: i32` (note the actual integer type and use it in S3). Also confirm `git diff --stat libs/backend-api/src/models/` shows essentially only `upload_rule_source.rs` changed (plus regen's deterministic field reordering, if any) — anything broader means the spec stamp diverged from #89's and should be reconciled before proceeding.

### S3 — Update the hand-rolled domain model

`agent/src/models/upload_rule.rs`, `UploadRuleSource` (around lines 71–86):

- Rename `poll_interval` → `poll_interval_secs` and `stability_window` → `stability_window_secs`, changing the type from `String` to the integer type regen produced in S2 (expected `i32`).
- Update the `From<backend_client::UploadRuleSource>` conversion (lines 78–86) to move `source.poll_interval_secs` / `source.stability_window_secs` into the renamed fields.
- `#[derive(... Default ...)]` stays valid (`i32` defaults to `0`); no manual `Default` needed for the source struct.
- No change to `UploadRule`, `UploadRuleDestination`, `DeletePolicy`, the `From<BaseUploadRule>`, or the custom `Deserialize` (they don't name these fields directly — `UploadRuleSource` is deserialized via its derived impl). Confirm `serde(rename_all)` is not needed: the field names now match the JSON keys exactly (`poll_interval_secs`), so the derived `Serialize`/`Deserialize` round-trips without rename attributes.

### S4 — Update tests/fixtures

Update every reference to the old field names/types (grep-verified set):

- `agent/tests/models/upload_rule.rs`:
  - The `RequiredField { key: "source", value: json!({...}) }` fixture (around lines 35–41): `"poll_interval": "60s"` → `"poll_interval_secs": 60`, `"stability_window": "30s"` → `"stability_window_secs": 30` (JSON numbers, not strings).
  - The `backend_rule(...)` builder's `backend_client::UploadRuleSource { ... }` (around lines 103–106): integer fields.
  - The `from_backend` expected `UploadRuleSource { ... }` (around lines 131–134): integer fields.
- `agent/tests/storage/upload_rules.rs`: the `rule(...)` builder's `UploadRuleSource { ... }` (lines 16–19): `poll_interval_secs: 60`, `stability_window_secs: 30`.
- Re-verify (run, don't assume) `agent/tests/http/upload_rules.rs`, `agent/tests/mocks/http_client.rs`, `agent/tests/sync/deployments.rs`: they build `BaseUploadRule` via `..BaseUploadRule::default()` and are expected to need no edit, but they MUST compile and pass after regen — a leftover old field name would be a compile error.
- Repo-wide guard: after edits,

      cd /home/ben/miru/workbench4/repos/agent
      grep -rn 'poll_interval\b\|stability_window\b' agent/ api/ | grep -iE 'upload'

  should return NOTHING (the only `poll_interval`/`stability_window` hits left in the repo are the UNRELATED sync-poller ones in `app/options.rs`, `workers/poller.rs`, `app/run.rs` — do not touch those).

## Test Steps

Tests use `--features test` (run via `scripts/test.sh`). The existing upload-rules unit tests are UPDATED in place (S4) to the new field names/types and must continue to pass. No new test files are needed — the change is a field rename/retype within fully-covered structs.

### T1. Model serde + conversion (`agent/tests/models/upload_rule.rs`)

After S4, the existing `serde_tests!(UploadRule)` harness, `from_backend`, `from_backend_invalid_dates`, and `defaults` tests must pass with the integer fields. The harness round-trips the `source` fixture (now integer JSON) through `Serialize`/`Deserialize`, and `from_backend` asserts the `From<BaseUploadRule>` conversion moves `poll_interval_secs`/`stability_window_secs`. This keeps `agent/src/models/.covgate` = **100** because `UploadRuleSource`'s derived impls and the `From` field-move stay fully exercised. Verify no coverage regression: the integer fields have no parse/fallback branches (they are plain field moves and derived serde), so the only requirement is that the fixture/builders reference them.

### T2. Storage round-trip (`agent/tests/storage/upload_rules.rs`)

The existing `write_then_read_round_trips` and `write_if_absent_does_not_overwrite_existing` tests must pass with the `rule(...)` builder using integer source fields (write → read identical).

### T3. HTTP + sync re-verification

`agent/tests/http/upload_rules.rs` (list / list_all / pagination / error) and the sync fixtures in `agent/tests/sync/deployments.rs` must still pass unchanged (they use `BaseUploadRule::default()`). Their green state confirms the regen didn't break the read-path wiring.

## Validation and Acceptance

**Changes MUST NOT be published until `scripts/preflight.sh` reports `Preflight clean`.** Run from the repo root:

    cd /home/ben/miru/workbench4/repos/agent
    scripts/update-deps.sh        # refresh Cargo.lock before linting
    scripts/preflight.sh          # lint + covgate(tests+coverage) + tools lint/tests, in parallel
    # final line must be: Preflight clean

Plus the individual gates the conventions mandate (all must pass clean):

    cargo build -p backend-api -p miru-agent
    scripts/test.sh                                   # RUST_LOG=off cargo test --features test
    cargo fmt -p miru-agent -- --check
    cargo clippy --package miru-agent --all-features -- -D warnings
    cargo machete

**Coverage gate (`scripts/covgate.sh`, invoked by preflight) imposes a per-module minimum via `.covgate` files and WILL fail the build if coverage drops.** The module this plan touches in source is `agent/src/models/.covgate` = **100** (the renamed `UploadRuleSource` fields must remain fully covered — the updated serde fixture + `from_backend` test do this). `http`, `storage`, and `sync` thresholds must also stay green. If a change lowers a module below its threshold, add tests (do NOT lower the threshold). Generated `libs/backend-api` clippy warnings are expected/ignored.

Acceptance (human-verifiable):

1. `api/specs/backend/v04.yaml` shows `version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, `x-git-commit.sha = fe6e9ca53d4d67c1aaf6ba36e8b3d5c5f0fcc85f`, contains `poll_interval_secs`/`stability_window_secs` as `type: integer`, and contains NO `poll_interval:`/`stability_window:` string fields or `$API_VERSION$` placeholders.
2. `libs/backend-api/src/models/upload_rule_source.rs` exposes `poll_interval_secs` and `stability_window_secs` as integers (`i32`); `cargo build -p backend-api` succeeds.
3. `agent/src/models/upload_rule.rs::UploadRuleSource` holds the two integer `*_secs` fields and its `From` conversion compiles; `cargo build -p miru-agent` succeeds.
4. `scripts/test.sh` runs the updated model/storage tests (T1, T2) and the unchanged http/sync tests (T3) green.
5. `scripts/preflight.sh` prints `Preflight clean`.

## Idempotence and Recovery

- **Regeneration is idempotent.** Re-running `api/regen.sh` reproduces `libs/backend-api/src/models/` from the vendored spec; safe to re-run. If regen output is unexpected, fix the spec stamp and re-run — never hand-edit generated files.
- If `openapi-generator-cli` is unavailable, the whole plan is blocked at S0; STOP and report (do not hand-write models).
- If `cargo build -p miru-agent` fails after S3 with a missing/renamed-field error on `UploadRuleSource`, a field name or type wasn't propagated — re-check S3 against the actual generated `upload_rule_source.rs`.
- If a test fails to compile after regen with an unknown-field error, S4 missed a fixture/builder — re-run the repo-wide grep guard.
- Rollback: `git -C /home/ben/miru/workbench4/repos/agent checkout -- api/specs/backend/v04.yaml libs/backend-api agent/src/models/upload_rule.rs agent/tests` restores pre-change state (only if abandoning).
- Commit the spec + regenerated models separately from the hand-written model/test edits so the generated churn is isolated and reviewable (mirrors #89's commit split).

## Outcomes & Retrospective

**2026-06-26 — implemented (S0–S4 done; V deferred).** Two commits in the agent repo on `feat/uploads-read-path`:

1. `chore(api): re-vendor agent bundle at fe6e9ca, retype upload-rule intervals` — `api/specs/backend/v04.yaml` (re-vendored from `fe6e9ca`, `v0.5.0-pre` stamping preserved, `x-git-commit` bumped) + regenerated `libs/backend-api/src/models/upload_rule_source.rs` (`poll_interval_secs: i32`, `stability_window_secs: i32`).
2. `feat(uploads): retype upload-rule interval fields to integer seconds` — `agent/src/models/upload_rule.rs` (`UploadRuleSource` fields + `From` conversion) and the test fixtures/builders in `agent/tests/models/upload_rule.rs` + `agent/tests/storage/upload_rules.rs`.

Validation performed: `cargo build -p backend-api`, `cargo build -p miru-agent`, and `cargo test --features test` filtered to `upload_rule` (28 tests, all green, including the `..default()`-based http/sync tests). The full `scripts/preflight.sh` / coverage gate was intentionally NOT run — it is owned by a later orchestration step. No deviations from the plan; every prediction in Surprises & Discoveries held.

---

Change note (2026-06-26): Initial draft. Contained spec-sync follow-up to #89: re-vendor the agent bundle from openapi `4c92b71` → `fe6e9ca` (`v0.5.0-pre` stamping unchanged, only `x-git-commit` bumped), regenerate models, and rename/retype the two `UploadRuleSource` interval fields (`poll_interval`/`stability_window` strings → `poll_interval_secs`/`stability_window_secs` `i32`) in the hand-rolled model and all upload-rule tests/fixtures. Pure contract-shape sync — no new behavior. `Miru-Version` header stays `v0.5.0-pre`.
