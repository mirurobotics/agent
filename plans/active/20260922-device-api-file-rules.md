# Serve Device API v0.2.2: `Release.file_rule_ids` and `GET /file_rules/{file_rule_id}`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/user/agent` (`mirurobotics/agent`) | read-write | Re-vendor the Device API spec, regenerate `libs/device-api`, serve the new field and endpoint, cache file-rule bodies on release cache misses, tests, docs. |
| `/home/user/openapi` (`mirurobotics/openapi`, `main` at `81f3a3a`) | read-only | Source of the device-server bundle and of the release stamping code (`tools/release/spec.py`). Nothing is written there. |

This plan lives in the agent repo because every change is made there. Work on the already checked-out branch `claude/sleepy-cannon-bmq3zz` (level with `origin/main`); do not create or switch branches and do not push from the milestones. Run git from `/home/user/agent`. Stage files by explicit path (never `git add .` or `git add -A`). Use Conventional Commits; every commit message ends with the two attribution trailer lines (`Co-Authored-By: …` and `Claude-Session: …`) supplied by the orchestrator, and no model identifiers appear anywhere else. Out of scope: the docs repo, the Python device SDK, and cutting the `device/v0.2.2` tag in openapi.

## Purpose / Big Picture

On-device applications query the Miru agent over its local Unix socket through the Device API (routes under `/v0.2/`). mirurobotics/openapi#291 (openapi `main` commit `81f3a3a`) added a required `file_rule_ids: string[]` to the `Release` response and a new `GET /file_rules/{file_rule_id}` (operationId `getFileRule`) returning a `BaseFileRule`. A customer migrating step by step off a legacy file archiver needs an on-device app to decide whether Miru uploads are active:

    rel = GET /v0.2/releases/current
    uploads_active = any(GET /v0.2/file_rules/{id}.upload is not None for id in rel.file_rule_ids)

A file rule uploads files only when it has an `upload` block; retention-only rules omit it. After this change, `GET /v0.2/releases/current` and `GET /v0.2/releases/{id}` include `file_rule_ids`. `GET /v0.2/file_rules/{id}` returns the rule from the agent's local cache, or HTTP 404 with error code `resource_not_found` when the rule is not cached. `GET /v0.2/version` also reports `api_release_version: "v0.2.2"`.

## Progress

- [x] M1: Vendor device spec v0.2.2, regenerate `libs/device-api`, populate `Release.file_rule_ids` and `VersionResponse.api_release_version` (commit). Done 2026-09-22: render and regen matched the expected file lists exactly (spec +410/-31; 6 modified and 5 new device models; backend models unchanged); `server::` and `version::` tests pass (68).
- [x] M2: `file_rule` service, `BaseFileRule` conversion, handler, route, tests, docs (commit). Done 2026-09-22: 10 new tests pass (2 service, 5 conversion, 3 route); custom linter clean on `agent/src` and `agent/tests`.
- [x] M3: `release::get` caches file-rule bodies on backend fallback (commit). Done 2026-09-22: 12 `get` and 8 `get_current` test call sites updated; 3 new tests pass; `services::release` and `server::handlers` pass (39).
- [x] M4: Local validation; push; preflight reports CLEAN (CI green on the pushed head). Done 2026-09-22: fmt, clippy `-D warnings`, the custom linter, and the full suite passed locally (apart from the root-only permission tests below). Draft PR mirurobotics/agent#261; CI run 35782899758 on `c0e39f9` passed `lint`, `test` (covgate), `windows-check`, and `tools` in the first CI round.

## Surprises & Discoveries

