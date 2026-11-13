# Host Device API versions side by side in the agent's server

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Rename the generated Device API crate by version, move the server's v0.2 wire code into a version module, make the device sync service return a domain type, tests, docs |

This plan lives in `agent/plans/` because every change is in the agent repo. Paths are relative to the agent repo root (the directory holding the workspace `Cargo.toml` and `ARCHITECTURE.md`); the crate is in `agent/`. Branch: `refactor/device-api-versions`, based on `main`.

## Purpose / Big Picture

The agent serves a local HTTP API, the Device API, to programs on the same device, over a Unix socket and loopback TCP. Today it serves one version, `/v0.2/...`, and every version-specific piece (route table, handlers, wire conversions, the server-sent events (SSE) stream at `/v0.2/events`) sits directly in `agent/src/server/`, built on one generated crate, `libs/device-api`. A follow-up PR will add `/v0.3`, whose only contract change is that calibrations drop `role` and `aspect`.

This PR is a structural move only. After it, each Device API version is one generated crate (`libs/device-api-v02`) plus one module (`agent/src/server/v02/`), and `agent/src/server/routes.rs` composes the versions and applies the shared middleware once. Adding `/v0.3` then means a new crate `libs/device-api-v03`, a new module `agent/src/server/v03/`, and one `.merge(...)` line in `routes.rs`.

Nothing a client sees changes: every `/v0.2` route keeps its path, status codes, bodies, headers, bearer auth on TCP, middleware and request body limit, and `/v0.3` is not served (404, as today).

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| crate `device-api-v02` (`device_api_v02` in Rust) at `libs/device-api-v02/` | `device-api` with one module per version; `libs/device-api/v02/` nested dir | The model template (`api/templates/rust/model.mustache`) emits `use crate::models;`, so a version's models must be a crate's `models` module; two versions can't share one crate without template changes. A flat sibling dir matches `libs/backend-api/`. The suffix matches the spec file `api/specs/device/v02.yaml`. Renaming also ends the near-clash between the crate `device_api` and the discovery-file module `disk::device_api`. |
| Makefile `DEVICE_V02_FILE`, `DEVICE_V02_CODEGEN_DIR` (`codegen/device-v02`), targets `gen-device-v02`, `clean-device-v02` | keep `gen-device` and loop over versions; `gen-device-02` | One target per version is the smallest change and lets `/v0.3` add `gen-device-v03` beside it; a loop is a generalization with one version. |
| module `server::v02` | `server::v0_2`, `server::device_v02` | Same suffix as the spec file and the crate; `device_` is redundant under `server`. |
| `server::v02::routes::table` (`pub fn table() -> Router<Arc<State>>`) | `server::v02::router`, `server::v02::routes::router` | Keeps the name the function already has in `server/routes.rs`; a second `router` beside `server::routes::router` would mean two things. |
| `services::device::SyncOutcome` (struct: `code`, `message`, `state`) | `Synced`, `SyncReport` | `Synced` already names an enum in `sync/system_metadata.rs`; "report" means a calibration report in this repo. `Outcome` is the word `deploy::apply::Outcome` and `app::upgrade::Outcome` use for a step's result. |
| `services::device::SyncCode` (enum: `Success`, `InCooldown`, `NetworkConnectionError`) | `SyncResult`, `SyncStatus` | It fills the wire field `code`; `SyncResult` reads like `std::result::Result`, and "status" here means stored deployment/device status enums. |

### Precedent

| New file | Mirrors |
|----------|---------|
| `libs/device-api-v02/` (moved from `libs/device-api/`) | `libs/backend-api/`; the move mirrors commit 8c51c6fb, which renamed `libs/openapi-server` to `libs/device-api` with `git mv` and a `name` change in `Cargo.toml` |
| `agent/src/server/v02/mod.rs` | `agent/src/server/mod.rs` (`pub mod` per file) |
| `agent/src/server/v02/routes.rs` | the `table` function moved out of `agent/src/server/routes.rs` unchanged except that it no longer calls `.with_state` |
| `agent/src/server/v02/{handlers,request,response,sse}.rs` | moved from `agent/src/server/` with `git mv`; only `use` lines and the `/version` handler change |
| `agent/src/server/v02/.covgate` | every subdirectory of `agent/src/` has one (e.g. `agent/src/services/device/.covgate`); value `87`, as `agent/src/server/.covgate` |
| `agent/tests/server/v02/mod.rs` and `agent/tests/server/v02/{handlers,request,response,sse}.rs` (moved) | `agent/tests/server/mod.rs`; tests mirror `agent/src/` |
| `agent/tests/server/routes.rs` | `agent/tests/server/tcp.rs` (its `Fixture` and the `oneshot` call in `authorization_header_is_redacted_in_trace_spans`) |

