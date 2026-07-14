# Re-vendor backend OpenAPI spec to openapi main (8a902ed) and regen models for UploadDestination.bucket_name

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench3/repos/agent` | read-write | Vendored spec update + regenerated `libs/backend-api` models |
| `/home/ben/miru/workbench3/repos/openapi` | read-only | Source of truth for the agent-facing backend spec (`main`, HEAD `8a902ed`, clean) |

This plan lives in the agent repo (`plans/backlog/`) because all code changes happen there.

Git note: the orchestrator owns branching. The agent repo is already on branch `feat/upload-destination-bucket-name` (branched off `main` dc7974c, clean). Do NOT create or switch branches. Commit from inside the agent repo's own git context (cwd `/home/ben/miru/workbench3/repos/agent`), never from the workbench root.

## Purpose / Big Picture

The backend added a required `bucket_name` field to the `UploadDestination` schema (openapi commit 8a902ed, PR #199): the physical object-store bucket name the device passes to the native cloud SDK, as opposed to the Miru `bucket_id`. The agent's upload pipeline needs this field to hand the correct bucket to the S3/GCS SDKs. This plan re-vendors the spec and regenerates the Rust models so `backend_client::UploadDestination` carries `pub bucket_name: String`.

After this plan: `api/specs/backend/v04.yaml` matches openapi `main` 8a902ed (with the standard vendored-header customizations), `libs/backend-api/src/models/upload_destination.rs` has the required `bucket_name` field, the workspace compiles, all tests pass, and `./scripts/preflight.sh` reports clean with CI green on the pushed branch head.

Observable outcome: `grep -n bucket_name libs/backend-api/src/models/upload_destination.rs` shows the field (it matches nothing before this change), and deserializing an `Upload` whose `destination` lacks `bucket_name` now fails — matching the backend contract.

## Progress

- [ ] M1: Re-vendor backend spec from openapi 8a902ed + regenerate models + absorb fallout (commit)
- [ ] M2: Validate locally and via preflight/CI (clean before PR leaves draft)

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Re-vendor by copying `apis/apps/backend-server/agent/openapi.gen.yaml` wholesale and re-applying the small header customizations, exactly as the previous sync did (`plans/completed/20260713-sync-openapi-spec-and-upload-client-ops.md`, PR #149).
  Rationale: The current vendored `v04.yaml` is byte-identical to the a1bcdf3 `openapi.gen.yaml` outside the header/version substitutions, so a wholesale copy from 8a902ed yields a minimal, purely semantic diff. The openapi repo's release build tooling (`build/dist/`) was rejected by the 20260630 sync for dumper-style noise and version regressions.
  Date/Author: 2026-07-13 / plan author.
- Decision: Do not surface `bucket_name` into any agent domain model in this plan. The generated model carries it; the upload worker/transfer code that will consume it does not exist yet on `main` (no `agent/src/upload/transfer.rs`).
  Rationale: Same "do not promote unused fields" convention as the 20260630 and 20260713 syncs.
  Date/Author: 2026-07-13 / plan author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Terms:

- "Vendored spec": a copy of the OpenAPI YAML checked into the agent repo at `api/specs/backend/v04.yaml`. The agent never fetches specs at build time.
- "Regen": running `./api/regen.sh` from the agent repo root. It invokes openapi-generator-cli 7.12.0 (pinned in `api/openapitools.json`; needs Node/npx + a Java JRE) via `make gen` in `api/`, with custom templates from `api/templates/rust/`, then replaces `libs/backend-api/src/models/*` and `libs/device-api/src/models/*` wholesale. Only models are generated; the HTTP operations layer is hand-written. The repo toolchain is pinned by `rust-toolchain.toml` (1.97.0).
- `UploadDestination` vs `UploadRuleDestination`: two different schemas. `UploadDestination` (vendored spec ~line 1290) is the per-upload destination on `Upload`/`BaseUpload` — this is what gains `bucket_name`. `UploadRuleDestination` (~line 885, also has a `bucket_id`) is the upload-rule config destination — unchanged; the `bucket_id` references in `agent/src/models/upload_rule.rs` belong to it and are untouched.

### Spec delta (openapi a1bcdf3 → 8a902ed, agent-facing bundle only)

One commit: 8a902ed "feat(uploads): add bucket_name to upload destination (#199)". The full diff of `apis/apps/backend-server/agent/openapi.gen.yaml` is 8 inserted lines, all in `UploadDestination` and its examples:

- `bucket_name` added to the schema's `required` list.
- New property: `type: string`, `example: my-uploads-bucket`, description "Physical name of the bucket in the object store. This is the bucket parameter the device passes to the native cloud SDK (not the Miru `bucket_id`)."
- `bucket_name: my-uploads-bucket` added to three example blocks (the schema example, the `Upload` example, and the confirm-response example).

No operations, parameters, or other schemas changed. Note: `UploadRuleDestination` already carries its own `bucket_name` field (5 occurrences in the current vendored spec, lines ~890-1010), so after this sync the spec contains 10 `bucket_name` matches total — the 5 new ones are all inside `UploadDestination` and its examples.

### Vendored header customizations

The vendored `v04.yaml` differs from the raw `openapi.gen.yaml` only in its header (vendored lines 1-13 vs gen.yaml lines 1-8) and two version substitutions:

1. `info.version`: gen.yaml says `0.0.0`; vendored says `v0.5.0-pre`.
2. Vendored adds, after the `license:` block, `x-release-version: v0.5.0-pre` and an `x-git-commit:` block (`sha`, `url`, `message`) recording which openapi commit the spec was vendored from. It currently points at a1bcdf3 and must be updated to point at 8a902ed.
3. gen.yaml contains two `$API_VERSION$` placeholders (the `MiruVersion` enum entry ~line 447 and an `example:` near the end); vendored substitutes `v0.5.0-pre` for both.

### Generated-model impact (`libs/backend-api/src/models/`, all replaced by regen)

Exactly one file changes: `upload_destination.rs` gains `pub bucket_name: String` (with `#[serde(rename = "bucket_name")]` and the doc comment) and `UploadDestination::new()` gains a `bucket_name: String` parameter. No files are added or removed, `mod.rs` is unchanged, and `libs/device-api` must be untouched. (`Upload::new`/`BaseUpload::new` signatures are unaffected — they already take a whole `models::UploadDestination`.)

### Hand-written code impact: none expected (verified)

Grep-verified on `main` (dc7974c): there are zero hand-written constructor calls, struct literals, or JSON fixtures for `UploadDestination`, `Upload`, or `BaseUpload` —

- `grep -rn "UploadDestination" agent/ tools/ --include='*.rs'` returns nothing (only `libs/backend-api` matches).
- The MockClient response closures (`agent/tests/mocks/http_client.rs`) and the `agent/tests/http/uploads.rs` assertions use `UploadWithCredentials::default()` / `Upload::default()`; the generated structs derive `Default`, so the new `String` field defaults to empty and nothing breaks.
- No test deserializes an `Upload` JSON body containing a `destination` object.

So although the `new()` signature change is breaking in principle, it has no callers today and the expected compile/test fallout is nil. The safety net is `cargo check --workspace` + `./scripts/test.sh` after regen; if either fails, an in-flight change introduced a call site — fix it by supplying a `bucket_name` value (fixtures: `"my-uploads-bucket"`).

### Validation tooling

- `./scripts/test.sh` — `RUST_LOG=off cargo test --features test`; `--features test` is mandatory (mocks/helpers are gated behind that feature).
- `./scripts/lint.sh` — import linter, `cargo fmt`, machete/diet, audit, clippy. Run `./scripts/update-deps.sh` first to refresh `Cargo.lock`. Clippy warnings inside generated model code are expected and ignorable.
- `./scripts/preflight.sh` — runs `scripts/lint.sh`, `scripts/covgate.sh`, `tools/lint/scripts/lint.sh`, `tools/lint/scripts/covgate.sh` in parallel; clean = all exit 0. Preflight mirrors the CI jobs in `.github/workflows/ci.yml`.

## Plan of Work

M1 — Re-vendor + regenerate. Copy `/home/ben/miru/workbench3/repos/openapi/apis/apps/backend-server/agent/openapi.gen.yaml` over `api/specs/backend/v04.yaml`, then re-apply the vendored header customizations to the new file:

1. `info.version`: `0.0.0` → `v0.5.0-pre` (line 5).
2. Re-insert, after the `license:` block, the two vendored header keys, updated to the new commit:

       x-release-version: v0.5.0-pre
       x-git-commit:
         sha: 8a902ed159a0215eb439a57caa0541404caf968d
         url: https://github.com/mirurobotics/openapi/commit/8a902ed159a0215eb439a57caa0541404caf968d
         message: 'feat(uploads): add bucket_name to upload destination (#199)'

3. Replace both `$API_VERSION$` occurrences with `v0.5.0-pre`.

Then run `./api/regen.sh` and verify only `libs/backend-api/src/models/upload_destination.rs` changed. Confirm no hand-written fallout with `cargo check --workspace` and `./scripts/test.sh`. Commit.

M2 — Validate. Full local validation (`cargo check`, tests, `update-deps` + lint, preflight), then push via the orchestrator's PR flow; CI on the pushed head must be green before the PR leaves draft. Commit only if fmt/covgate/`Cargo.lock` force fixups.

## Concrete Steps

All commands state their working directory. Agent repo root = `/home/ben/miru/workbench3/repos/agent`.

### M1 — Re-vendor + regenerate

    cd /home/ben/miru/workbench3/repos/openapi
    git rev-parse HEAD          # expect 8a902ed159a0215eb439a57caa0541404caf968d
    git status --short          # expect empty

    cp /home/ben/miru/workbench3/repos/openapi/apis/apps/backend-server/agent/openapi.gen.yaml \
       /home/ben/miru/workbench3/repos/agent/api/specs/backend/v04.yaml

Apply the three header edits from Plan of Work (info.version, x-release-version + x-git-commit block with the 8a902ed sha/url/message, both `$API_VERSION$` occurrences). Verify:

    cd /home/ben/miru/workbench3/repos/agent
    grep -c 'v0.5.0-pre' api/specs/backend/v04.yaml     # expect 4 (info.version, x-release-version, enum entry, example)
    grep -n 'API_VERSION\|version: 0.0.0\|a1bcdf37' api/specs/backend/v04.yaml   # expect no output
    grep -n '8a902ed15' api/specs/backend/v04.yaml      # expect 2 lines (sha + url)
    grep -c 'bucket_name' api/specs/backend/v04.yaml    # expect 10 (5 pre-existing in UploadRuleDestination + 5 new in UploadDestination)

Regenerate (needs npx + Java):

    cd /home/ben/miru/workbench3/repos/agent
    ./api/regen.sh              # generator runs twice (backend, device), exits 0

Verify the model delta and absence of fallout:

    git status --short libs/
    # expect exactly one modified file: libs/backend-api/src/models/upload_destination.rs
    # no files added/removed; libs/device-api untouched
    grep -n 'bucket_name' libs/backend-api/src/models/upload_destination.rs
    # expect: serde rename line, pub bucket_name: String, new() param, struct-init line
    cargo check --workspace     # expect: Finished, no errors (no hand-written callers exist)
    ./scripts/test.sh           # expect: all tests pass

Commit M1:

    cd /home/ben/miru/workbench3/repos/agent
    git add api/specs/backend/v04.yaml libs/backend-api/src/models/
    git commit -m "chore(api): sync vendored backend openapi spec to 8a902ed and regen models (UploadDestination.bucket_name)" \
      -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"

If `cargo check` or tests fail (an in-flight call site appeared), fix the call site by supplying `bucket_name` (fixtures use `"my-uploads-bucket".to_string()`), include the fix in the M1 commit, and record it under Surprises & Discoveries.

### M2 — Validate

    cd /home/ben/miru/workbench3/repos/agent
    cargo check --workspace     # expect: clean
    ./scripts/test.sh           # expect: all pass
    ./scripts/update-deps.sh    # refresh Cargo.lock (commit if changed)
    ./scripts/lint.sh           # expect: pass (generated-code clippy warnings ignorable)
    ./scripts/preflight.sh      # expect: clean — all four sub-checks exit 0

If fmt/covgate/`Cargo.lock` produce fixups:

    git add -A
    git commit -m "chore(api): apply lockfile/formatting fixups after spec sync" \
      -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"

Then push via the orchestrator's PR flow and watch CI on the pushed branch head (`gh pr checks` or `gh run watch`). CI must be green before proceeding.

## Validation and Acceptance

1. Spec current: from the agent repo root, `diff <(sed -n '14,$p' api/specs/backend/v04.yaml) <(sed -n '9,$p' /home/ben/miru/workbench3/repos/openapi/apis/apps/backend-server/agent/openapi.gen.yaml)` (skipping each file's info header: vendored lines 1-13, gen.yaml lines 1-8) shows only the two version-substitution lines (`v0.5.0-pre` vs `$API_VERSION$`); the M1 header greps pass; `x-git-commit.sha` is 8a902ed159a0215eb439a57caa0541404caf968d.
2. Model current: `grep -n bucket_name libs/backend-api/src/models/upload_destination.rs` matches (before this change it matches nothing); `UploadDestination::new` takes three parameters (`bucket_id`, `bucket_name`, `object_key`); `git status` shows no other model file changed and `libs/device-api` untouched.
3. Compile/tests: `cargo check --workspace` finishes with no errors; `./scripts/test.sh` reports all tests passed (same count as before the change — no test edits expected).
4. Lint: `./scripts/lint.sh` (after `./scripts/update-deps.sh`) passes; clippy warnings inside generated model code are expected and ignorable.
5. Preflight/CI gate (hard requirement): `./scripts/preflight.sh` exits 0 locally, AND preflight must report CLEAN — meaning CI (`.github/workflows/ci.yml`) is green on the pushed branch head (`feat/upload-destination-bucket-name`) — before the PR leaves draft or the task is reported complete. A red or pending CI head blocks completion.

## Idempotence and Recovery

- The spec copy is a single-file overwrite; re-running `cp` + header edits is idempotent. Rollback: `git checkout -- api/specs/backend/v04.yaml`.
- `./api/regen.sh` clears `libs/*/src/models` before copying, so re-runs are reproducible. If regen output diverges from the expected one-file delta (files added/removed, device-model churn), rollback with `git checkout -- libs/` and re-check the spec edits before retrying.
- If npx/Java are unavailable for regen, stop and fix the toolchain — do not hand-edit generated models (`libs/*/src/models` are generated-only per AGENTS.md).
- One commit per milestone; to amend a milestone before pushing, `git reset --soft HEAD~1`, fix, recommit. Never create or switch branches — stay on `feat/upload-destination-bucket-name`.