- Local validation ran as root (uid 0). Seven existing permission-denial tests fail there, because chmod 555/000 does not block root: `deploy::filesys::tests::{rollback_returns_errors_when_restores_fail_synthetic, remove_backups_continues_when_delete_fails}` (lib), and in `mod` `deploy::apply::deploy_errors::config_instance_write_permission_denied`, `deploy::apply::remove_action::remove_io_error_permission_denied`, `filesys::files::copy_to::unreadable_source_returns_copy_file_err_permission_denied`, and `sync::deployments::apply_error_isolation::{apply_error_still_pushes_retrying_status, apply_error_does_not_fail_sync}`. None touch code changed here; CI runs as a non-root user. All other tests pass (lib 462, `mod` 1577).
- The local `main` ref was two Dependabot commits behind `origin/main`. The branch is based on `origin/main` (`f372c9c`), so compare against `origin/main`.
- `cargo-llvm-cov`, `cargo-machete`, `cargo-audit`, and `cargo-diet` are not installed locally, so `covgate.sh` and the full `lint.sh` run only in CI. The custom linter (imports, funclen, field asserts), `cargo fmt --check`, and `cargo clippy --all-targets --all-features -D warnings` ran locally and are clean.

## Decision Log

- Each milestone commit includes its tests, as the plan's milestones specify, so `$implement` steps 3–4 (test-plan adjustment and test implementation) produced no separate commit. No deviations from the plan's test list were needed.
- `cache_miss_rule_cache_failure_does_not_cache_release` shuts the `FileRules` store down before `get`, so every `write_if_absent` fails. This exercises the logged-error branch of `cache_file_rules` and keeps `services/release` at or above its `.covgate`.
- `converts_retention_only_rule_omits_require_upload` stores `require_upload: true` on a rule without `upload`. That proves the conversion omits the field based on whether an upload block exists, not on the stored value.
- Living-section updates to this plan ride along in each milestone commit instead of separate commits.

## Outcomes & Retrospective

- The agent now serves Device API v0.2.2. `Release` responses carry `file_rule_ids`, `GET /v0.2/file_rules/{file_rule_id}` serves cached rules (404 `resource_not_found` otherwise), and `GET /v0.2/version` reports `api_release_version`. The client flow (`/releases/current`, then `/file_rules/{id}` for each id, then check `upload`) is covered end to end through the router.
- `release::get` no longer drops rule bodies on a backend fallback, and it caches the release only after every rule body is cached.
- Each milestone was one commit, plus one comment-wording refinement (`docs(server)`). The plan needed no deviations. CI was green on the first round and every `.covgate` passed, including the new `services/file_rule` gate at 100.
- Follow-ups (out of scope): tag `device/v0.2.2` in openapi and re-vendor if the asset differs beyond `built_at`; document the field and endpoint in the docs repo; regenerate the Python device SDK.

## Context and Orientation

### Agent server, services, and caches

The workspace's binary crate is `miru-agent` in `agent/`. It serves the Device API with axum under `agent/src/server/`:

- `routes.rs` registers each route under `/{api_version}`, where `api_version` is `device_api::models::ApiVersion::API_VERSION` (`v0.2`).
- `handlers.rs` has one async fn per route. Each wraps a service call in `handle(...)`, which returns `Ok` values as 200 JSON. Errors become a `device_server::ErrorResponse` carrying the error's `http_status()` and `code()`.
- `response.rs` holds the `From<&models::X> for device_server::X` conversions. `device_server` is the alias for `device_api::models`.

Services live in `agent/src/services/<resource>/`; each `mod.rs` is `mod get; pub use get::*;`. `git_commit/get.rs` and `release/get.rs` read the local cache first and fall back to the backend on a miss, via the `BackendFetcher` trait in `services/backend.rs`.

Local caches ("stores") are `cache::FileCache<K, V>` actors defined in `agent/src/disk/`. They are bundled in `disk::Storage` (`storage.releases`, `storage.file_rules`, …), which handlers reach as `state.storage.*`. `read(key)` returns `CacheErr::CacheElementNotFound` on a miss, and that error reports `code()` `resource_not_found` and `http_status()` 404. `ServiceErr` implements `From<cache::CacheErr>`, so `?` in a service turns a miss into a 404 response; `GET /deployments/current` 404s the same way. `write_if_absent(key, value, |_, _| false)` writes only when the key is absent.

File rules are defined in `agent/src/models/file_rule.rs`:

    FileRule { id, name, digest: String,
               source: FileRuleSource { glob: String, stability_window_secs: i64 },
               upload: Option<FileRuleUpload { upload_collection_id, upload_collection_name,
                                               bucket_id, bucket_name, path: String }>,
               retention: Option<FileRuleRetention { require_upload: bool, ttl_secs: u64 }>,
               created_at, updated_at: DateTime<Utc> }