### Left out

- `/v0.3` itself, `api/specs/device/v03.yaml` and `libs/device-api-v03`: the follow-up PR.
- Splitting `sse.rs` into version-neutral plumbing (cursor, type filter, stream, heartbeat) in `server/` and the wire parts in `server/v02/`: the whole file moves, because its neutral parts call `request::split_list`, which moves too, and a shared generic stream builder would have one caller. Proposed follow-up for the `/v0.3` PR, which has the second caller.
- Changing the `impl_status_enum!` `agent_type` twins in `agent/src/models/{device,deployment,os,file_rule}.rs`: they keep pointing at v0.2 wire types (import rename only). Follow-up: `/v0.3`'s `response.rs` maps the domain enums to its own wire enums with `From` impls (the macro takes one `agent_type`).
- Changing the event store: `agent/src/events/model.rs` keeps writing event data as v0.2 wire JSON (`DeploymentDeployedEvent`, `DeploymentRemovedEvent`). Follow-up: if the v0.3 event schemas match, `/v0.3` serves the stored JSON as is; if they ever differ, store a domain event and convert per version.
- Keeping `version::api_version`, `version::api_git_commit`, `version::api_release_version`: each version reports its own spec constants, so the v0.2 `/version` handler reads them from `device_api_v02::models` and the helpers are deleted. Their two "is not empty" tests in `agent/tests/version/mod.rs` go too: `returns_ok_with_version_and_commit` already compares every field to the generated constants.
- `SyncOutcome` holding `in_cooldown`: the v0.2 conversion calls `state.is_in_cooldown()`, microseconds after the service would have; the response is the same.
- `impl_status_enum!` for `SyncCode`: the macro's `agent_type` ties a type to one Device API crate, which is what this PR removes from services. `SyncCode` is a per-request result never stored or parsed, so it gets plain derives and a `From` impl in `server/v02/response.rs`.
- Using axum `Router::nest("/v0.2", ...)` in place of the `format!("/{api_version}/...")` paths: rewriting every line of the table is diff noise, and `nest` strips the prefix from the URI and adds a `NestedPath` extension, which a pure move should not risk.
- A shared test fixture for building `State` (each of `handlers.rs`, `sse.rs`, `tcp.rs` has its own): unrelated cleanup.

### Open questions

- Should `api/Makefile` keep a `gen-device` target that calls every version? Default: no; `gen` calls `gen-device-v02` and `clean` calls `clean-device-v02`.
- Is `87` right for `agent/src/server/v02/.covgate`? Default: yes, the parent's value. If CI's `test` job reports `server/v02` below it, set the file to the reported value rounded down and record it in Surprises & Discoveries; never lower `agent/src/server/.covgate`.

## Progress

