# Data-upload support M0 + M1: vendor the uploads contract and add the upload-rules read path

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** MUST be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench4/repos/agent` (the Rust device agent — the only repo edited) | read-write | M0: re-vendor `api/specs/backend/v04.yaml` from the openapi `main` bundle and regenerate `libs/backend-api` models. M1: add an `http/upload_rules.rs` client, a hand-rolled `models/upload_rule.rs`, a `storage/upload_rules.rs` store wired into `storage/{mod,layout}.rs`, and fetch+cache wiring in `sync/deployments.rs`. Add unit tests for the HTTP client and the store. |
| `/home/ben/miru/workbench4/repos/openapi` | read-only | Source of the vendored bundle. Read `apis/apps/backend-server/agent/openapi.gen.yaml` at commit `4c92b71`. No edits. |

This plan lives in `plans/backlog/` per the agent repo's plan conventions (`plans/{active,archived,backlog,completed}/`). Commit all changes from inside the agent repo's own git context (see workbench `CLAUDE.md`), never from the workbench root.

**This plan covers ONLY M0 (vendor spec + regenerate models) and M1 (upload-rules READ path).** It deliberately ENDS at "the deployed release's upload rules are fetched from `GET /upload_rules` and cached in local state." Nothing consumes the rules beyond storage.

### Explicitly OUT OF SCOPE (deferred to M2–M5)

These are intentionally NOT in this plan and MUST NOT be implemented here:

- File discovery / glob matching on `source.glob` (M2).
- Per-rule poll loop honoring `poll_interval`; stability / finalization detection via `stability_window` (M2).
- Streaming sha256 digest + size computation over candidate files (M3).
- `POST /uploads` (`createUpload`) and the presigned `PUT` to the customer bucket with `required_headers` (M3).
- `POST /uploads/{upload_id}/confirm` (`confirmUpload`) (M3).
- The local uploads ledger / idempotency / retry state (M4).
- `delete_policy` enforcement (deleting local source files) (M5).
- Any background upload worker or `app/run.rs` integration (M5).

Note: M0 regenerates the *models* for the out-of-scope endpoints (`PresignedUpload`, `Upload`, `CreateUploadRequest`, `UploadSource`, `UploadDestination`, `UploadStatus`, etc.). Those generated types will exist but be **unused** after this plan. That is expected and fine — leaving generated-but-unused models in `libs/backend-api` is the normal state of the generated crate and matches how other endpoints are already represented.

## Purpose / Big Picture

The `openapi` repo defines a device-facing data-upload contract (landed on `main` 2026-06-26 via PRs #133/#144, not yet in a tagged agent-spec release). The agent has zero upload code today. The full feature is: discover upload rules from the deployed release → watch the filesystem for matching, quiesced files → mint a presigned URL → `PUT` the file to the customer's bucket → confirm the durable write. Research: `/home/ben/miru/workbench4/research/20260626-agent-data-upload-implementation.md`.

This plan delivers the two lowest-risk, independently-verifiable slices:

- **M0** brings the contract into the agent. The agent does NOT consume the openapi repo directly — it vendors a copy of the agent bundle at `api/specs/backend/v04.yaml` and regenerates Rust *models* (not a client) via `api/regen.sh`. M0 re-vendors that file from the current openapi `main` bundle (commit `4c92b71`) as a **pre-release snapshot** and regenerates `libs/backend-api/src/models/`. This unblocks everything: no upload code can be built or tested without the generated `UploadRuleList` / `BaseUploadRule` / etc. types.
- **M1** adds the upload-rules read path, mirroring how config-instance acquisition already works. Upload rules ship with the deployed release (the `GET /upload_rules` endpoint is scoped to the device's currently-deployed release server-side, via the session token). M1 adds an HTTP client method, a hand-rolled domain model, a persisted store under `/var/lib/miru/resources/`, and wires fetch+cache into the existing sync flow.

**Observable outcome at completion:** after a sync, the deployed release's upload rules are present in the agent's local state at `/var/lib/miru/resources/upload_rules.json`. A unit test feeds an `UploadRuleList` JSON payload to the new HTTP client and asserts it parses into the expected rules; another test round-trips an `UploadRule` through the new store (write → read back identical). `scripts/preflight.sh` prints `Preflight clean`.

## Progress

- [x] (2026-06-26) **M0.1** Verify `openapi-generator-cli` availability — DONE: `openapi-generator-cli` 7.12.0 + `npx` + `java` all present.
- [x] (2026-06-26) **M0.2** Re-vendor `api/specs/backend/v04.yaml` from openapi `4c92b71` bundle, stamped info block (version `v0.5.0-pre`, `x-release-version: v0.5.0-pre`, `x-git-commit.sha` = full `4c92b71...c8a4`). Also substituted the `$API_VERSION$` placeholder (APIVersion enum value + MiruVersion example) → `v0.5.0-pre` (see Surprises).
- [x] (2026-06-26) **M0.3** Ran `api/regen.sh`; 13 new upload model files under `libs/backend-api/src/models/`; `cargo build -p backend-api` succeeds.
- [x] (2026-06-26) **M0.4** `libs/backend-api/src/models/api_version.rs` renders `ApiVersion::API_VERSION` as `v0.5.0-pre`; header consequence recorded in Decision Log.
- [x] (2026-06-26) **M1.1** Added `agent/src/models/upload_rule.rs` (`UploadRule`, `UploadRuleSource`, `UploadRuleDestination`, `DeletePolicy`, `UploadRuleID`) + registered/re-exported in `agent/src/models/mod.rs`. `DeletePolicy` is hand-rolled (not via `impl_status_enum!` — that macro needs an `agent_type` device-API enum that doesn't exist for delete policy).
- [x] (2026-06-26) **M1.2** Added `agent/src/http/upload_rules.rs` (`list` + `list_all`, pagination only) + registered in `agent/src/http/mod.rs`.
- [x] (2026-06-26) **M1.3** Added `agent/src/storage/upload_rules.rs`; wired into `Capacities` (default 1000), `Storage` init/shutdown, and `storage/layout.rs` (`upload_rules.json`).
- [x] (2026-06-26) **M1.4** Wired fetch+cache into `agent/src/sync/deployments.rs` (`pull_upload_rules`, `write_if_absent`) + `sync::deployments::Storage` field + `syncer.rs` call site.
- [x] (2026-06-26) **M1.5** Extended `agent/tests/mocks/http_client.rs` with `Call::ListUploadRules`, `set_list_all_upload_rules`/`set_list_upload_rules_page`, and `GET /upload_rules` routing.
- [x] (2026-06-26) **M1.6** Added tests: `agent/tests/http/upload_rules.rs` (list/list_all/pagination/error), `agent/tests/storage/upload_rules.rs` (round-trip + no-overwrite), `agent/tests/storage/layout.rs` (`upload_rules()` path), `agent/tests/models/upload_rule.rs` (serde harness + `From` + invalid-date fallback + `DeletePolicy`), and updated `agent/tests/storage/caches.rs`. All 24 new tests pass; existing sync/caches suites (95 tests) still green.
- [ ] **V** DEFERRED to the later preflight/lint step (per the implementation brief, the final preflight/lint orchestration is run separately). Verified locally instead: `cargo build -p backend-api -p miru-agent` clean, `cargo test --features test` new+adjacent suites pass, `cargo fmt -p miru-agent -- --check` clean. Covgate (`scripts/covgate.sh`) NOT run here; new model tests were designed to keep `models` at 100% (cover `Display`, both `From` paths, `Deserialize` known/unknown, timestamp fallbacks, `Default`).

Use timestamps when completing steps. Split partially-completed work into "done" / "remaining".

## Surprises & Discoveries

(Add entries as work proceeds. Seed findings from research below.)

- **CORRECTION (2026-06-26, during M0): the outbound `Miru-Version` header is NOT derived from `info.version`.** It is generated from the `components.schemas.APIVersion` enum value (`x-enum-varnames: [API_VERSION]`). In the openapi `main` bundle that enum value is a literal placeholder `$API_VERSION$` that the openapi **release pipeline** substitutes with the real version at release-stamping time (the previously vendored `v0.4` bundle had it already substituted). Regenerating directly from the raw `4c92b71` bundle therefore produced `ApiVersion::API_VERSION => "$API_VERSION$"` — a broken header. To honor this plan's documented intent (outbound header = `v0.5.0-pre`), I reproduced the pipeline substitution by hand: replaced both `$API_VERSION$` occurrences in the vendored spec (the `APIVersion` enum value at the schema and the `MiruVersion` parameter `example:`) with `v0.5.0-pre`, then re-ran regen. `api_version.rs` now renders `v0.5.0-pre`. `agent/src/http/request.rs:151` sets `api_version: backend_api::models::ApiVersion::API_VERSION.to_string()` and `request.rs:165` sends it as the `Miru-Version` header on every backend call, so the outbound header is now `v0.5.0-pre`. The backend must accept `v0.5.0-pre`. See Decision Log.
  Evidence: `libs/backend-api/src/models/api_version.rs`; `api/specs/backend/v04.yaml` (APIVersion schema enum + MiruVersion param example); `agent/src/http/request.rs:136-166`.
- **The original Surprises note (below) was based on a wrong assumption (info.version drives the header).** The raw openapi bundle at `4c92b71` has `info.version: 0.0.0` AND `$API_VERSION$` placeholders for the version enum/example. The vendoring step is a hand-stamp of the `info:` block PLUS the `$API_VERSION$` → `v0.5.0-pre` substitution, not a passthrough.
- The raw openapi bundle at `4c92b71` has `info.version: 0.0.0` (unstamped); the existing vendored `v04.yaml` has `info.version: v0.4`, `x-release-version: v0.4.0`, and an `x-git-commit:` block (`sha`/`url`/`message`). The vendoring step is a hand-stamp of the `info:` block, not a passthrough.
- `UploadRuleList` is `allOf: [PaginatedList, {data: [BaseUploadRule]}]` — the same paginated shape as `DeploymentList` (`total_count`, `has_more`, `data`). The existing `list`/`list_all` + `Page`/`MAX_PAGE_LIMIT` pagination in `agent/src/http/deployments.rs` is a direct template.
- `GET /upload_rules` takes **no** query filters in the contract (only the `MiruVersion` header param). It is scoped to the device's currently-deployed release server-side via the session token. So the client needs no `activity_status`-style filter; pagination only.
- The generated `UploadDeletePolicy` enum already carries forward-compatible-enum treatment (the repo did "forward-compatible enums via generator" work — see `plans/completed/20260515-forward-compatible-enums-via-generator.md` and the `api/templates/rust/model.mustache` template). The hand-rolled domain model can either reuse a small domain enum (mirroring `agent/src/models/status.rs`) or carry the policy as a validated field; see Decision Log.
- `agent/src/models/.covgate` requires **100%** coverage for the models module. The new `models/upload_rule.rs` must be fully covered (including `Default` and the custom `Deserialize` error paths) or covgate fails. `http/.covgate` = 93.9, `storage/.covgate` = 94.83.

## Decision Log

- **Decision: Vendor from openapi `main` (`4c92b71`) NOW as a pre-release snapshot rather than waiting for a tagged agent-bundle release.** Stamp `info.version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, and update `x-git-commit` to `4c92b71`. Rationale: per user decision, unblock agent development immediately; the `-pre` suffix marks the snapshot as not-yet-released so it is never mistaken for a shipped contract. When openapi cuts the real agent-bundle release, re-vendor and drop the `-pre`.
  Date/Author: 2026-06-26 / plan author.