`From<backend_client::BaseFileRule>` converts the backend's `require_upload: Option<bool>` with `unwrap_or(false)` and clamps `ttl_secs: i64` to `>= 0` as `u64`. `models::Release` already has `file_rule_ids: Vec<FileRuleID>`. The store is `disk::FileRules` (`agent/src/disk/file_rules.rs`). Today only the syncer writes rule bodies: `store_expanded_release` in `agent/src/sync/deployments.rs` calls `write_if_absent` for the release and each rule of a deployment fetched with `release.file_rules` expanded. The backend API has no standalone file-rule endpoint, so the new route has no backend fallback.

There is a gap in `agent/src/services/release/get.rs`. On a cache miss it fetches the release with `file_rules` expanded (`HttpBackend::fetch_release` passes `&["file_rules"]`), but it caches only the release and its ids and discards the rule bodies. `/file_rules/{id}` would then 404 for those ids.

### Vendored Device API spec and code generation

`api/specs/device/v02.yaml` is a vendored copy of openapi's `apis/apps/device-server/openapi.gen.yaml` as stamped by openapi's release tooling, not the raw bundle. `tools/release/spec.py::render` does the stamping:

- It substitutes the `$API_VERSION$`, `$RELEASE_VERSION$`, and `$API_GIT_COMMIT$` placeholders.
- It sets `info.version`, `info.x-release-version`, `info.x-git-commit` (`sha`, `url`, `message`, `author`, `branch`, `dirty`), and `info.x-build.built_at`.
- It re-dumps the result with PyYAML, so the formatting differs from the raw bundle.

The current file is `v0.2.1` (openapi `10dc72e`). Rendering `10dc72e` with that code reproduces the file byte for byte, except for one agent-local hand edit: the `200` description of `GET /events` (SSE) was extended to document the heartbeat comment that `agent/src/server/sse.rs` sends. The render script below re-applies that edit.

The `device/v0.2.2` tag has not been cut yet, so no release asset exists. This plan renders openapi `main` `81f3a3a05a8a3c4e6502304248e73aa7683895b8` with `api_version=v0.2`, `release_version=v0.2.2`, `branch=HEAD`, and `dirty=false`. Once `device/v0.2.2` is tagged on that commit, the release workflow should produce the same file apart from `x-build.built_at` and the SSE hand edit. The M1 commit body records this provenance.

Rendering both versions shows the spec changes from v0.2.1 to v0.2.2:

- New path `/file_rules/{file_rule_id}` and new parameter `file_rule_id`.
- New schemas `BaseFileRule`, `FileRuleSource`, `FileRuleUpload`, `FileRuleRetention`, and `ReleaseVersion` (enum `[v0.2.2]`).
- `Release` gains the required `file_rule_ids`.
- `VersionResponse` gains the required `api_release_version`. This field was added upstream after v0.2.1 and ships in v0.2.2.
- The `APIGitCommit` enum becomes `81f3a3a…`.
- Copy edits ("filesystem" becomes "file system" in two event descriptions) and an `Error` example.

The server URL stays `http://localhost/v0.2`.

`./api/regen.sh` runs from the repo root and needs `npx` plus a Java runtime; `api/openapitools.json` pins openapi-generator 7.12.0. It regenerates models for both specs and replaces `libs/backend-api/src/models/*` and `libs/device-api/src/models/*` wholesale, using the gitignored `api/codegen/` as scratch. Never hand-edit these models. A dry run confirmed what regeneration changes:

- The backend models are unchanged.
- New device models: `base_file_rule.rs`, `file_rule_source.rs`, `file_rule_upload.rs`, `file_rule_retention.rs`, `release_version.rs`.
- Modified device models: `api_git_commit.rs`, `deployment_deployed_event.rs`, `deployment_removed_event.rs`, `mod.rs`, `release.rs`, `version_response.rs`.