- [x] M1: generated crate renamed to `device-api-v02`; all imports updated.
- [x] M2: `services::device::sync` returns `SyncOutcome`; v0.2 conversion added.
- [x] M3: `server/v02/` version module; `routes::router` composes it; tests moved.
- [x] M4: new tests (router composition, `SyncOutcome` conversion).
- [x] M5: docs (`CLAUDE.md`, `AGENTS.md`, `ARCHITECTURE.md`, `README.md`).
- [ ] CI green on the pushed head (`$preflight` reports `CLEAN`).
- [x] Source written
- [x] Tests written

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- M2: the `SyncCode` to `SyncDeviceResult` mapping is its own `impl From<SyncCode> for device_server::SyncDeviceResult` in `server/v02/response.rs`, which `From<&SyncOutcome> for SyncDeviceResponse` calls with `outcome.code.into()`. It is the "`From` impl" the Left-out section names for `SyncCode`, and `Self::` keeps each match arm within rustfmt's line width.
- M2: `services/device/sync.rs` imports `sync::syncer::State` and types the field `pub state: State`; `syncer::State` would put the module `syncer` beside the parameter `syncer` in `sync`.
- M5: `CLAUDE.md` is a symlink to `AGENTS.md`, so one edit keeps them identical.
- M1: `tools/lint/src/classifier/mod.rs` keeps `"device_api"` in its unit test: it is the linter's own fixture list, not the repo's `.lint-imports.toml`. `api/regen.sh` keeps its `device_*` variable names; only the two paths change.
- Design review: the author chose that each version module is a full copy of the API, so `/v0.3` mirrors `server/v02/` whole (handlers, SSE, conversions, and `From` impls for its own status enums) rather than reusing v0.2's unchanged handlers. The Follow-up stands as written.
- Design review: `InstanceFormat::from_device_api` moves out of `models/instance_format.rs` into `server/v02/request.rs` as a private conversion used by the `TryFrom` there, with its tests moved to `agent/tests/server/v02/request.rs` and run through the public `TryFrom`. It is named `to_instance_format`, as `response.rs` names its private `to_retention`. Request conversions belong in the version's `request.rs`, and with a full copy per version `/v0.3` writes its own beside it instead of editing `models`.
- CI on the source stage: `services/device` coverage fell to 94.28% (gate 96.07%) because `sync` got shorter while its two `get_sync_state()?` error paths stayed uncovered (`MockSyncer::get_sync_state` can't fail). The tests stage adds a way for the mock to fail `get_sync_state` and covers both paths; the gate is not lowered.
- Self-review: in `agent/tests/services/device/sync.rs`, the locals `resp` and the helper `expected_response` become `outcome` and `expected_outcome`, since they now hold a `SyncOutcome`, not a wire response.

## Outcomes & Retrospective

Source and tests are written; CI on the pushed head has not run on the tests yet.

- `agent/tests/server/routes.rs` checks that `routes::router` answers `GET /v0.3/health` with 404 and an empty body and `POST /v0.2/health` with 405. The planned `serves_v02_routes` (`GET /v0.2/health` → 200) was left out: `redaction::authorization_header_is_redacted_in_trace_spans` in `agent/tests/server/tcp.rs` already sends that request through `routes::router` and asserts 200, and every route test in `agent/tests/server/v02/handlers.rs` goes through it too.
- `agent/tests/server/v02/response.rs` gets a `sync_device_response` module, named like its sibling modules (`device_response`, `deployment_response`) after the wire type rather than `sync_outcome`: one test maps each `SyncCode` with `cooldown_ends_at` at `UNIX_EPOCH` (`in_cooldown` false), one with `cooldown_ends_at` a day ahead (`in_cooldown` true). Both compare whole `SyncDeviceResponse` values.
- Coverage fix for `services/device`: `MockSyncer` gets `set_get_sync_state_fail`, like `set_subscribe_fail`, and `agent/tests/services/device/sync.rs` adds `sync_state_unavailable_after_sync` and `sync_state_unavailable_after_cooldown`, each asserting `ServiceErr::SyncErr(SyncErr::MockErr(_))`. The cooldown case is used for the error branch because its sync error differs from the mock's state-read error, so the assertion shows which call failed.

## Context and Orientation

Generated code. `api/specs/device/v02.yaml` is the vendored OpenAPI spec of the Device API. `api/Makefile` target `gen-device` runs openapi-generator into `api/codegen/device` (git-ignored) with `--additional-properties=packageName=device-api`, and `api/regen.sh` copies only `src/models/` from there into `libs/device-api/src/models/`. `libs/device-api/Cargo.toml` and `libs/device-api/src/lib.rs` (`pub mod models;`) are hand-kept; the generated model files do not contain the package name, so renaming the package never touches them. The crate is a workspace member and dependency in the root `Cargo.toml` (`"libs/device-api"`, `device-api = { path = "libs/device-api" }`), a dependency in `agent/Cargo.toml` (`device-api = { workspace = true }`), a package in `Cargo.lock`, and an internal crate for the import linter in `.lint-imports.toml` (`internal_crates`), which sorts `use` lines into `// internal crates` and `// external crates` groups. `libs/backend-api` (spec `api/specs/backend/v05.yaml`) is the control-plane client and is not touched.

The server (`agent/src/server/`). `routes.rs` has `router(state)`, which builds the private `table(state)` and wraps it in `middleware` (Authorization redaction, `TraceLayer`, `DefaultBodyLimit::max(MAX_REQUEST_BYTES)`). `table` reads `device_api::models::ApiVersion::API_VERSION` (`"v0.2"`) and registers 15 routes as `format!("/{api_version}/...")`, with `/deployments/current` before `/deployments/{deployment_id}` and `/releases/current` before `/releases/{release_id}`, then calls `.with_state(state)`. `tcp.rs` and `unix.rs` call `routes::router(state)`; `tcp.rs` adds loopback and bearer checks around it. Version-neutral files: `auth.rs`, `errors.rs` (`ServerErr`, `InvalidRequestErr`, `CalibrationRequestTooLargeErr`), `state.rs` (`State`), `tcp.rs`, `unix.rs`, `mod.rs`, and the middleware in `routes.rs`. Version-specific files, all importing `device_api::models as device_server`:

- `handlers.rs`: one handler per route; `reject_body` (uses `routes::MAX_REQUEST_BYTES`) and `reject_query`; `handle`, `error_response`, `to_error_response` (build the wire `ErrorResponse`). `version()` fills `VersionResponse` from `crate::version` constants and `version::api_version()`, `api_git_commit()`, `api_release_version()`. `sync_device` returns `dvc_svc::sync(...)` directly.
- `request.rs`: `TryFrom<CreateCalibrationRequest> for services::calibration::CreateParams`, `LatestCalibrationsQuery` and `From<LatestCalibrationsQuery> for calibrations::Filter`, `NULL_SLUG`, `pub(crate) fn split_list`, private `split_slugs`, inline unit tests.
- `response.rs`: `From` impls from domain types to wire types, plus `latest_calibrations` and `calibration_content`.
- `sse.rs`: `EventsQuery`, `events` (the `/events` handler; error body is a wire `ErrorResponse`), `events_impl` (serializes each event as wire `Event`), `resolve_cursor`, `parse_event_filter` (calls `request::split_list`).

`agent/src/version/mod.rs` defines `api_version()`, `api_git_commit()`, `api_release_version()` from `device_api::models::{ApiVersion, ApiGitCommit, ReleaseVersion}`; only the `/version` handler calls them.

`agent/src/services/device/sync.rs` defines `sync(syncer) -> Result<device_api::models::SyncDeviceResponse, ServiceErr>`: on success code `SYNC_DEVICE_RESULT_SUCCESS` and message `"successfully synced"`; on `SyncErr::InCooldownErr` code `SYNC_DEVICE_RESULT_IN_COOLDOWN`, on a network error (`is_network_conn_err()`) code `SYNC_DEVICE_RESULT_NETWORK_CONNECTION_ERROR`, both with `message: e.to_string()`; any other error is returned. Each `Ok` reads `syncer.get_sync_state()` (`sync::syncer::State`: `last_attempted_sync_at`, `last_synced_at`, `cooldown_ends_at`, `err_streak`, and method `is_in_cooldown()`) and formats times with `to_rfc3339()`. It is the only service that returns a wire type; others return domain types such as `services::calibration::Created`.

Other `device_api` users, import-only in this PR: `agent/src/models/{device,deployment,os,file_rule,instance_format}.rs` (`use device_api::models as agent_server;`), `agent/src/events/model.rs` (`use device_api::models as device_server;`). Do not confuse them with `agent/src/disk/device_api.rs` (`disk::device_api`, the TCP discovery file) and `Layout::device_api()`, which keep their names.

Tests. `agent/tests/` mirrors `agent/src/`. `agent/tests/server/mod.rs` declares `errors, handlers, request, response, sse, tcp`. `handlers.rs` holds handler tests plus a `routes` module with route tests through `routes::router` (each URI is a literal like `/v0.2/deployments/current`, including `current_route_not_captured_as_deployment_id`). `request.rs`, `response.rs` import `miru_agent::server::{request, response}` items; `sse.rs` and `tcp.rs` use `routes::router`; `tcp.rs` and `agent/tests/app/run.rs` build URLs from `device_api::models::ApiVersion`. `agent/tests/services/device/sync.rs` asserts whole `SyncDeviceResponse` values. `agent/tests/version/mod.rs` tests the three `api_*` helpers. Other test files import `device_api::models`: `agent/tests/models/{deployment,device,file_rule,instance_format,os}.rs`, `agent/tests/events/model.rs`.

Coverage. `scripts/covgate.sh` finds every `.covgate` under `agent/src`, and each gate counts every file whose path starts with its directory, so `agent/src/server/.covgate` (87) keeps counting files moved into `server/v02/`.

CI. `.github/workflows/ci.yml` runs on pull requests: `lint` (`scripts/lint.sh`: import linter, `cargo fmt --check`, clippy `-D warnings`, machete, audit), `test` (`scripts/covgate.sh`: the suite plus coverage gates), `windows-check` (`cargo test --package miru-agent --locked`, so `Cargo.lock` must match), `windows-agent-build` (`--locked`), and others.

## Plan of Work

M1 — rename the generated crate. From the agent repo root, `git mv libs/device-api libs/device-api-v02`, then set `name = "device-api-v02"` in `libs/device-api-v02/Cargo.toml`. In the root `Cargo.toml`, change the member `"libs/device-api"` to `"libs/device-api-v02"` and the dependency line to `device-api-v02 = { path = "libs/device-api-v02" }`. In `agent/Cargo.toml`, change `device-api = { workspace = true }` to `device-api-v02 = { workspace = true }`. In `api/Makefile`, rename `DEVICE_FILE` to `DEVICE_V02_FILE`, `DEVICE_CODEGEN_DIR` to `DEVICE_V02_CODEGEN_DIR := codegen/device-v02`, `gen-device` to `gen-device-v02` (with `packageName=device-api-v02`), `clean-device` to `clean-device-v02`, and update `gen` and `clean` to call them. In `api/regen.sh`'s device block, read from `$codegen_dir/device-v02/src/models` and write to `libs/device-api-v02/src/models`. In `.lint-imports.toml`, replace `"device_api"` with `"device_api_v02"`. Replace every path `device_api::models` that is not preceded by `::` in `agent/src` and `agent/tests` with `device_api_v02::models` (this leaves `disk::device_api::...` alone). Refresh the lockfile with Cargo, never by hand. Do not run the generator: the model files are unchanged.

M2 — make the sync service version-neutral. In `agent/src/services/device/sync.rs`, add above `sync`:

    /// How a requested sync ended, when it didn't fail outright.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SyncCode {
        Success,
        InCooldown,
        NetworkConnectionError,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct SyncOutcome {
        pub code: SyncCode,
        pub message: String,
        pub state: sync::syncer::State,
    }

Change `sync` to return `Result<SyncOutcome, ServiceErr>` with the same branching: success gives `SyncCode::Success` and `"successfully synced"`, cooldown and network errors give their codes and `e.to_string()`, each with `state: syncer.get_sync_state().await?`. Drop the `device_api_v02` import. (`sync::syncer::State` already derives `Debug, Clone, PartialEq`.) In `agent/src/server/response.rs` (still at its old path in this milestone), add `impl From<&services::device::SyncOutcome> for device_server::SyncDeviceResponse` that maps each `SyncCode` to the matching `SyncDeviceResult::SYNC_DEVICE_RESULT_*`, copies `message`, formats the three times with `to_rfc3339()` and sets `in_cooldown: outcome.state.is_in_cooldown()`. In `handlers.rs` `sync_device`, wrap it like `get_device`: `let outcome = dvc_svc::sync(state.syncer.as_ref()).await?; Ok::<_, ServerErr>(device_server::SyncDeviceResponse::from(&outcome))`. In `agent/tests/services/device/sync.rs`, change the expected values from `SyncDeviceResponse` to `SyncOutcome` (`code`, `message`, `state`) so the test crate compiles; the assertions check the same facts.

M3 — the version module. With `git mv`, move `agent/src/server/{handlers,request,response,sse}.rs` to `agent/src/server/v02/`. Create `agent/src/server/v02/mod.rs`:

    pub mod handlers;
    pub mod request;
    pub mod response;
    pub mod routes;
    pub mod sse;

Create `agent/src/server/v02/routes.rs` by moving `table` and its `// lint:allow(funclen)` line out of `agent/src/server/routes.rs`: make it `pub fn table() -> Router<Arc<State>>`, keep its body and comments, change `handlers::` to the v02 path and `super::sse::events` to `sse::events`, and drop the final `.with_state(state)`. In `agent/src/server/routes.rs`, change `router` to `middleware(v02::routes::table().with_state(state))`, drop the `get`/`post` and `handlers` imports, and keep `MAX_REQUEST_BYTES`, `middleware` and `redact_authorization` unchanged. In `agent/src/server/mod.rs`, replace `pub mod handlers; pub mod request; pub mod response; pub mod sse;` with `pub mod v02;` (keep alphabetical order). Fix `use` lines in the moved files: `crate::server::{request, response, ...}` becomes `crate::server::v02::{request, response}` alongside `crate::server::{errors::*, routes, state::State}`. In `v02/handlers.rs` `version()`, set `api_version: ApiVersion::API_VERSION.to_string()`, `api_git_commit: ApiGitCommit::API_GIT_COMMIT.to_string()`, `api_release_version: ReleaseVersion::RELEASE_VERSION.to_string()` via `device_server::`, and delete the three `api_*` functions from `agent/src/version/mod.rs`. Add `agent/src/server/v02/.covgate` containing `87`.

Tests for M3 (moves and import fixes only): `git mv agent/tests/server/{handlers,request,response,sse}.rs agent/tests/server/v02/`; create `agent/tests/server/v02/mod.rs` with `pub mod handlers; pub mod request; pub mod response; pub mod sse;`; in `agent/tests/server/mod.rs` replace those four with `pub mod v02;`. Update `miru_agent::server::handlers` → `miru_agent::server::v02::handlers`, `miru_agent::server::request::` → `miru_agent::server::v02::request::`, `miru_agent::server::response::` → `miru_agent::server::v02::response::`. In `v02/handlers.rs` `version_tests::returns_ok_with_version_and_commit`, build the three `api_*` expected values from the generated constants as above. Move `test_api_release_version_extends_api_version` from `agent/tests/version/mod.rs` into `version_tests` as `api_release_version_extends_api_version`, asserting on the `/version` response's fields; delete the other two `api_*` tests there.

M4 — new tests. Create `agent/tests/server/routes.rs` (declare `pub mod routes;` in `agent/tests/server/mod.rs`) with a `Fixture` copied from `agent/tests/server/tcp.rs` (without the token) and three tests that send one request each through `routes::router(state).oneshot(...)`: `serves_v02_routes` (`GET /v0.2/health` → 200), `does_not_serve_v03` (`GET /v0.3/health` → 404, empty body), `wrong_method_is_405` (`POST /v0.2/health` → 405). In `agent/tests/server/v02/response.rs`, add a `sync_outcome` module whose test maps each `SyncCode` and checks the full `SyncDeviceResponse` (code, message, rfc3339 times, `in_cooldown` true for a `cooldown_ends_at` one day ahead and false for `UNIX_EPOCH`).

M5 — docs; keep `CLAUDE.md` and `AGENTS.md` identical. In both, the "Device API conversions" paragraph names `agent/src/server/v02/handlers.rs`, `server/v02/response.rs`, `server/v02/request.rs`, with "(one module per Device API version)"; the Testing line names `agent/tests/server/v02/handlers.rs`; "Generated code" and the enum conventions name `libs/device-api-v02/` and `device_api_v02`. In `ARCHITECTURE.md`: in the `server` paragraph, say each Device API version is a module (`server/v02/`) holding its route table, handlers and wire conversions over one generated crate, and `server/routes.rs` merges the versions and applies the shared middleware; "Route handlers live in `server/v02/handlers.rs`"; the `calibrations/` paragraph names `server/v02/handlers.rs`; the generated-code paragraph and invariant name `libs/device-api-v02` (one crate per Device API version). In `README.md`'s tree, `device-api/` becomes `device-api-v02/`.

## Concrete Steps

All commands run from the agent repo root on branch `refactor/device-api-versions`. Never run build, lint or test commands locally.

1. M1. Make the edits, then rewrite the imports and refresh the lockfile:

        grep -rlE '(^|[^:])device_api::models' agent/src agent/tests | xargs sed -i -E 's/(^|[^:])device_api::models/\1device_api_v02::models/g'
        cargo update --workspace

   Check: `grep -rnE '(^|[^:])device_api::models' agent/src agent/tests` prints nothing (the `device_api::write` and `device_api::remove` calls in `agent/tests/disk/device_api.rs` are the discovery-file module and stay); `git diff Cargo.lock` only renames `device-api` to `device-api-v02`. Then:

        git add -A libs api Cargo.toml Cargo.lock agent/Cargo.toml .lint-imports.toml agent/src agent/tests
        git commit -m "refactor(api): name the generated device API crate by version"

2. M2. Make the edits, then:

        git add agent/src/services/device/sync.rs agent/src/server/response.rs agent/src/server/handlers.rs agent/tests/services/device/sync.rs
        git commit -m "refactor(services): return a domain sync outcome from the device sync service"

3. M3. Run the `git mv` commands above, make the edits, then check `grep -rn "server::\(handlers\|request\|response\|sse\)\|version::api_" agent/src agent/tests` prints only `server::v02::...` paths, and:

        git add -A agent/src agent/tests
        git commit -m "refactor(server): move the v0.2 device API into a version module"

4. M4. Write the tests, then:

        git add agent/tests/server
        git commit -m "test(server): cover device API version routing and sync outcome conversion"

5. M5. Edit the docs, check `diff CLAUDE.md AGENTS.md` prints nothing, then:

        git add CLAUDE.md AGENTS.md ARCHITECTURE.md README.md
        git commit -m "docs: describe per-version device API modules and crates"

6. Validate with `$preflight` (push and watch CI).

## Validation and Acceptance

Validation runs in CI, never locally, and is complete only when `$preflight` reports `CLEAN`: CI green on the pushed branch head. The PR does not leave draft, and the task is not reported complete, before that. On the pushed head, `ci.yml` must pass:

- `lint`: the import linter accepts `device_api_v02` as an internal crate; fmt and clippy pass.
- `test`: the whole suite and coverage gates, including `agent/src/server/.covgate` (87) and the new `agent/src/server/v02/.covgate`. The moved v0.2 route tests in `agent/tests/server/v02/handlers.rs` pass unchanged apart from imports, which proves every `/v0.2` route keeps its status codes and bodies (including `current_route_not_captured_as_deployment_id` and the 413/422 body-limit tests); `agent/tests/server/tcp.rs` passes, proving bearer auth, loopback checks and Authorization redaction are unchanged; `agent/tests/server/v02/sse.rs` passes for `/v0.2/events`. New tests: `serves_v02_routes`, `does_not_serve_v03` and `wrong_method_is_405` in `agent/tests/server/routes.rs` cover version composition; the `sync_outcome` test in `agent/tests/server/v02/response.rs` covers the moved sync wire mapping.
- `windows-check` and `windows-agent-build`: compile with `--locked`, proving `Cargo.lock` matches the rename.

Observable behavior on a device after the change: `curl --unix-socket /run/miru/miru.sock http://localhost/v0.2/version` returns 200 with the same JSON as before (`"api_version":"v0.2"` and the same `api_git_commit` and `api_release_version`); `curl --unix-socket /run/miru/miru.sock http://localhost/v0.3/health` returns 404 with an empty body.

Follow-up (separate PR, not this plan): `/v0.3`. Vendor `api/specs/device/v03.yaml`, add `gen-device-v03` and a `regen.sh` block, add `libs/device-api-v03`, add `agent/src/server/v03/` mirroring `server/v02/` without `role` and `aspect` in calibrations, and change `routes::router` to `v02::routes::table().merge(v03::routes::table())`. That PR also settles the left-out items above: SSE plumbing shared between versions, `From` impls for the status enums, and how stored event JSON is served.

## Idempotence and Recovery

Every step is a file edit, `git mv` or commit. The M1 `sed` only matches `device_api::models` not preceded by `::`, so running it again changes nothing (`device_api_v02::models` doesn't match). `cargo update --workspace` only rewrites workspace entries in `Cargo.lock`; if it changes other packages, run `git checkout Cargo.lock` and retry offline with `cargo update --workspace --offline`. To undo a milestone, `git revert <sha>` from the agent repo root. No data or on-disk format changes.