- **Decision: Accept that bumping `info.version` changes the outbound `Miru-Version` header from `v0.4` to `v0.5.0-pre`.** Rationale: the header value is generated from the spec; there is no clean way to bump the contract without bumping the header. RISK: a backend that strictly validates `Miru-Version` against a known allowlist will reject requests from an agent built on this snapshot. Before integration-testing against any real backend, confirm the backend accepts `v0.5.0-pre` (or only build/test this snapshot against a backend that does). For M0/M1, all tests use the mock HTTP client and never send a real header, so unit/preflight validation is unaffected. Record the integration-test gating in Outcomes.
  Date/Author: 2026-06-26 / plan author.
- **Decision: Mirror config-instance acquisition for the read path** — hand-rolled domain model (`From<backend_client::BaseUploadRule>` + custom `Deserialize` with `deserialize_error!` for timestamps + `Default` with `unknown-<uuid>` ids), an actor-per-store `FileCache`, and `write_if_absent` during sync. Rationale: upload rules are immutable and fleet-uniform (like releases/config-instance metadata); `write_if_absent` avoids per-sync I/O for already-cached rules, exactly as `store_expanded_release` does. Keeping a hand-rolled domain model isolates the agent from generated-type churn, consistent with `models/config_instance.rs` and `models/release.rs`.
  Date/Author: 2026-06-26 / plan author.