Code against these generated shapes:

    pub struct BaseFileRule { pub object: base_file_rule::Object /* ::FileRule */,
        pub id, name, digest: String, pub source: Box<FileRuleSource>,
        pub upload: Option<Box<FileRuleUpload>>, pub retention: Option<Box<FileRuleRetention>>,
        pub created_at, updated_at: String }
    pub struct FileRuleSource { pub glob: String, pub stability_window_secs: i64 }
    pub struct FileRuleUpload { pub upload_collection_id, upload_collection_name,
        bucket_id, bucket_name, path: String }
    pub struct FileRuleRetention { pub require_upload: Option<bool>, pub ttl_secs: i64 }
    pub struct Release { .., pub file_rule_ids: Vec<String>, .. }          // required
    pub struct VersionResponse { .., pub api_release_version: String, .. } // required
    pub enum ReleaseVersion { RELEASE_VERSION /* "v0.2.2" */, ReleaseVersionUnknown }

`Option` fields are skipped when serializing a `None`. The two new required fields mean the workspace does not compile after regeneration until `response.rs` and `handlers::version` set them, so M1 includes those edits and each commit still builds.

The spec defines `retention.require_upload` as "present exactly when the rule has an `upload` block". The domain model stores a plain `bool`, so the response sets `require_upload` to `Some(b)` when `rule.upload.is_some()` and to `None` otherwise. `ttl_secs` is `u64` in the model and `i64` on the wire; convert it with `i64::try_from(..).unwrap_or(i64::MAX)`.

API version reporting lives in `agent/src/version/mod.rs`. `api_version()` returns `ApiVersion::API_VERSION`, and `api_git_commit()` returns `ApiGitCommit::API_GIT_COMMIT`, which updates automatically on regen. `handlers::version` builds `VersionResponse` from both. `agent/build.rs` does not touch the API version. M1 adds `api_release_version()`.

### Repo conventions (from `AGENTS.md`)

- Imports come in groups headed `// standard crates`, `// internal crates`, and `// external crates`. The custom linter enforces this and treats `device_api`, `backend_api`, and `miru_agent` as internal.
- Production functions may have at most 50 non-blank, non-comment body lines. `routes()` has 43 today; the new route makes it 47.
- Integration tests live in `agent/tests/`, mirror `agent/src/`, and are declared in each `mod.rs`.
- A test must not make 4 or more `assert_eq!` calls on fields of the same variable; compare whole structs instead.
- Use portable temp-dir fixtures (`crate::test_utils::filesys::dirs::temp`) and no `#[cfg(unix)]`, because CI also runs the suite on Windows.
- Each module directory with a `.covgate` file (for example `agent/src/services/git_commit/.covgate`) is gated at that coverage percentage by `scripts/covgate.sh`. `agent/src/version/.covgate` is `100`.

## Plan of Work

### M1: vendor spec v0.2.2, regenerate, and set the required fields

1. Render the spec over `api/specs/device/v02.yaml` with the script in Concrete Steps, then run `./api/regen.sh`.
2. `agent/src/version/mod.rs`: after `api_git_commit`, add `pub fn api_release_version() -> String { device_api::models::ReleaseVersion::RELEASE_VERSION.to_string() }`.
3. `agent/src/server/handlers.rs` `version()`: add `api_release_version: version::api_release_version(),` after `api_git_commit`.
4. `agent/src/server/response.rs` `From<&models::Release>`: add `file_rule_ids: release.file_rule_ids.clone(),`.
5. Update the tests:
   - `agent/tests/server/response.rs`: add `file_rule_ids: Vec::new()` to both expected `openapi::Release` literals. Add `converts_release_with_file_rule_ids`, where ids `["fr-1", "fr-2"]` convert unchanged and in order.
   - `agent/tests/server/handlers.rs`: add `api_release_version: version::api_release_version()` to the expected `VersionResponse`. In `releases::get_current_release_returns_200`, store the release with `file_rule_ids: vec!["fr-1".into(), "fr-2".into()]` and assert that `actual.file_rule_ids` equals it. That makes three field asserts on `actual`, which is under the linter limit.
   - `agent/tests/version/mod.rs`: add `test_api_release_version_extends_api_version`, which asserts `version::api_release_version().starts_with(&format!("{}.", version::api_version()))`.

### M2: add `GET /file_rules/{file_rule_id}`

