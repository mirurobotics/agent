# Sync vendored backend OpenAPI spec to openapi main (a1bcdf3), regen models, and add upload client operations

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench3/repos/agent` | read-write | Vendored spec update, regenerated models, upload rule i64 fixups, new `http/uploads.rs` client operations + tests |
| `/home/ben/miru/workbench3/repos/openapi` | read-only | Source of truth for the agent-facing backend spec (`main`, HEAD `a1bcdf3`, clean) |

This plan lives in the agent repo (`plans/backlog/`) because all code changes happen there.

Git note: the orchestrator owns branching. The agent repo is already on branch `chore/update-openapi-client-ops` (branched off `main` 0c81056, clean). Do NOT create or switch branches. Commit from inside the agent repo's own git context (cwd `/home/ben/miru/workbench3/repos/agent`), never from the workbench root.

## Purpose / Big Picture

The agent's vendored backend spec (`api/specs/backend/v04.yaml`) is stale: it reflects openapi `main` as of commit 316936e (its header still says 97809d8, but its schema content matches 316936e after the previous sync, `plans/completed/20260630-sync-openapi-spec-models.md`). Since then, openapi `main` (now a1bcdf3) replaced the presigned-URL upload flow with a token-only "broker" flow (downscoped S3/GCS credentials), added a `POST /uploads/{upload_id}/credentials` operation, renamed upload fields, and widened `stability_window_secs` to int64.

After this plan: the vendored spec and generated Rust models in `libs/backend-api` match openapi `main` a1bcdf3; the agent's hand-written HTTP client gains an `http::uploads` module implementing `createUpload`, `vendUploadCredentials`, and `confirmUpload` (the backend seam the in-progress upload pipeline — `agent/src/scan/`, `agent/src/s3/`, `agent/src/gcs/` — will call); no client code references removed schemas; and `./scripts/preflight.sh` reports clean with CI green on the pushed branch head.

Observable outcome: `http::uploads::create/vend_credentials/confirm` free functions exist and are exercised by new tests in `agent/tests/http/uploads.rs` that assert exact method, path, body, and token via the shared `MockClient`; `grep -rn "PresignedUpload\|UploadRequiredHeaders\|file_modified_at\|upload_rule_name" api/specs libs agent` returns nothing.

## Progress

- [x] M1: Re-vendor backend spec from openapi a1bcdf3 + regenerate models (commit 86501a6)
- [x] M2: Fix hand-written code for `stability_window_secs` i64 and removed `content` field (commit 76f82a3)
- [x] M3: Implement upload HTTP client operations + mock + tests (commit e48b5b3)
- [ ] M4: Validate locally and via preflight/CI (clean before PR leaves draft)

## Surprises & Discoveries

- M1/M2 landed as planned: the regen delta matched the Context section exactly (2 files deleted, 4 added, expected modified set; `libs/device-api` untouched), all verification greps passed, and the i64 fallout was confined to the predicted files. Full test suite green after M2 (1449 tests passed).
- M3: `cargo fmt --all` produced formatting fallout in `libs/` generated models; that fallout was reverted as out of scope (CI's fmt check is `cargo fmt -p miru-agent -- --check`, which does not cover generated libs). All 6 new `http::uploads` tests pass; full suite green (1780 tests, 0 failures).
- Refine pass found one issue: a redundant `as i64` cast left at `agent/src/scan/collection.rs:226` after the i64 widening, which trips `clippy -D warnings` (unnecessary_cast). Fixed in 4f37df6.
- M4: `./scripts/update-deps.sh` bumped several transitive deps in Cargo.lock (committed separately); `./scripts/preflight.sh` clean locally on the first run.

## Decision Log

- Decision: Re-vendor by copying `apis/apps/backend-server/agent/openapi.gen.yaml` wholesale, then re-applying only the small header/version customizations (see M1), instead of the openapi repo's release build tooling or a line-by-line surgical edit.
  Rationale: The vendored `v04.yaml` body formatting already matches `openapi.gen.yaml` exactly — `diff` between them today is 493 lines of purely semantic delta plus the header. The prior sync (20260630) found the release build tooling (`build/build-release.sh`) produces noisy dumper-style diffs and version regressions; those problems only apply to `build/dist/` artifacts, not to `openapi.gen.yaml` itself.
  Date/Author: 2026-07-13 / plan author.
- Decision: Implement all three upload operations (`createUpload`, `vendUploadCredentials`, `confirmUpload`) in `agent/src/http/uploads.rs`, but do NOT wire them into `services::backend::BackendFetcher` or any worker.
  Rationale: The repo convention is that only operations the runtime uses (or is being built to use) get client functions — `getVersion`, `syncDevice`, `pingDevice`, `devicePong` have models but no client fn. The upload pipeline (scan/s3/gcs modules) is actively being built and these three ops are its complete backend seam; they must be added together to be usable. Runtime wiring is a separate future task.
  Date/Author: 2026-07-13 / plan author.
- Decision: Propagate `stability_window_secs` as `i64` through the agent domain model (`agent/src/models/upload_rule.rs`) and all scan-module call sites, rather than truncating the generated `i64` to `i32` at the conversion boundary.
  Rationale: Matches the backend contract (spec now says `format: int64`); avoids silent truncation; the compiler enumerates every affected site.
  Date/Author: 2026-07-13 / plan author.
- Decision: Do not surface `Upload.metadata`, `UploadSource.first_observed_at/last_observed_at/mtime`, or the credentials schemas into agent domain models. Generated models carry them; domain modeling happens when the upload worker is built.
  Rationale: Same "do not promote unused fields" call as the 20260630 plan made for `content`.
  Date/Author: 2026-07-13 / plan author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Terms:

- "Vendored spec": a copy of the OpenAPI YAML checked into the agent repo at `api/specs/backend/v04.yaml`. The agent never fetches specs at build time.
- "Regen": running `./api/regen.sh` (from the agent repo root), which invokes openapi-generator-cli 7.12.0 (pinned in `api/openapitools.json`; needs Node/npx + a Java JRE) with custom templates from `api/templates/rust/`, then replaces `libs/backend-api/src/models/*` and `libs/device-api/src/models/*` wholesale with the generated model files. Only models are generated; the HTTP operations layer is hand-written.
- "Broker upload flow": the new backend contract — `createUpload` returns `UploadWithCredentials` (ledger entry + short-lived downscoped S3/GCS credentials scoped to one object key); the device uploads via native cloud SDK, may re-vend credentials mid-upload via `vendUploadCredentials`, then calls `confirmUpload`.

### Spec delta (openapi 316936e → a1bcdf3, agent-facing spec only)

Commits: 36aebf3 (#167 expand=upload_rules on getRelease), 7a4b12c (#168 drop upload-rule `content`), e8e70b6 (#171 upload_rule_name→upload_collection_name), 2e23d0a (#178 token-only upload flow), 4794667 (#179 generic Upload.metadata map), a6b5999 (#185 int64 stability_window_secs), ce3ee88 (#196 mtime + observed_at fields).

Operations:

- `createUpload` (POST `/uploads`): response schema changed `PresignedUpload` → `UploadWithCredentials`. Request body unchanged (`CreateUploadRequest`).
- NEW `vendUploadCredentials` (POST `/uploads/{upload_id}/credentials`, no body): returns `UploadCredentials`.
- `confirmUpload` (POST `/uploads/{upload_id}/confirm`, no body): unchanged, returns `Upload`.
- `getRelease`: gained the `expand` query param (`release_expansions` / `ReleaseExpansion` enum). The vendored spec ALREADY contains this param and enum (from the 97809d8-era sync); only their position in the file moves. `http::releases::get` already accepts expansions and `services/backend.rs` already passes `["upload_rules"]` — no client change needed.
- No operations were removed from the spec, and no existing client function references a dropped operation — so the "remove dropped operations" half of this task is a verified no-op.

Schemas:

- REMOVED: `PresignedUpload`, `UploadRequiredHeaders`.
- ADDED: `UploadWithCredentials` (`upload` + `credentials`), `UploadCredentials` (scheme s3|gcs + nullable `s3_credentials`/`gcs_credentials` + `expires_at`), `S3UploadCredentials`, `GcsUploadCredentials`.
- `UploadSource`: `file_modified_at` renamed to `mtime`; new required `first_observed_at`, `last_observed_at`.
- `BaseUpload`: `upload_rule_name` renamed to `upload_collection_name`.
- `Upload`: new optional `metadata` (map of string→string).
- `BaseUploadRule`: optional `content` field removed; its source's `stability_window_secs` gains `format: int64`.

### Generated-model impact (`libs/backend-api/src/models/`, all replaced by regen)

- Deleted files: `presigned_upload.rs`, `upload_required_headers.rs`.
- New files: `upload_with_credentials.rs`, `upload_credentials.rs`, `s3_upload_credentials.rs`, `gcs_upload_credentials.rs`.
- Modified: `upload_source.rs` (mtime + observed fields), `base_upload.rs` (upload_collection_name), `upload.rs` (metadata), `base_upload_rule.rs` (content dropped), `upload_rule_source.rs` (`stability_window_secs: i64`), `mod.rs` (regenerated exports). Possibly cosmetic reordering elsewhere. `libs/device-api` must be untouched.

### Hand-written code impact (found by grep; the compiler will confirm)

`stability_window_secs` is `i32` today in these hand-written locations and must become `i64` (or have call sites adjusted) once the generated `UploadRuleSource.stability_window_secs` turns `i64`:

- `agent/src/models/upload_rule.rs` — line ~42 `pub stability_window_secs: i32` (domain `UploadRuleSource`), line ~49 copies it in `From<backend_client::UploadRuleSource>`.
- `agent/src/scan/scanner.rs` ~522, 529; `agent/src/scan/collection.rs` ~226, 338, 1321; `agent/src/scan/state.rs` ~189-194 (`fn rule(..., window: i32)`); `agent/src/disk/upload_rules.rs` ~53, 58 (`fn rule_with(..., stability_window_secs: i32)` in a `#[cfg(test)]` block).
- Tests: `agent/tests/disk/upload_rules.rs` ~17, `agent/tests/models/upload_rule.rs` ~37 (JSON), ~103, ~131 (struct literals), `agent/tests/workers/scan_bridge.rs` ~134, 139.

Removed `content` field: `agent/tests/models/upload_rule.rs::backend_rule()` builds a full `backend_client::BaseUploadRule` struct literal including `content: None` (added by the 20260630 sync) — that line must be deleted. `agent/tests/sync/helpers.rs::make_backend_upload_rule()` uses `..Default::default()` and needs no change.

No hand-written code references `PresignedUpload`, `UploadRequiredHeaders`, `file_modified_at`, or `upload_rule_name` (verified by grep over `agent/` and `tools/`), so those renames/removals only touch generated files.

### HTTP client operations layer

- `agent/src/http/` holds one file per resource with free functions over `ClientI` (see `devices.rs` for the POST pattern: param structs + `request::Params::post(&url, request::marshal_json(payload)?)` for JSON bodies, `request::Params::post(&url, String::new())` for empty bodies — `issue_token` is the empty-body example). `agent/src/http/mod.rs` lists modules alphabetically.
- Currently implemented ops: deployments list/get/update, config instance content, git commit get, release get, device provision/reprovision/issue_token/update/get. No uploads module exists yet.
- Tests live in `agent/tests/http/<resource>.rs` and use `crate::mocks::http_client::{Call, CapturedRequest, MockClient}` (defined in `agent/tests/mocks/http_client.rs`): the mock routes `(method, path)` to a `Call` enum variant (~line 198) and serves a per-call response closure returning a `Default` model (~line 223). Each op gets a `success` test asserting the `CapturedRequest` (method/path/query/body/token) and an `error_propagates` test.
- Coverage: `agent/src/http/.covgate` is 93.9 — the new module must ship with tests covering all three functions.

### Validation tooling

- `./scripts/test.sh` — `RUST_LOG=off cargo test --features test`; `--features test` is mandatory.
- `./scripts/lint.sh` — import linter, `cargo fmt`, machete/diet, audit, clippy. Run `./scripts/update-deps.sh` first. Clippy warnings inside generated model code are expected and ignorable.
- `./scripts/preflight.sh` — runs `scripts/lint.sh`, `scripts/covgate.sh`, `tools/lint/scripts/lint.sh`, `tools/lint/scripts/covgate.sh` in parallel; clean = all exit 0. Preflight mirrors the CI jobs in `.github/workflows/ci.yml` (lint, test/covgate, tools).

## Plan of Work

M1 — Re-vendor + regenerate. Copy `/home/ben/miru/workbench3/repos/openapi/apis/apps/backend-server/agent/openapi.gen.yaml` over `api/specs/backend/v04.yaml`, then re-apply the vendored header customizations to the new file (the raw gen.yaml carries build-time placeholders):

1. `info.version`: `0.0.0` → `v0.5.0-pre` (line 5).
2. Re-insert, after the `license:` block, the two vendored header keys (matching the current file's style, lines 9-13):

       x-release-version: v0.5.0-pre
       x-git-commit:
         sha: a1bcdf376f6aa360a024730ed941dc5a6547b63c
         url: https://github.com/mirurobotics/openapi/commit/a1bcdf376f6aa360a024730ed941dc5a6547b63c
         message: 'feat(buckets): expose read-only gcs session_name for wif binding (#197)'

3. Replace both `$API_VERSION$` occurrences with `v0.5.0-pre` (the `MiruVersion` enum entry ~line 440 and the `example:` ~line 1936).

Then run `./api/regen.sh` and verify the generated-model impact matches the Context section (files added/removed/modified; device models untouched). Commit.

M2 — Hand-written fallout. Change `stability_window_secs` to `i64` in `agent/src/models/upload_rule.rs` and chase the compile errors through the scan/disk modules and tests listed in Context (mechanical `i32`→`i64`; integer literals need no suffix changes). Delete `content: None,` from `agent/tests/models/upload_rule.rs::backend_rule()`. `cargo check --workspace` then compiles and `./scripts/test.sh` passes. Commit.

M3 — Upload client operations. Create `agent/src/http/uploads.rs` following the `devices.rs` pattern (import ordering per AGENTS.md; param structs; free functions):

- `pub struct CreateParams<'a> { pub payload: &'a CreateUploadRequest, pub token: &'a str }`; `pub async fn create(client: &impl ClientI, params: CreateParams<'_>) -> Result<UploadWithCredentials, HTTPErr>` — POST `{base}/uploads` with `request::marshal_json(params.payload)?`.
- `pub struct VendCredentialsParams<'a> { pub id: &'a str, pub token: &'a str }`; `pub async fn vend_credentials(...) -> Result<UploadCredentials, HTTPErr>` — POST `{base}/uploads/{id}/credentials` with `String::new()` body.
- `pub struct ConfirmParams<'a> { pub id: &'a str, pub token: &'a str }`; `pub async fn confirm(...) -> Result<Upload, HTTPErr>` — POST `{base}/uploads/{id}/confirm` with `String::new()` body.

Register `pub mod uploads;` in `agent/src/http/mod.rs` (alphabetical position). Extend `agent/tests/mocks/http_client.rs`: add `Call::CreateUpload`, `Call::VendUploadCredentials`, `Call::ConfirmUpload` variants; add routing arms (order matters — match the two suffixed paths before any generic `/uploads` arm): POST + path == `/uploads` → CreateUpload; POST + starts_with `/uploads/` + ends_with `/credentials` → VendUploadCredentials; POST + starts_with `/uploads/` + ends_with `/confirm` → ConfirmUpload; add response closures returning `UploadWithCredentials::default()`, `UploadCredentials::default()`, `Upload::default()` following the existing per-call `*_fn` pattern. Add `agent/tests/http/uploads.rs` mirroring `agent/tests/http/devices.rs` (`success` + `error_propagates` per operation, asserting the full `CapturedRequest`) and register it in `agent/tests/http/mod.rs`. Commit.

M4 — Validate. Full local validation, then preflight; push happens via the orchestrator's PR flow, and CI on the pushed head must be green before the PR leaves draft. Commit only if fmt/covgate force fixups.

## Concrete Steps

All commands state their working directory. Agent repo root = `/home/ben/miru/workbench3/repos/agent`.

### M1 — Re-vendor + regenerate

    cd /home/ben/miru/workbench3/repos/openapi
    git rev-parse HEAD          # expect a1bcdf376f6aa360a024730ed941dc5a6547b63c
    git status --short          # expect empty

    cp /home/ben/miru/workbench3/repos/openapi/apis/apps/backend-server/agent/openapi.gen.yaml \
       /home/ben/miru/workbench3/repos/agent/api/specs/backend/v04.yaml

Apply the three header edits from Plan of Work (info.version, x-release-version + x-git-commit block, both `$API_VERSION$` occurrences). Verify:

    cd /home/ben/miru/workbench3/repos/agent
    grep -c 'v0.5.0-pre' api/specs/backend/v04.yaml     # expect 4 (info.version, x-release-version, enum entry, example)
    grep -n 'API_VERSION\|version: 0.0.0' api/specs/backend/v04.yaml   # expect no output
    grep -n 'a1bcdf37' api/specs/backend/v04.yaml       # expect 2 lines (sha + url)

Regenerate (needs npx + Java):

    cd /home/ben/miru/workbench3/repos/agent
    ./api/regen.sh              # generator runs twice (backend, device), exits 0

Verify the model delta:

    git status --short libs/
    # expect: backend models only. Deleted: presigned_upload.rs, upload_required_headers.rs.
    # Added: upload_with_credentials.rs, upload_credentials.rs, s3_upload_credentials.rs, gcs_upload_credentials.rs.
    # Modified: upload_source.rs, base_upload.rs, upload.rs, base_upload_rule.rs, upload_rule_source.rs, mod.rs (+/- cosmetic churn).
    # libs/device-api: no changes.
    grep -rn 'file_modified_at\|upload_rule_name\|PresignedUpload\|UploadRequiredHeaders' api/specs libs/backend-api/src   # expect no output
    grep -n 'i64' libs/backend-api/src/models/upload_rule_source.rs    # expect stability_window_secs: i64

Commit M1:

    cd /home/ben/miru/workbench3/repos/agent
    git add api/specs/backend/v04.yaml libs/backend-api/src/models/
    git commit -m "chore(api): sync vendored backend openapi spec and regen models from openapi main (a1bcdf3)" \
      -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"

### M2 — Hand-written fallout

Edit `agent/src/models/upload_rule.rs`: `pub stability_window_secs: i32` → `i64` (~line 42; the `From` impl at ~49 then compiles as-is). Run `cargo check --workspace` and fix every resulting `i32`/`i64` mismatch in: `agent/src/scan/scanner.rs`, `agent/src/scan/collection.rs`, `agent/src/scan/state.rs`, `agent/src/disk/upload_rules.rs`, `agent/tests/disk/upload_rules.rs`, `agent/tests/models/upload_rule.rs`, `agent/tests/workers/scan_bridge.rs` (change declared types/parameters to `i64`; do not sprinkle `as` casts). Delete the `content: None,` line from `agent/tests/models/upload_rule.rs::backend_rule()`.

    cd /home/ben/miru/workbench3/repos/agent
    cargo check --workspace     # expect: Finished, no errors
    ./scripts/test.sh           # expect: all tests pass

Commit M2:

    git add agent/src agent/tests
    git commit -m "refactor(uploads): widen stability_window_secs to i64 and drop removed content field" \
      -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"

### M3 — Upload client operations

Create `agent/src/http/uploads.rs`, edit `agent/src/http/mod.rs`, extend `agent/tests/mocks/http_client.rs`, create `agent/tests/http/uploads.rs`, edit `agent/tests/http/mod.rs` — all as specified in Plan of Work.

    cd /home/ben/miru/workbench3/repos/agent
    cargo check --workspace     # expect: Finished, no errors
    ./scripts/test.sh           # expect: all tests pass, including new http::uploads tests (6+ new tests)

Commit M3:

    git add agent/src/http agent/tests
    git commit -m "feat(http): add upload client operations (create, vend credentials, confirm)" \
      -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"

### M4 — Validate

    cd /home/ben/miru/workbench3/repos/agent
    cargo check --workspace     # expect: clean
    ./scripts/test.sh           # expect: all pass
    ./scripts/update-deps.sh    # refresh Cargo.lock (commit if changed)
    ./scripts/lint.sh           # expect: pass (generated-code clippy warnings ignorable)
    ./scripts/preflight.sh      # expect: clean — all four sub-checks exit 0

If fmt/covgate produce fixups:

    git add -A
    git commit -m "chore(http): apply formatting/coverage gate fixups" \
      -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"

Then push via the orchestrator's PR flow and watch CI on the pushed branch head (`gh run watch` or `gh pr checks`). CI must be green before proceeding.

## Validation and Acceptance

1. Spec current: from the agent repo root, `diff <(sed -n '14,$p' api/specs/backend/v04.yaml) <(sed -n '9,$p' /home/ben/miru/workbench3/repos/openapi/apis/apps/backend-server/agent/openapi.gen.yaml)` (skipping each file's info header: vendored header is lines 1-13, gen.yaml's is lines 1-8) shows only the two version-substitution lines (`v0.5.0-pre` vs `$API_VERSION$`); and the header greps in M1 pass.
2. Models current: `grep -rn 'file_modified_at\|upload_rule_name\|PresignedUpload\|UploadRequiredHeaders' api/specs libs agent` returns no output (before M1 it matches the spec and two model files); `ls libs/backend-api/src/models/ | grep -c credentials` returns 3 (gcs, s3, upload); `upload_with_credentials.rs` exists.
3. Client surface matches spec: `agent/src/http/uploads.rs` defines `create`, `vend_credentials`, `confirm`; no client function references any operation absent from `api/specs/backend/v04.yaml` (spot-check: every `format!("{}/...` path in `agent/src/http/*.rs` appears in the spec's `paths:`).
4. Tests: `./scripts/test.sh` passes; the new `agent/tests/http/uploads.rs` tests fail before M3 (module absent → compile error) and pass after, asserting POST `/uploads`, POST `/uploads/{id}/credentials`, POST `/uploads/{id}/confirm` with correct body/token.
5. Compile gate: `cargo check --workspace` fails between M1 and M2 (i64 mismatch, removed `content`) and passes from M2 on — demonstrating the model changes were absorbed.
6. Preflight/CI gate (hard requirement): `./scripts/preflight.sh` exits 0 locally, AND preflight must report CLEAN — meaning CI is green on the pushed branch head (`chore/update-openapi-client-ops`) — before the PR leaves draft or the task is reported complete. A red or pending CI head blocks completion.

## Idempotence and Recovery

- The spec copy is a single-file overwrite; re-running `cp` + header edits is idempotent. Rollback: `git checkout -- api/specs/backend/v04.yaml`.
- `./api/regen.sh` clears `libs/*/src/models` before copying, so re-runs are reproducible. If regen output diverges from the expected delta (extra files, device-model churn), rollback with `git checkout -- libs/` and re-check the spec edits before retrying.
- If npx/Java are unavailable for regen, stop and fix the toolchain — do not hand-edit generated models (`libs/*/src/models` are generated-only per AGENTS.md).
- M2/M3 edits are ordinary source changes, reversible per file via `git checkout -- <path>`. If `cargo check` still fails after M2, re-grep for `stability_window_secs` to find missed `i32` sites.
- One commit per milestone; to amend a milestone before pushing, `git reset --soft HEAD~1`, fix, recommit. Never create or switch branches — stay on `chore/update-openapi-client-ops`.