- **Decision (to be finalized during M1.1): represent `delete_policy` as a small domain enum** (`DeletePolicy { Never, AfterUpload }`) in `models/upload_rule.rs`, mapped from the generated `UploadDeletePolicy` with an unknown→safe-default (`Never`) fallback + log, mirroring `agent/src/models/status.rs`. Rationale: `Never` is the safe default (never deletes local data) for an unrecognized future policy value; this matches the forward-compatible-enum philosophy already in the repo. If the implementer finds the status-enum macro is the cleaner reuse, prefer it; record the final choice here.
  Date/Author: 2026-06-26 / plan author.

## Outcomes & Retrospective

**M0 + M1 implemented (2026-06-26).** `openapi-generator-cli` 7.12.0 was available, so M0 regen ran successfully (no hand-written generated models).

- **Generated upload model files** (13, all under `libs/backend-api/src/models/`): `base_upload.rs`, `base_upload_rule.rs`, `create_upload_request.rs`, `presigned_upload.rs`, `upload.rs`, `upload_delete_policy.rs`, `upload_destination.rs`, `upload_required_headers.rs`, `upload_rule_destination.rs`, `upload_rule_list.rs`, `upload_rule_source.rs`, `upload_source.rs`, `upload_status.rs`. The POST/confirm-related ones (`Upload`, `PresignedUpload`, `CreateUploadRequest`, `UploadSource`, `UploadDestination`, `UploadRequiredHeaders`, `UploadStatus`, `BaseUpload`) are generated-but-unused (expected). Regen also picked up unrelated additive changes already on openapi `main` since v0.4 (notably `DeviceStatus::archived` + doc/optional-field updates).
- **`Miru-Version` header:** now `v0.5.0-pre` (see the corrected Surprises entry — it derives from the `APIVersion` schema enum, not `info.version`; the `$API_VERSION$` placeholder was substituted by hand to reproduce the openapi release pipeline). **OPEN / integration-test gate:** whether the real backend accepts `v0.5.0-pre` is NOT verified here (all M0/M1 tests use the mock client and send no real header). Confirm backend acceptance before integration-testing an agent built from this snapshot.
- **Covgate numbers:** NOT measured in this pass (`scripts/covgate.sh`/preflight deferred to the separate validation step per the brief). Thresholds to meet remain `models`=100, `http`=93.9, `storage`=94.83. New model tests were written to keep `models` at 100% (every branch of `upload_rule.rs` is exercised).
- **Local verification done:** `cargo build -p backend-api -p miru-agent` clean; new test suites (24 tests across http/models/storage) pass; existing sync+caches suites (95 tests) unaffected by the added `pull_upload_rules` sync step; `cargo fmt -p miru-agent -- --check` clean.
- **Commits:** (1) `feat(api): vendor uploads contract from openapi 4c92b71 as v0.5.0-pre` (spec + regenerated models); (2) `feat(uploads): add upload-rules read path (M1)` (source + mock + fixture compile-fixes); (3) tests commit.