1. Create `agent/src/services/file_rule/mod.rs` containing `mod get; pub use get::*;`. Create `agent/src/services/file_rule/get.rs`:

        // internal crates
        use crate::disk;
        use crate::models;
        use crate::services::errors::ServiceErr;

        pub async fn get(
            file_rules: &disk::FileRules,
            id: String,
        ) -> Result<models::FileRule, ServiceErr> {
            Ok(file_rules.read(id).await?)
        }

   Create `agent/src/services/file_rule/.covgate` containing `100`. Add `pub mod file_rule;` after `pub mod events;` in `agent/src/services/mod.rs`.
2. `agent/src/server/response.rs`: add field-copy conversions `From<&models::FileRuleSource> for device_server::FileRuleSource` and `From<&models::FileRuleUpload> for device_server::FileRuleUpload`. Add a private `fn to_retention(retention: &models::FileRuleRetention, has_upload: bool) -> device_server::FileRuleRetention` that sets `require_upload: has_upload.then_some(retention.require_upload)` and `ttl_secs: i64::try_from(retention.ttl_secs).unwrap_or(i64::MAX)`. Add `From<&models::FileRule> for device_server::BaseFileRule` with `object: device_server::base_file_rule::Object::FileRule`, boxed `source`, `upload`, and `retention` (the latter through `to_retention(r, rule.upload.is_some())`), and timestamps via `to_rfc3339()`.
3. `agent/src/server/handlers.rs`: add `file_rule as file_rule_svc` to the `crate::services::{...}` import. Add a `FILE RULES` banner section with `get_file_rule(AxumState(state): AxumState<Arc<State>>, Path(file_rule_id): Path<String>)`. Mirror `get_git_commit`: it calls `file_rule_svc::get(&state.storage.file_rules, file_rule_id)`, returns `device_server::BaseFileRule::from(&rule)`, and uses the error message `"Error getting file rule"`.
4. `agent/src/server/routes.rs`: after the git-commits route, add a `FILE RULES` banner and `.route(format!("/{api_version}/file_rules/{{file_rule_id}}").as_str(), get(handlers::get_file_rule))`.
5. `ARCHITECTURE.md`: add `file_rule` (cached file-rule lookup) to the `services/` submodule list. In the "Generated code" paragraph, add one sentence saying that the vendored specs are the openapi release-stamped artifacts (they carry `x-release-version` and `x-git-commit`), not raw bundles. A search found no README, AGENTS.md, or CHANGELOG list of Device API routes to update.
6. Add the M2 tests listed under Validation and Acceptance. Declare `pub mod file_rule;` in `agent/tests/services/mod.rs` and `pub mod get;` in the new `agent/tests/services/file_rule/mod.rs`.

### M3: cache rule bodies when a release is fetched from the backend

In `agent/src/services/release/get.rs`, `get` becomes `get<B: BackendFetcher>(releases: &disk::Releases, file_rules: &disk::FileRules, backend: &B, id: String)`. On a cache miss it now does the following:

1. Make `backend_rls` mutable and `take()` its `file_rules`. Keep returning today's `FileRulesNotExpanded` error when that is `None`.
2. Build `storage_rls` from the rule ids, as today.
3. Call a new `async fn cache_file_rules(file_rules: &disk::FileRules, rules: Vec<backend_client::BaseFileRule>) -> bool`. It converts each rule with `models::FileRule::from` and calls `write_if_absent(id, rule, |_, _| false)`. It logs each failure with `error!` and returns whether all writes succeeded.
4. Call `cache_release` only when `cache_file_rules` returned `true`.

Add a one-line code comment explaining the order: a cached release skips the backend fetch, so it must never be cached while one of its rule bodies is missing. Import `backend_api::models as backend_client`. Cache failures stay best-effort, as in `cache_release`: they are logged and the release is still returned.

`agent/src/services/release/current.rs`: `get_current(deployments, releases, file_rules, backend)` passes `file_rules` through. In `agent/src/server/handlers.rs`, `get_release` and `get_current_release` pass `&state.storage.file_rules`. Leave `sync/deployments.rs` unchanged.