## Context and Orientation

The agent is a Rust workspace rooted at `repos/agent/Cargo.toml`: `agent/` (binary crate — all logic), `libs/backend-api/` and `libs/device-api/` (OpenAPI-generated models; do NOT hand-edit — regenerate via `api/regen.sh`). Repo conventions: `repos/agent/AGENTS.md` (import ordering, `thiserror` errors, `#[cfg(feature = "test")]` gating, `scripts/test.sh` with `--features test`, per-module `.covgate`).

### Spec vendoring & codegen (M0 surface)

- `api/specs/backend/v04.yaml` — the vendored agent bundle. Stamped `info:` block (lines 1–13 today): `version: v0.4`, `license`, `x-release-version: v0.4.0`, `x-git-commit: {sha,url,message}`.
- `api/Makefile` — `make gen` runs `npx --yes @openapitools/openapi-generator-cli generate -i specs/backend/v04.yaml -g rust -t templates/rust -o codegen/backend --additional-properties=packageName=backend-api` (and the device spec analogue).
- `api/regen.sh` — runs `make gen`, then wipes `libs/backend-api/src/models/*` and `libs/device-api/src/models/*` and copies the freshly generated models in.
- `api/templates/rust/model.mustache` — custom model template (recently updated for forward-compatible enums). Codegen uses it; do not bypass it.
- `api/openapitools.json` — pins the generator CLI version.

The bundle at openapi `4c92b71` adds three device endpoints (`GET /upload_rules` → `UploadRuleList`; `POST /uploads` → `PresignedUpload`; `POST /uploads/{upload_id}/confirm` → `Upload`) and schemas/enums: `BaseUploadRule`, `UploadRuleSource`, `UploadRuleDestination`, `UploadRuleList`, `UploadDeletePolicy`, `CreateUploadRequest`, `UploadSource`, `UploadDestination`, `PresignedUpload`, `UploadRequiredHeaders`, `BaseUpload`, `Upload`, `UploadStatus`.

`BaseUploadRule` fields (all required): `object` (const `"upload_rule"`), `id`, `upload_collection_id`, `upload_collection_name`, `digest` (`sha256:...`), `source` (`UploadRuleSource`: `glob`, `poll_interval`, `stability_window`), `destination` (`UploadRuleDestination`: `bucket_id`, `bucket_name`, `path`, `delete_policy`), `created_at`, `updated_at` (both `date-time`).

### Read-path patterns to MIRROR (M1 surface)

- **HTTP client**: `agent/src/http/deployments.rs` — `ListParams`/`ListAllParams` builders, `list` (single page via `super::client::fetch`), `list_all` (loop over `Page`/`MAX_PAGE_LIMIT` until `!has_more`). `agent/src/http/config_instances.rs` is the minimal single-call template. `agent/src/http/query.rs` provides `Page`, `MAX_PAGE_LIMIT`, `QueryParams::paginate`. Register new module in `agent/src/http/mod.rs`. Fetch+parse via `agent/src/http/client.rs::fetch` (uses `response::parse_json`). Retries via `crate::http::with_retry`.
- **Domain model**: `agent/src/models/config_instance.rs` and `agent/src/models/release.rs` — `From<backend_client::T>` + a custom `Deserialize` using the `deserialize_error!` macro to fall back to defaults for bad timestamps, plus a `Default` impl with `unknown-<uuid>` placeholder ids. `agent/src/models/status.rs` shows the domain-enum + `impl_status_enum!` mapping pattern for `delete_policy`. Register exports in `agent/src/models/mod.rs`.
- **Storage**: `agent/src/storage/releases.rs` (`pub type Releases = cache::FileCache<models::ReleaseID, models::Release>;`) is the minimal actor-per-store template; `agent/src/storage/deployments.rs` adds an `is_dirty` helper (not needed here — rules are immutable). The `cache::FileCache` is spawned via `FileCache::spawn(buffer, file, capacity)` and exposes `write_if_absent`, `read_optional`, `entries`, `shutdown` (see `agent/src/cache/file.rs`, `agent/src/cache/mod.rs`). Wire the new store into:
  - `agent/src/storage/mod.rs`: add `pub mod upload_rules;`, a `Capacities.upload_rules` field (default `1000`, like `releases`), an `Arc<UploadRules>` field on `Storage`, spawn it in `Storage::init`, add its handle to the shutdown `join_all`, and shut it down in `Storage::shutdown`.
  - `agent/src/storage/layout.rs`: add `pub fn upload_rules(&self) -> filesys::File { self.resources().file("upload_rules.json") }`.
- **Sync wiring**: `agent/src/sync/deployments.rs` — `store_expanded_release` (uses `write_if_absent` for immutable release/git_commit data) is the closest analogue. `pull_content_for_cfg_insts` shows the `with_retry` + `error`-collecting pattern. The `sync::deployments::Storage<'a>` struct (lines 27–44) is constructed in `agent/src/sync/syncer.rs::sync_impl` (lines 235–250) from `storage::Storage`. Add the upload-rules store to both.
- **Test mock**: `agent/tests/mocks/http_client.rs` defines `MockClient`, a `Call` enum, `CapturedRequest`, and per-endpoint setters (`set_list_all_deployments`, `set_list_deployments_page`, `set_get_config_instance_content`, etc.). Add a `Call::ListUploadRules` variant and `set_list_all_upload_rules` / `set_list_upload_rules_page` setters mirroring the deployments ones.

### Validation tooling

- `scripts/preflight.sh` runs, in parallel: `scripts/lint.sh`, `scripts/covgate.sh` (tests + coverage gate), and the `tools/lint` lint + covgate. Prints `Preflight clean` on success, `Preflight FAILED (...)` and exits non-zero otherwise.
- `scripts/covgate.sh` runs `cargo test --features test` with coverage and enforces each module's `.covgate` minimum. Relevant thresholds: `agent/src/models/.covgate` = **100**, `agent/src/http/.covgate` = **93.9**, `agent/src/storage/.covgate` = **94.83**.
- `scripts/test.sh` = `RUST_LOG=off cargo test --features test`. The `--features test` flag is REQUIRED (test helpers/mocks are behind `#[cfg(feature = "test")]`).
- `scripts/lint.sh` runs the custom import linter, `cargo fmt`, `cargo machete`/diet unused-dep checks, security audit, and clippy. Run `scripts/update-deps.sh` first to refresh `Cargo.lock`.

## Plan of Work

### M0 — Vendor spec + regenerate models

#### S0. Preflight the generator (gate)

Confirm `openapi-generator-cli` can run before changing anything:

    cd /home/ben/miru/workbench4/repos/agent/api
    npx --yes @openapitools/openapi-generator-cli version

If this fails (no network / npx / Java unavailable), **STOP and report**. Do NOT hand-write or hand-edit generated models in `libs/backend-api/src/models/` — the generated crate is regenerated wholesale by `regen.sh`, and hand-edits will be silently destroyed on the next regen and will diverge from the contract. Reporting the blocker is the correct outcome.

#### S1. Re-vendor `api/specs/backend/v04.yaml`

1. Extract the source bundle (read-only) from openapi:

       cd /home/ben/miru/workbench4/repos/openapi
       git show 4c92b71:apis/apps/backend-server/agent/openapi.gen.yaml > /tmp/claude-1000/-home-ben-miru-workbench4/0b7d4b5d-b8dc-472f-a9dd-ca87bdaacaa6/scratchpad/agent-bundle-4c92b71.yaml

2. Capture the commit metadata for the `x-git-commit` block:

       git -C /home/ben/miru/workbench4/repos/openapi show -s --format='%H%n%s' 4c92b71

3. Diff the source bundle's `info:` block against the current `repos/agent/api/specs/backend/v04.yaml` `info:` block (lines 1–13) to define the exact stamp. The only differences to introduce are inside `info:`:
   - `version: 0.0.0` → `version: v0.5.0-pre`
   - add `x-release-version: v0.5.0-pre`
   - add the `x-git-commit:` block with `sha: <4c92b71 full sha>`, `url: https://github.com/mirurobotics/openapi/commit/<full sha>`, and `message:` set to the commit subject (`feat(uploads): add upload_collection parent resource, re-parent upload rules (#144)`). Match the YAML field shape used by the existing block (the existing block uses a literal/folded multi-line `message:`; a single-line message is acceptable — keep the three keys `sha`/`url`/`message`).
   - The `license:` block already matches between both files; leave it.
   - Leave everything outside `info:` (the `servers:`, `security:`, `paths:`, `components:` from the `4c92b71` bundle) exactly as the source bundle has it — that is the new contract surface being vendored in.