In the tests, the `setup` functions in `agent/tests/services/release/get.rs` and `current.rs` also spawn `FileRules::spawn(16, dir.file("file_rules.json"), 1000)` and return the store. Update every call site: 12 `rls_svc::get` calls and 8 `rls_svc::get_current` calls. Then add the M3 tests listed under Validation and Acceptance.

## Concrete Steps

Run every command from `/home/user/agent` unless another directory is stated. The integration test target is `mod` (`agent/tests/mod.rs`).

### M1

Check the preconditions:

    git status --short                                    # expect nothing outside plans/
    git -C /home/user/openapi cat-file -e 81f3a3a05a8a3c4e6502304248e73aa7683895b8 && echo ok
    python3 -c 'import yaml' && echo ok                   # PyYAML (used by spec.py)

Save the render script as `${TMPDIR:-/tmp}/render_device_spec.py`, removing the 4-space block indent shown here:

    import subprocess, sys
    sys.path.insert(0, "/home/user/openapi")
    from tools.release import spec
    from tools.release.models import GitCommit

    SHA = "81f3a3a05a8a3c4e6502304248e73aa7683895b8"
    git = lambda *a: subprocess.run(["git", "-C", "/home/user/openapi", *a], check=True, capture_output=True, text=True).stdout
    commit = GitCommit(sha=SHA, message=git("log", "-1", "--pretty=%B", SHA).strip(), author=git("log", "-1", "--pretty=%an", SHA).strip(), branch="HEAD", dirty=False)
    text = spec.render(template=git("show", f"{SHA}:apis/apps/device-server/openapi.gen.yaml"), api_version="v0.2", release_version="v0.2.2", commit=SHA, git_commit=commit.to_dict(), build_info=spec.build().to_dict())
    OLD = "          description: SSE event stream. Each event is delivered as an SSE frame with\n            `id`, `event`, and `data` fields.\n"
    NEW = "          description: >-\n            SSE event stream. Each event is delivered as an SSE frame with\n            `id`, `event`, and `data` fields. A \": heartbeat\" comment is sent\n            immediately when the connection opens, and again every 30 seconds\n            while the stream is idle. Comments carry no data and should be\n            ignored by clients.\n"
    assert text.count(OLD) == 1, "SSE description anchor not found exactly once"
    open(sys.argv[1], "w", encoding="utf-8").write(text.replace(OLD, NEW))

Render the spec and verify it:

    PYTHONDONTWRITEBYTECODE=1 python3 "${TMPDIR:-/tmp}/render_device_spec.py" api/specs/device/v02.yaml
    grep -n 'x-release-version: v0.2.2' api/specs/device/v02.yaml                 # 10:  x-release-version: v0.2.2
    grep -c 81f3a3a05a8a3c4e6502304248e73aa7683895b8 api/specs/device/v02.yaml    # 3 (sha, url, APIGitCommit)
    grep -c '\$[A-Z_]*\$' api/specs/device/v02.yaml                               # 0
    grep -c 'heartbeat' api/specs/device/v02.yaml                                 # 1
    git diff --stat api/specs/device/v02.yaml                                     # ~410 insertions, ~31 deletions
    git -C /home/user/openapi status --short                                      # expect empty

Regenerate the models (the first run downloads the generator):

    ./api/regen.sh
    git status --short libs/

Expected output, in any order:

     M libs/device-api/src/models/api_git_commit.rs
     M libs/device-api/src/models/deployment_deployed_event.rs
     M libs/device-api/src/models/deployment_removed_event.rs
     M libs/device-api/src/models/mod.rs
     M libs/device-api/src/models/release.rs
     M libs/device-api/src/models/version_response.rs
    ?? libs/device-api/src/models/base_file_rule.rs
    ?? libs/device-api/src/models/file_rule_retention.rs
    ?? libs/device-api/src/models/file_rule_source.rs
    ?? libs/device-api/src/models/file_rule_upload.rs
    ?? libs/device-api/src/models/release_version.rs

Nothing under `libs/backend-api` should change.

Make the M1 source and test edits, then run:

    cargo build -p miru-agent
    cargo test -p miru-agent --test mod server::
    cargo test -p miru-agent --test mod version::