4. Write the stamped result to `repos/agent/api/specs/backend/v04.yaml` (overwrite). Sanity-check:

       cd /home/ben/miru/workbench4/repos/agent
       grep -nE '^  version:|x-release-version:|sha:' api/specs/backend/v04.yaml | head
       grep -nE 'upload_rules|UploadRuleList|BaseUploadRule|PresignedUpload' api/specs/backend/v04.yaml | head

   Expect the version/release lines to show `v0.5.0-pre` and the upload references to be present.

#### S2. Regenerate models

    cd /home/ben/miru/workbench4/repos/agent
    api/regen.sh
    cargo build -p backend-api 2>&1 | tail -20

Confirm new model files exist (names follow the generator's snake_case convention; verify actual filenames after regen):

    ls libs/backend-api/src/models | grep -iE 'upload'

Expect files for `base_upload_rule`, `upload_rule_source`, `upload_rule_destination`, `upload_rule_list`, `upload_delete_policy`, `create_upload_request`, `upload_source`, `upload_destination`, `presigned_upload`, `upload_required_headers`, `base_upload`, `upload`, `upload_status` (exact set/names per generator output). The POST/confirm-related models are expected and unused — leave them.

#### S3. Confirm the `Miru-Version` consequence

    grep -n 'rename' libs/backend-api/src/models/api_version.rs

Confirm `ApiVersion::API_VERSION` now renders `v0.5.0-pre`. Record in the Decision Log / Outcomes that the agent's outbound `Miru-Version` header is now `v0.5.0-pre`, and the integration-test gating (backend must accept it). No code change is needed in `agent/src/http/request.rs` — it already reads the generated constant.

Commit M0 as its own commit (regenerated `libs/backend-api` + the vendored spec) before starting M1, so the generated-code churn is isolated from the hand-written M1 changes.

### M1 — Upload-rules READ path

#### S4. Domain model — `agent/src/models/upload_rule.rs`

Mirror `models/config_instance.rs` / `models/release.rs`. Define:

- `pub type UploadRuleID = String;`
- `pub struct UploadRuleSource { glob: String, poll_interval: String, stability_window: String }` (keep durations as validated `String` for M1 — parsing into `Duration` is an M2 concern; do not parse here).
- `pub struct UploadRuleDestination { bucket_id: String, bucket_name: String, path: String, delete_policy: DeletePolicy }`.
- `pub enum DeletePolicy { Never, AfterUpload }` with `Default = Never`, `From<&backend_client::UploadDeletePolicy>` (unknown → `Never` + log), `Display`, and `Serialize`/custom `Deserialize` mirroring `models/status.rs` (or reuse `impl_status_enum!` if cleaner — finalize per Decision Log).
- `pub struct UploadRule { id, upload_collection_id, upload_collection_name, digest, source: UploadRuleSource, destination: UploadRuleDestination, created_at: DateTime<Utc>, updated_at: DateTime<Utc> }` deriving `Clone, Debug, PartialEq, Serialize`.
- `impl Default for UploadRule` with `unknown-<uuid>` placeholder ids and `UNIX_EPOCH` timestamps (mirror `ConfigInstance::default`).
- `impl From<backend_client::BaseUploadRule> for UploadRule` mapping fields and parsing `created_at`/`updated_at` with `.parse::<DateTime<Utc>>().unwrap_or_else(|e| { error!(...); UNIX_EPOCH })`.
- `impl<'de> Deserialize<'de> for UploadRule` using the `deserialize_error!` macro for missing/invalid timestamps (mirror `ConfigInstance`/`Release`).

Register in `agent/src/models/mod.rs`: `pub mod upload_rule;` and `pub use self::upload_rule::{UploadRule, UploadRuleID};` (plus `DeletePolicy` and the nested structs as needed).

Because `models/.covgate` = 100%, include unit tests in this file (or in `agent/tests/models/`) covering `From`, the custom `Deserialize` happy path, the timestamp-fallback path, `Default`, and the `DeletePolicy` unknown→`Never` path.

#### S5. HTTP client — `agent/src/http/upload_rules.rs`

Mirror `http/deployments.rs` (pagination) but with NO filters:

    // internal crates
    use crate::http::{
        errors::HTTPErr,
        query::{Page, QueryParams, MAX_PAGE_LIMIT},
        request, ClientI,
    };
    use backend_api::models::{BaseUploadRule, UploadRuleList};

    pub struct ListParams<'a> { pub pagination: &'a Page, pub token: &'a str }
    pub struct ListAllParams<'a> { pub token: &'a str }

    pub async fn list(client: &impl ClientI, params: ListParams<'_>) -> Result<UploadRuleList, HTTPErr> {
        let qp = QueryParams::new().paginate(params.pagination);
        let url = format!("{}/upload_rules", client.base_url());
        let request = request::Params::get(&url).with_query(qp).with_token(params.token);
        super::client::fetch(client, request).await
    }

    pub async fn list_all(client: &impl ClientI, params: ListAllParams<'_>) -> Result<Vec<BaseUploadRule>, HTTPErr> {
        // loop over Page { limit: MAX_PAGE_LIMIT, offset } until !page.has_more, extending page.data
    }

(Confirm the actual generated field/type names — `UploadRuleList.data: Vec<BaseUploadRule>`, `.has_more` — after M0 regen, and adjust.) Register `pub mod upload_rules;` in `agent/src/http/mod.rs`.

#### S6. Storage — `agent/src/storage/upload_rules.rs`

Mirror `storage/releases.rs`:

    // internal crates
    use crate::cache;
    use crate::models;

    pub type UploadRules = cache::FileCache<models::UploadRuleID, models::UploadRule>;

Wire-up:
- `agent/src/storage/layout.rs`: add `pub fn upload_rules(&self) -> filesys::File { self.resources().file("upload_rules.json") }`.
- `agent/src/storage/mod.rs`: add `pub mod upload_rules;`; `pub use self::upload_rules::UploadRules;`; add `upload_rules: usize` to `Capacities` (default `1000`) and update the `Default` impl + the `caches.rs` capacity test (S9); add `pub upload_rules: Arc<UploadRules>` to `Storage`; spawn it in `Storage::init` (`UploadRules::spawn(64, layout.upload_rules(), capacities.upload_rules).await?`), add its handle to the `join_all` shutdown vec, and add `self.upload_rules.shutdown().await?;` in `Storage::shutdown`.

#### S7. Sync wiring — `agent/src/sync/deployments.rs` + `syncer.rs`

- Add `pub upload_rules: &'a storage::UploadRules` to `sync::deployments::Storage<'a>`.
- In `agent/src/sync/syncer.rs::sync_impl`, add `upload_rules: storage_ref.upload_rules.as_ref()` when constructing `deployments::Storage` (alongside `deployments`, `cfg_insts`, `releases`, `git_commits`).
- Add a `pull_upload_rules` step to `sync::deployments::sync` (call it after `pull_deployments`, collecting errors into the existing `errors` vec like the other pull steps):

      async fn pull_upload_rules<'a, HTTPClientT: http::ClientI>(
          http_client: &HTTPClientT, storage: &Storage<'a>, token: &str,
      ) -> Result<(), SyncErr> {
          let rules = http::with_retry(|| {
              http::upload_rules::list_all(http_client, http::upload_rules::ListAllParams { token })
          }).await?;
          for backend_rule in rules {
              let rule: models::UploadRule = backend_rule.into();
              let id = rule.id.clone();
              storage.upload_rules.write_if_absent(id, rule, |_, _| false).await?;
          }
          Ok(())
      }

  Rationale for `write_if_absent`: rules are immutable (digest-deduped) like releases — mirror `store_expanded_release`. (If a future milestone needs to detect rule removal/rotation, that is M2+; M1 only adds/caches.)

Milestone ENDS here: rules are fetched and cached. No consumer reads `storage.upload_rules` beyond this.

#### S8. Test mock — `agent/tests/mocks/http_client.rs`

Add a `Call::ListUploadRules` variant and `set_list_all_upload_rules` / `set_list_upload_rules_page` setters mirroring `set_list_all_deployments` / `set_list_deployments_page`, and route `GET /upload_rules` in the mock's request handling to record `Call::ListUploadRules` and return the configured `UploadRuleList`.

## Test Steps

Tests use `--features test` (run via `scripts/test.sh`). Test files mirror `agent/src/` under `agent/tests/`. Use `#[serial]` only for tests binding fixed OS paths (none here — storage tests use `filesys::Dir::create_temp_dir`).

### T1. HTTP client tests — `agent/tests/http/upload_rules.rs`

Mirror `agent/tests/http/deployments.rs`. Register the module in `agent/tests/http/mod.rs`. Cover:

- `list::success` — set the mock to return a one-rule `UploadRuleList { total_count: Some(1), data: vec![BaseUploadRule { id: "upl_rule_1", ..default }], .. }`; call `upload_rules::list` with `Page::default()`; assert the parsed result equals the expected `UploadRuleList` and that the captured request is `GET /upload_rules` with `query: [("limit","10"),("offset","0")]`, no body, token attached, `Call::ListUploadRules` count == 1. **This is the `UploadRuleList` response-parsing test required by the brief.**
- `list_all::single_page` — `has_more: false`, one rule; assert `list_all` returns the one rule and queries `limit=100&offset=0`.
- `list_all::multi_page_pagination` — first page `has_more: true`/`upl_rule_1`, second `has_more: false`/`upl_rule_2`; assert both rules returned in order and two requests captured with offsets `0` then `100` (mirror the deployments multi-page test).
- `list_all::empty_result` — no rules; assert empty vec and one call.
- `list::error_propagates` / `list_all::error_propagates` — mock returns `HTTPErr::MockErr`; assert the error propagates.