Commit with the subject `feat(server): vendor Device API v0.2.2 and serve Release.file_rule_ids`. The body states that the spec was rendered from openapi `main` `81f3a3a` with `tools/release/spec.py` and stamped `v0.2.2` ahead of the `device/v0.2.2` tag, and that the SSE heartbeat description was re-applied. Append the orchestrator's trailer lines.

    git add api/specs/device/v02.yaml libs/device-api/src/models agent/src/version/mod.rs \
      agent/src/server/handlers.rs agent/src/server/response.rs agent/tests/server/response.rs \
      agent/tests/server/handlers.rs agent/tests/version/mod.rs
    git commit -F <message-file>

### M2

Make the edits, then run:

    cargo test -p miru-agent --test mod services::file_rule
    cargo test -p miru-agent --test mod server::

Commit with the subject `feat(server): add GET /file_rules/{file_rule_id} from the local cache`:

    git add agent/src/services/file_rule/mod.rs agent/src/services/file_rule/get.rs \
      agent/src/services/file_rule/.covgate agent/src/services/mod.rs agent/src/server/response.rs \
      agent/src/server/handlers.rs agent/src/server/routes.rs agent/tests/services/mod.rs \
      agent/tests/services/file_rule/mod.rs agent/tests/services/file_rule/get.rs \
      agent/tests/server/response.rs agent/tests/server/handlers.rs ARCHITECTURE.md
    git commit -F <message-file>

### M3

Make the edits, then run:

    cargo test -p miru-agent --test mod services::release
    cargo test -p miru-agent --test mod server::handlers

Commit with the subject `fix(services): cache file rule bodies when fetching an uncached release`:

    git add agent/src/services/release/get.rs agent/src/services/release/current.rs \
      agent/src/server/handlers.rs agent/tests/services/release/get.rs \
      agent/tests/services/release/current.rs
    git commit -F <message-file>

### M4: validate

Run the repo's local equivalents of the CI jobs (`.github/workflows/ci.yml`):

    cargo fmt -p miru-agent -- --check     # CI lint step
    ./scripts/test.sh                      # RUST_LOG=off cargo test --package miru-agent; expect 0 failed
    LINT_FIX=0 ./scripts/lint.sh           # import/funclen/assert linter, fmt --check, machete, diet, audit, clippy -D warnings
    ./scripts/covgate.sh                   # tests plus per-module .covgate gates (needs jq; installs cargo-llvm-cov)
    ./scripts/preflight.sh                 # lint + covgate + tools lint + tools covgate in parallel; ends "Preflight clean"
    git diff --exit-code Cargo.lock        # no dependency changes

`scripts/lint.sh` needs `cargo-machete`, `cargo-audit`, and `cargo-diet`; install them with `cargo binstall` as CI does, or rely on CI when they are unavailable. Do not run `scripts/update-deps.sh`, because it runs `cargo update` and this change adds no dependencies. If `services/file_rule` measures below 100% in `covgate.sh`, set its `.covgate` to the measured value and record it in Surprises & Discoveries. If lint or format fixes are needed, commit them as `style: apply lint fixups` with explicit paths. The orchestrator then pushes the branch and opens a draft PR, and preflight watches CI.

## Validation and Acceptance

M1 tests:

- `server::response::release_response::converts_release_with_file_rule_ids`, plus the two updated release conversions.
- `server::handlers::version_tests::returns_ok_with_version_and_commit`: the body includes `api_release_version` equal to `version::api_release_version()`.
- `server::handlers::routes::releases::get_current_release_returns_200`: `file_rule_ids == ["fr-1", "fr-2"]`.
- `version::test_api_release_version_extends_api_version`: `v0.2.2` starts with `v0.2.`.

M2 tests:

- `agent/tests/services/file_rule/get.rs`:
  - `get_file_rule::returns_cached_rule`: a rule written with `write_if_absent` comes back equal.
  - `get_file_rule::missing_rule_returns_not_found`: an empty store gives `Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))`.