### T2. Storage round-trip test

Add a storage test (e.g. extend `agent/tests/storage/` with an `upload_rules.rs` module registered in `agent/tests/storage/mod.rs`, or add to an existing store test file) that:
- Creates a temp layout (`filesys::Dir::create_temp_dir("testing")`, `Layout::new`), inits `Storage`, writes an `UploadRule` via `storage.upload_rules.write_if_absent(...)`, reads it back via `read_optional`, and asserts the round-tripped value equals the written value (write → read identical). **This is the round-trip persistence test required by the brief.**
- Also assert `write_if_absent` does NOT overwrite an existing id (write rule A under id `x`, attempt write rule B under id `x`, read back A).

### T3. Capacities + layout tests

- Update `agent/tests/storage/caches.rs::default_capacities::default` to include `upload_rules: 1000` in the expected `Capacities`.
- Extend `agent/tests/storage/layout.rs` to assert `layout.upload_rules()` resolves to `<root>/var/lib/miru/resources/upload_rules.json` (mirror the existing `deployments()`/`releases()` layout assertions).

### T4. Model conversion test

In `agent/src/models/upload_rule.rs` (or `agent/tests/models/`), assert `From<BaseUploadRule>` maps every field, `created_at`/`updated_at` parse correctly, an invalid timestamp falls back to `UNIX_EPOCH` (covering the `deserialize_error!` path so `models` stays at 100% covgate), and `DeletePolicy` maps `never`→`Never`, `after_upload`→`AfterUpload`, and an unknown value→`Never`.

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

**Coverage gate (`scripts/covgate.sh`, invoked by preflight) imposes a per-module minimum via `.covgate` files and WILL fail the build if coverage drops.** The modules this plan touches: `agent/src/models/.covgate` = **100** (the new `upload_rule.rs` must be fully covered — include the timestamp-fallback and `DeletePolicy`-unknown paths), `agent/src/http/.covgate` = **93.9**, `agent/src/storage/.covgate` = **94.83**. If adding the new files lowers a module below its threshold, add tests (do not lower the threshold). Generated `libs/backend-api` code is not agent-source and its clippy warnings are expected/ignored.

Acceptance (human-verifiable):

1. M0: `api/specs/backend/v04.yaml` shows `version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, `x-git-commit.sha` = `4c92b71`'s full sha, and contains `upload_rules`/`UploadRuleList`/`BaseUploadRule`. `libs/backend-api/src/models/` contains the regenerated upload models and `cargo build -p backend-api` succeeds. `api_version.rs` renders `v0.5.0-pre`.
2. M1: `cargo test --features test` runs the new HTTP tests (T1) — `UploadRuleList` parses and pagination works — and the storage round-trip test (T2) passes (write → read identical).
3. After this plan, `storage.upload_rules` is populated during sync and persisted at `/var/lib/miru/resources/upload_rules.json`; nothing consumes the rules further (scope boundary respected).
4. `scripts/preflight.sh` prints `Preflight clean`.

## Idempotence and Recovery

- **M0 is regenerative.** Re-running `api/regen.sh` reproduces `libs/backend-api/src/models/` from the vendored spec; it is safe to run repeatedly. If regen produces unexpected output, fix the spec stamp and re-run — never hand-edit generated files.
- If `openapi-generator-cli` is unavailable, the whole plan is blocked at S0; STOP and report (do not hand-write models).
- M1 source edits are additive (new files + new fields/wiring). If `cargo build -p miru-agent` fails after S6/S7 with a missing-field error on `sync::deployments::Storage` or `storage::Storage`, the wiring is incomplete — finish all sites listed in S6/S7 (init, shutdown handle vec, shutdown call, sync construction, `Capacities`).
- Rollback: `git -C /home/ben/miru/workbench4/repos/agent checkout -- api/specs/backend/v04.yaml libs/backend-api && git checkout -- agent/src/{models,http,storage,sync} agent/tests` restores pre-change state (only if abandoning).
- Commit M0 (spec + regenerated models) separately from M1 (hand-written code + tests) so the generated churn is isolated and reviewable.

---

Change note (2026-06-26): Initial draft. Covers M0 (vendor openapi `4c92b71` bundle as `v0.5.0-pre` pre-release snapshot, regenerate `libs/backend-api`) and M1 (upload-rules read path: `http/upload_rules.rs`, `models/upload_rule.rs`, `storage/upload_rules.rs`, sync fetch+cache). Key risk flagged: bumping the vendored `info.version` changes the agent's outbound `Miru-Version` header to `v0.5.0-pre` (generated `ApiVersion`), which the backend must accept before integration testing.