- `agent/tests/server/response.rs`, new module `file_rule_response`:
  - `converts_upload_rule_with_retention`: compare the whole expected `openapi::BaseFileRule`, where retention `require_upload: true` becomes `Some(true)`.
  - `converts_upload_rule_with_best_effort_retention`: `require_upload` becomes `Some(false)`.
  - `converts_retention_only_rule_omits_require_upload`: with no upload, the result has `upload == None` and retention `require_upload == None`, and `serde_json::to_value(&sdk)` has no `upload` key and no `retention.require_upload` key.
  - `converts_rule_without_retention`: `retention == None`.
  - `saturates_ttl_secs_above_i64_max`: `u64::MAX` becomes `i64::MAX`.
- `agent/tests/server/handlers.rs`, new module `routes::file_rules`:
  - `get_file_rule_returns_200`: the body equals `openapi::BaseFileRule::from(&stored_rule)`.
  - `get_file_rule_returns_404_when_not_cached`: status 404 and `error.code == "resource_not_found"`.
  - `client_flow_detects_active_uploads`: store a Deployed deployment whose release has ids `["fr-upload", "fr-retain"]`, plus both rule bodies; `fr-retain` is retention-only. Then `GET /v0.2/releases/current` and, for each id, `GET /v0.2/file_rules/{id}`. Every request returns 200, `any(upload.is_some())` is true, and `fr-retain` has `upload == None`.

M3 tests, in `agent/tests/services/release/get.rs`:

- `cache_miss_caches_file_rule_bodies`: the backend release has `file_rule_1` (with upload) and `file_rule_2` (retention-only). After `get`, each `file_rules.read(id)` equals `FileRule::from(backend_rule)`, and a second `get` with `PanicBackend` succeeds.
- `cache_miss_keeps_existing_file_rule_body`: a pre-seeded `file_rule_1` named `"seeded"` is not overwritten.
- `cache_miss_rule_cache_failure_does_not_cache_release`: the `FileRules` store is shut down first. `get` still returns `Ok` with the ids, and `releases.read_optional("rls_1")` is `None`.
- The existing `cache_miss_backend_missing_file_rules_errors_and_does_not_cache` still passes.

Before this change these tests do not compile or they fail; afterwards they pass. `./scripts/test.sh` reports `0 failed`.

Observable behavior with a running agent (Linux socket `/run/miru/miru.sock`): `curl --unix-socket /run/miru/miru.sock http://localhost/v0.2/releases/current` includes `"file_rule_ids":[…]`. `curl --unix-socket … http://localhost/v0.2/file_rules/<cached id>` returns the rule JSON; a retention-only rule has no `upload` key. An unknown id returns HTTP 404 with a body like `{"error":{"code":"resource_not_found",…}}`. `…/v0.2/version` includes `"api_release_version":"v0.2.2"`.

Spec provenance check: re-running the render script into a temp file and diffing it against `api/specs/device/v02.yaml` shows only the `built_at` line.

CI gate: CI runs `lint`, `test` (`./scripts/covgate.sh`), `windows-check` (`cargo test --package miru-agent --locked` on Windows), and `tools`. Preflight must report CLEAN, meaning the `ci.yml` run is green on the pushed branch head, before the PR leaves draft or the task is reported complete. A local pass is necessary but not sufficient. A red or pending head blocks completion; fix from the CI job logs and push again.

## Idempotence and Recovery

- Rendering is repeatable: each run rewrites `api/specs/device/v02.yaml` and changes only `x-build.built_at`. `git checkout -- api/specs/device/v02.yaml` restores the old file. If the script's assert fires, the SSE anchor has changed upstream; re-derive `OLD` from the rendered text before retrying.
- `./api/regen.sh` deletes and rewrites the models, so re-runs are safe. To recover from a bad run, use `git checkout -- libs/ && git clean -fd libs/device-api/src/models` and `rm -rf api/codegen`. If `npx` or Java is missing, fix the toolchain; never hand-edit generated models.
- If `device/v0.2.2` is later tagged on a commit other than `81f3a3a`, or the official `device.yaml` asset differs beyond `built_at`, re-vendor from that asset, re-apply the SSE edit, and regenerate.
- The source edits are ordinary and reversible. The milestones are one commit each and each commit builds, so the branch can be bisected or a milestone reverted on its own. If `Cargo.lock` changes, revert it with `git checkout -- Cargo.lock`.
