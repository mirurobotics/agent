# Serve Device API /v0.3 beside /v0.2

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add `daemon/src/server/v03/` over `libs/device-api-v03`, mount it beside `/v0.2`, add the flat-component read path for calibrations, tests, docs |
| `openapi/` | read-only | Source of the v0.3 contract (openapi#470) and of the Agent API v0.5 rule for how the control plane names a component |

This plan lives in `agent/plans/` because every change is in the agent repo. Paths are relative to the agent repo root (the directory holding the workspace `Cargo.toml` and `ARCHITECTURE.md`); the crate is `daemon/`. Branch: `feat/device-api-v03`, stacked on the draft PR branch `chore/vendor-device-v03-spec` (agent#510), which vendors `api/specs/device/v03.yaml` and generates `libs/device-api-v03`.

## Purpose / Big Picture

The agent serves the Device API, a local HTTP API for programs on the device, over a Unix socket and loopback TCP. Today it serves only `/v0.2`. Device API v0.3 (openapi#470) flattens calibrations: a calibration names one `component` slug and no `role` or `aspect`. After this change the agent serves every `/v0.3/...` route beside the unchanged `/v0.2/...` routes, over the same stores.

How to see it, on a device running this build (Linux, as root or a `miru` group member):

    curl -s --unix-socket /run/miru/miru.sock http://localhost/v0.3/version
    # {"api_version":"v0.3","api_release_version":"v0.3.0-next",...}

    curl -s --unix-socket /run/miru/miru.sock -X POST http://localhost/v0.2/calibrations \
      -H 'content-type: application/json' \
      -d '{"component":"arm","role":"left","content":{"format":"json","data":"{}"}}'
    curl -s --unix-socket /run/miru/miru.sock http://localhost/v0.2/calibrations/latest
    # one calibration: "component":"arm","role":"left","aspect":null   (unchanged)
    curl -s --unix-socket /run/miru/miru.sock 'http://localhost/v0.3/calibrations/latest?component=arm-left'
    # the same report: "component":"arm-left", with no role or aspect keys

`POST /v0.3/calibrations` with `{"component":"arm-left","content":{...}}` returns 200 with a `report_id`, and the agent uploads it to the control plane with only `component` set.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| module `server::v03` | `server::v0_3`, `server::v3` | Matches sibling `server::v02` and crate `device_api_v03`. |
| `ReportMetadata::flat_component` (`pub(crate)`, `daemon/src/calibrations/report.rs`) | `joined_component`; `component_slug` | "Flat" is openapi#470's word ("flatten components"). `component_slug` already means the as-reported component in `backend_api::models::Calibration`, so it would clash. |
| `services::calibration::list_latest_flat` | `list_latest_by_component`; `list_latest_per_component` | Pairs with `flat_component` and sits beside `list_latest`; "by component" reads like a filter. |
| `server::v03::request::LatestCalibrationsQuery` | `CalibrationsQuery`; `LatestCalibrationsParams` | Same name as the v0.2 sibling. |
| `LatestCalibrationsQuery::into_components` (v03 only) | `impl From<LatestCalibrationsQuery> for Vec<String>`; `components()` | v0.2 converts into the store's `Filter`, which v0.3 can't use (it matches stored components, not flat ones). `into_` because it consumes the query. |

### Precedent

| New file | Mirrors |
|----------|---------|
| `daemon/src/server/v03/mod.rs` | `daemon/src/server/v02/mod.rs` |
| `daemon/src/server/v03/routes.rs` | `daemon/src/server/v02/routes.rs` |
| `daemon/src/server/v03/handlers.rs` | `daemon/src/server/v02/handlers.rs` |
| `daemon/src/server/v03/request.rs` | `daemon/src/server/v02/request.rs`, without the `role`/`aspect` and `NULL_SLUG` parts |
| `daemon/src/server/v03/response.rs` | `daemon/src/server/v02/response.rs` |
| `daemon/src/server/v03/enums.rs` | `daemon/src/server/v02/enums.rs` |
| `daemon/src/server/v03/sse.rs` | `daemon/src/server/v02/sse.rs` |
| `daemon/src/server/v03/.covgate` | `daemon/src/server/v02/.covgate` (87) |
| `daemon/tests/server/v03/{mod,enums,handlers,request,response,sse}.rs` | the same files under `daemon/tests/server/v02/` |

### Left out

- Factoring the version-neutral helpers out of `server/v02/` into `server/`: `request::split_list`, `handlers::{reject_body, reject_query, handle, error_response}` (generic except for the `ErrorResponse` type), `response::{calibration_content, to_retention}`, and `sse::{resolve_cursor, parse_event_filter}` with the stream assembly in `events_impl`. A later release drops `/v0.2`, leaving one copy, and a move must land first as its own PR (shared rule). Proposed follow-up only if a reviewer wants the overlap shorter.
- Matching the store's `Filter` on flat component: it would change `/v0.2`'s `component`, `role` and `aspect` filters. `/v0.3` filters in the service instead.
- Rejecting `role`/`aspect` sent to `/v0.3`: the spec allows extra properties and the generated struct ignores them, as an unknown query parameter is ignored.
- `NULL_SLUG` checks on `/v0.3`: it has no role or aspect.
- Changing the backend client, `calibrations::sender` or the pull: Agent API v0.5 keeps its shape, and a `/v0.3` report is a v0.2-shaped report with `role` and `aspect` `None`.
- Re-vendoring when a `device/v0.3` tag exists (`api_release_version` is `v0.3.0-next` until then): agent#510's concern.

### Open questions

- Resolved by keying the store by flat component (Decision Log, 2026-11-16): should `GET /v0.3/calibrations/latest/{report_id}/content` return 404 for an entry that the `/v0.3` list hides behind a newer entry with the same flat component? No entry is hidden any more: a newer report replaces the older one and deletes its content, so the older report's content is 404 on both versions.
- Resolved by the same decision: when two entries with the same flat component have the same `reported_at`, which wins? The store holds one entry per flat component, and `put_if_newer` and the pull keep the entry it has on a tie (`is_as_new_as`), as they did per (component, role, aspect).
- Copy every `/v0.2` test for `/v0.3`, or only what differs? Default: copy every one, adapted, because the `/v0.2` tests leave with `/v0.2` and `/v0.3` must then stand alone and keep `server/v03/.covgate` at 87.

## Progress

- [x] M1: source: dependency and lint config, `flat_component`, `list_latest_flat`, `server/v03/`, router merge, docs.
- [x] Source written
- [x] Design review: store keyed by flat component (user's choice); `list_latest_flat` reduced to filter and sort; an index with several entries for one flat component keeps the newest on read.
- [ ] M2: tests: `daemon/tests/server/v03/`, router test, `list_latest_flat` service tests, `flat_component` unit tests.
- [ ] CI green on the pushed head (`$preflight` reports `CLEAN`).

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: the join lives on the domain type, as `ReportMetadata::flat_component`, and `list_latest_flat` in the calibration service picks the newest entry for each flat component; `server/v03/response.rs` calls `flat_component` to fill `Calibration.component`. The store and its `Filter` don't change.
  Rationale: the join is the control plane's rule (Agent API v0.5: "a report's component, role and aspect together name one component"), not only a v0.3 wire format, and both the service (to group and filter) and the v0.3 response need it; the server must not be imported by services. Stores return the domain type and the service picks what the route needs (AGENTS.md).
  Date/Author: 2026-11-16, plan authoring. Superseded in part by the store-keying entry below: the store changes, and `list_latest_flat` only filters and sorts.
- Decision: the store stays keyed by (component, role, aspect). On `/v0.3`, entries whose keys join to the same flat component count as one, and the newest `reported_at` wins, as the control plane's latest does; the `component` filter matches the flat component; results are ordered by flat component. `/v0.2` lists every entry as reported, as today.
  Rationale: `/v0.3` promises "one for each component"; `/v0.2` must not change.
  Date/Author: 2026-11-16, plan authoring. Superseded by the next entry.
- Decision: the store is keyed by flat component. `ReportMetadata::key()` and its `Key` tuple are gone; `is_as_new_as` and the store's `take_entry` compare `flat_component()`, so `put`, `put_if_newer`, the pull's skip check and the capacity limit work per flat component. `index.json` stores no key, so its format doesn't change. `LatestStoreImpl::list` still orders by (component, role, aspect), so `/v0.2`'s order and its `Filter` don't change; `list_latest_flat` keeps the entries whose flat component is listed and sorts them by it. Every index read keeps the newest entry for each flat component (`keep_newest`; on a tie the one listed first), because an index an earlier agent wrote can hold `arm`+`left` and `arm-left`; the dropped entry's content is then orphaned, and the worker's start deletes it as it does any orphan.
  Rationale: the control plane treats `arm`+`left` and `arm-left` as one component; keying the store the same way removes the hidden entries and the tie rule from `/v0.3`. Accepted consequence: `/v0.2` lists one entry instead of two only when one flat component was reported under two splits; it still forwards role and aspect on create and returns them as reported.
  Date/Author: 2026-11-16, design review; the user's choice.
- Decision: `/v0.3/version` reports the v0.3 crate's constants (`v0.3`, `v0.3.0-next`, openapi commit `36f5894...`); `/v0.2/version` keeps v0.2's. Nothing else reports a Device API version (the only other `ApiVersion` use, `daemon/src/http/request.rs`, is the backend API's).
  Date/Author: 2026-11-16, plan authoring.
- Decision: `server::routes::router` binds the merged table first (`let routes = v02::routes::table().merge(v03::routes::table());`) and then calls `middleware(routes.with_state(state))`, rather than one nested call.
  Rationale: the single expression exceeds rustfmt's call and chain widths, so rustfmt would split it over five lines; two lines read better and behave the same.
  Date/Author: 2026-11-16, source stage.
- Decision: ARCHITECTURE.md's generated-code paragraph and "Generated code is never hand-edited" invariant also name `libs/device-api-v03`, beside the planned `server` and calibrations edits.
  Rationale: they listed only `libs/device-api-v02`, which stops being true once the daemon depends on the v0.3 crate; README's `libs/` tree gets the same line in the plan.
  Date/Author: 2026-11-16, source stage.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Device API versions. Since agent#485 each version is one generated crate plus one module. `libs/device-api-v02` (crate `device_api_v02`; never edit `libs/`, it is generated) is imported in source as `use device_api_v02::models as device_server;` and in tests as `use device_api_v02::models as openapi;`. `daemon/src/server/v02/` holds:

- `mod.rs`: `mod enums; pub mod handlers; pub mod request; pub mod response; pub mod routes; pub mod sse;`
- `routes.rs`: `pub fn table() -> Router<Arc<State>>`, building every path as `/{api_version}/...` from `device_api_v02::models::ApiVersion::API_VERSION`.
- `handlers.rs`: one async fn per route; each calls a service in `daemon/src/services/` and returns `device_server::X::from(&domain)`. `version()` fills `VersionResponse` from `device_server::{ApiVersion, ApiGitCommit, ReleaseVersion}`. Utilities `handle`, `error_response`, `to_error_response`, `reject_body`, `reject_query`.
- `request.rs`: `impl TryFrom<device_server::CreateCalibrationRequest> for calibration::CreateParams` (rejects role or aspect `"null"`, the reserved `NULL_SLUG`, and an unknown format), `LatestCalibrationsQuery { component, role, aspect: Option<String> }` with `impl From<LatestCalibrationsQuery> for calibrations::Filter`, `pub(crate) fn split_list` (also used by `sse.rs`), `split_slugs`, and inline unit tests for `split_slugs`.
- `response.rs`: `From` impls from domain types to wire types, including `From<&ReportMetadata> for device_server::Calibration`, `pub fn latest_calibrations(&[ReportMetadata])` and `pub fn calibration_content(calibration::Content) -> Response`.
- `enums.rs`: `From` impls from domain enums to wire enums.
- `sse.rs`: `GET /events`, a server-sent event stream; it converts each stored event with `device_server::Event::try_from`.

`daemon/src/server/routes.rs` `router(state)` returns `middleware(v02::routes::table().with_state(state))`; `daemon/src/server/mod.rs` declares `pub mod v02;`. `daemon/src/server/state.rs` `State` holds the stores and actors every version shares, including `latest_cals: calibrations::LatestStore` and `cal_uploader`. AGENTS.md "Device API conversions": handlers don't convert wire types; response conversions go in `response.rs`, request conversions in `request.rs`.

The v0.3 contract. `diff -r libs/device-api-v02/src libs/device-api-v03/src` shows v0.3 is v0.2 except: `Calibration`, `CreateCalibrationRequest` and `LatestCalibrationsResponse` have no `role` or `aspect`; `CalSearch` has no `role`/`aspect` variants; `ApiVersion` is `v0.3`, `ReleaseVersion` `v0.3.0-next`, `ApiGitCommit` `36f589410c2088909607a1f68ccde69ccabb8fc0`. In `api/specs/device/v03.yaml`, `GET /calibrations/latest` takes only `component` (comma-separated, `explode: false`) and is "the latest calibration for each component"; the content route says "An older report returns 404". (`FileRuleUpload.path`'s description also changed; the agent does nothing with it.)

The control plane (Agent API v0.5, `libs/backend-api`, unchanged): create still takes `component`, `role`, `aspect`; the control plane joins them with `-`, skipping empty ones, to find the flat component, and stores and returns `component_slug`/`role_slug`/`aspect_slug` as reported. Its latest list is "the calibration with the latest `reported_at` for each component ..., where a report's `component`, `role` and `aspect` together name one component, breaking ties by `created_at`". This plan calls that joined slug the flat component: `arm` with role `left` and no aspect is `arm-left`.

Calibrations in the agent. `daemon/src/calibrations/report.rs` `ReportMetadata { report_id, component, format, role: Option<String>, aspect: Option<String>, serial_number, reported_by, deployment_id, reported_at }`, with `pub(crate) fn flat_component()` and `is_as_new_as` (same flat component, reported at the same time or later), and inline unit tests at the bottom. `calibrations::LatestStore` (`latest.rs`, an actor over `latest_impl.rs`) keeps one entry per flat component; `list(&Filter)` returns the entries `Filter { component, role, aspect }` matches (stored values), ordered by (component, role, aspect); `get(report_id)`. `daemon/src/services/calibration/` has `create(cal_uploader, deployments, CreateParams, now)`, `list_latest(store, &Filter)` (`latest.rs`) and `get_content(store, report_id)` (`content.rs`); `services/calibration/.covgate` is 100.00. The sender uploads `role`/`aspect` from the metadata and omits them when `None`.

Tests. `daemon/tests/server/v02/` mirrors the module: `handlers.rs` (route tests through `routes::router` with a `Fixture`; they check status codes and bodies only), `request.rs`, `response.rs`, `enums.rs` (uses `crate::models::harnesses::assert_converts_each`), `sse.rs`; `daemon/tests/server/mod.rs` lists the submodules; `daemon/tests/server/routes.rs` tests the merged router. Service tests: `daemon/tests/services/calibration/latest.rs` with `helpers::{Fixture, params}` (`Fixture.latest` is the `LatestStore`; `Fixture.create` stamps the fixed `received_at()`). `crate::test_utils::calibrations::reported(component, role, aspect)` builds a `ReportMetadata`.

Build and lint config. `daemon/Cargo.toml` depends on `device-api-v02 = { workspace = true }` only; the workspace already declares `device-api-v03`. `.lint-imports.toml` lists `internal_crates = ["backend_api", "device_api_v02", "miru_daemon"]`; the CI import linter fails a `device_api_v03` import outside the "internal crates" group unless it is listed. CI builds with `--locked`, so `Cargo.lock` must gain `device-api-v03` in `miru-daemon`'s dependencies.

CI: `.github/workflows/ci.yml` on pull requests runs `lint` (`scripts/lint.sh`: import linter, function length, field-by-field asserts, `cargo fmt --check`, clippy `-D warnings`, machete, audit), `test` (`scripts/covgate.sh`: the suite plus each `.covgate`; a `.covgate` covers its directory and subdirectories), `windows-check` and `windows-agent-build`.

## Plan of Work

### M1: source

Dependencies and lint config. In `daemon/Cargo.toml`, add `device-api-v03 = { workspace = true }` after `device-api-v02`, and refresh `Cargo.lock` with the command in Concrete Steps (never by hand). In `.lint-imports.toml`, add `"device_api_v03"` after `"device_api_v02"`.

The join. In `daemon/src/calibrations/report.rs`, in `impl ReportMetadata` after `is_as_new_as`, add:

    /// The component the control plane files this report under: the
    /// component, role and aspect joined with `-`, skipping missing or empty ones.
    pub(crate) fn flat_component(&self) -> String {
        let parts = [Some(self.component.as_str()), self.role.as_deref(), self.aspect.as_deref()];
        let parts: Vec<&str> = parts.into_iter().flatten().filter(|part| !part.is_empty()).collect();
        parts.join("-")
    }

The v0.3 read path. In `daemon/src/services/calibration/latest.rs`, after `list_latest`, add `pub async fn list_latest_flat(store: &LatestStore, components: &[String]) -> Result<Vec<ReportMetadata>, ServiceErr>`. It calls `store.list(&Filter::default())`, keeps the entries whose `flat_component()` is in `components` (an empty slice keeps all), and sorts them by flat component. The store holds one entry per flat component (Decision Log), so nothing needs collapsing. Doc comment: the latest calibration for each flat component, as the control plane keys them; answers from the store and never calls the control plane.

The v0.3 module. Copy `daemon/src/server/v02/*.rs` and `.covgate` to `daemon/src/server/v03/` (commands below), replace `device_api_v02` with `device_api_v03` and `v02::` with `v03::`, and change the comment in `response.rs` above `to_event_data` to say v0.3. Then make only these v0.3 differences:

- `request.rs`: the `TryFrom<device_server::CreateCalibrationRequest> for calibration::CreateParams` keeps only the format check and sets `component: request.component`, `role: None`, `aspect: None`; its doc comment says it checks the request's shape and the control plane judges the rest. Delete `NULL_SLUG`, `split_slugs` and the inline `mod tests` (they test `split_slugs`). `LatestCalibrationsQuery` keeps only `pub component: Option<String>`. Replace `impl From<LatestCalibrationsQuery> for Filter` with `impl LatestCalibrationsQuery { pub fn into_components(self) -> Vec<String> { split_list(self.component) } }` and drop the `Filter` import. Keep `to_instance_format` and `pub(crate) fn split_list` as they are.
- `handlers.rs`: `list_latest_calibrations` does `let components = query.into_components();` then `calibration_svc::list_latest_flat(&state.latest_cals, &components).await?` and returns `response::latest_calibrations(&latest)`. Every other handler is the copy as is; `version()` now reads the v0.3 constants through the `device_server` alias.
- `response.rs`: `From<&ReportMetadata> for device_server::Calibration` sets `component: metadata.flat_component()` and has no `role` or `aspect`. Everything else is the copy.
- `routes.rs`, `enums.rs`, `sse.rs`, `mod.rs`: the copy with the renames only.

Mount it. In `daemon/src/server/mod.rs` add `pub mod v03;` after `pub mod v02;`. In `daemon/src/server/routes.rs`, import `v03` beside `v02` and build `middleware(v02::routes::table().merge(v03::routes::table()).with_state(state))`; the doc comment on `router` stays true.

Docs. `ARCHITECTURE.md`: in the `server` paragraph, say each Device API version is a module (`server/v02/`, `server/v03/`) and that the router serves both until a later release drops `/v0.2`; in the calibrations paragraphs, add that `POST /v0.3/calibrations` takes only `component` (queued with no role or aspect), and that `GET /v0.3/calibrations/latest` lists each entry under its flat component (component, role and aspect joined with `-`), filtering by it (`services::calibration::list_latest_flat`), and that the store is keyed by flat component, so a report under another split replaces the entry, and every read keeps the newest of an older agent's duplicates. `README.md`: under `libs/`, add `device-api-v03/  #   Device API v0.3 server`.

### M2: tests

Mirror `daemon/tests/server/v02/` as `daemon/tests/server/v03/` (copy, then the same renames, plus `/v0.2/` to `/v0.3/` in URIs), declare `pub mod v03;` in `daemon/tests/server/mod.rs`, then adapt:

- `enums.rs`, `sse.rs`: renames only. The SSE contract test already deserializes data into the version's `openapi::Event`.
- `handlers.rs`: `health`, `version_tests` (now asserting the v0.3 constants; `api_release_version_extends_api_version` holds for `v0.3.0-next`), device, deployments, releases, git commits and file rules: renames only. In `mod calibrations`, `URI` is `/v0.3/calibrations`; `request()` sends `component: "arm-left"` and no role or aspect; delete `create_calibration_returns_422_for_a_null_role`; `list_latest_calibrations_returns_200_with_the_calibrations_it_matches` reports `arm-left` and `leg`, lists `?component=arm-left,hand`, and expects only the `arm-left` report; `list_latest_calibrations_returns_422_for_a_repeated_filter` uses `?component=a&component=b`. All other calibration route tests are the copy.
- `request.rs`: `create_calibration_request::converts_every_field` expects `role: None, aspect: None`; keep `maps_each_known_format` and `rejects_an_unknown_format`; delete `accepts_a_missing_role_and_aspect` and `rejects_a_null_role_or_aspect`. `latest_calibrations_query`: `splits_comma_separated_values` asserts `into_components()` of `"arm, leg,,"` is `["arm", "leg"]`, and `an_empty_query_filters_nothing` asserts an empty vec; delete `maps_a_null_role_or_aspect_to_none`.
- `response.rs`: `latest_calibrations_response::converts_report_metadata` builds metadata with component `arm`, role `left`, aspect `None` and expects `component: "arm-left"`; keep `converts_latest_calibrations_in_order` and the rest of the file as the copy.

Router: in `daemon/tests/server/routes.rs` `mod router`, add `serves_each_version`: `GET /v0.2/health` and `GET /v0.3/health` through `routes::router` both return 200.

Service: in `daemon/tests/services/calibration/latest.rs`, add, putting entries with `f.latest.put(metadata, data)` and `reported(...)` where a test needs chosen keys or times:

- `list_latest_flat_orders_by_flat_component`: entries `arm`/None/`pose` and `arm`/`left`/None (which the store lists in that order) give `arm-left`, then `arm-pose`, each the stored metadata unchanged.
- `list_latest_flat_matches_the_flat_component`: with `arm`/`left`/None stored, `["arm-left"]` lists it and `["arm"]` lists nothing.
- `list_latest_flat_fails_when_the_store_cannot_be_read`: as the `list_latest` sibling, with `corrupt_latest_index`.

Store: in `daemon/tests/calibrations/latest.rs`, add `put_replaces_the_entry_of_another_split_of_its_key` (`arm`/`left`/None, then `arm-left`/None/None, lists only the second and keeps only its content) and `put_if_newer_keeps_a_newer_entry_of_another_split` (the reverse, with the stored entry 1 s later). `spawn_keeps_only_the_newest_of_two_splits_of_one_key` (an `index.json` holding both splits) landed with the design-review revision.

Unit: in `daemon/src/calibrations/report.rs` `mod tests`, add a `FLAT COMPONENT` section with `flat_component_joins_the_parts_it_has`, a table over `reported(...)`: (`arm`, None, None) is `arm`; (`arm`, `left`, None) is `arm-left`; (`arm`, None, `pose`) is `arm-pose`; (`arm`, `left`, `pose`) is `arm-left-pose`; (`arm`, `""`, `pose`) is `arm-pose`.

## Concrete Steps

All commands run from the agent repo root on branch `feat/device-api-v03`.

M1:

    sed -i 's/^device-api-v02 = { workspace = true }$/&\ndevice-api-v03 = { workspace = true }/' daemon/Cargo.toml
    cargo update --workspace --offline     # lockfile refresh only; if offline fails, drop --offline
    git diff --stat Cargo.lock             # expect: 1 file changed, 1 insertion(+)
    sed -i 's/"device_api_v02", /"device_api_v02", "device_api_v03", /' .lint-imports.toml
    mkdir -p daemon/src/server/v03
    cp daemon/src/server/v02/*.rs daemon/src/server/v02/.covgate daemon/src/server/v03/
    sed -i 's/device_api_v02/device_api_v03/g; s/v02::/v03::/g; s/v0\.2/v0.3/g' daemon/src/server/v03/*.rs

Then make the edits in Plan of Work (report.rs, services latest.rs, the v03 differences, server mod.rs and routes.rs, docs), and commit:

    git add daemon/Cargo.toml Cargo.lock .lint-imports.toml daemon/src/calibrations/report.rs \
      daemon/src/services/calibration/latest.rs daemon/src/server/mod.rs daemon/src/server/routes.rs \
      daemon/src/server/v03 ARCHITECTURE.md README.md
    git commit -m "feat(server): serve Device API v0.3 beside v0.2"   # plus the session trailers

M2:

    mkdir -p daemon/tests/server/v03
    cp daemon/tests/server/v02/*.rs daemon/tests/server/v03/
    sed -i 's/device_api_v02/device_api_v03/g; s/v02::/v03::/g; s/v0\.2/v0.3/g' daemon/tests/server/v03/*.rs

Then make the test edits in Plan of Work and commit:

    git add daemon/tests/server daemon/tests/services/calibration/latest.rs daemon/src/calibrations/report.rs
    git commit -m "test(server): cover Device API v0.3"   # plus the session trailers

Never run cargo build, check, test, clippy or fmt, or the repo's lint and test scripts, locally; CI runs them.

## Validation and Acceptance

Validation runs in CI on the pushed head: `$preflight` pushes the branch and watches `.github/workflows/ci.yml`. The PR leaves draft, and the task is reported complete, only when `$preflight` reports `CLEAN`, meaning every job is green on the pushed branch head:

- `lint`: imports (with `device_api_v03` internal), function length, field-by-field asserts, fmt, clippy `-D warnings`, machete (the new dependency is used).
- `test`: the full suite plus coverage gates, including the new `daemon/src/server/v03/.covgate` (87), `daemon/src/server/.covgate` (87), `daemon/src/services/calibration/.covgate` (100.00) and `daemon/src/calibrations/.covgate` (99.64).
- `windows-check`, `windows-agent-build`: the build with `--locked`, which proves `Cargo.lock` is current.

Acceptance, each proven by a named test:

- Every `/v0.3/...` route answers as its `/v0.2` twin does, with v0.3 types: `daemon/tests/server/v03/handlers.rs`, `sse.rs`; both versions are mounted together: `server::routes::router::serves_each_version`.
- `GET /v0.3/version` reports `api_version` `v0.3` and `api_release_version` `v0.3.0-next`: `server::v03::handlers::version_tests`.
- `POST /v0.3/calibrations` queues the report with role and aspect `None`: `server::v03::request::create_calibration_request::converts_every_field`.
- `/v0.3` reads return the joined slug, one per flat component, ordered and filtered by it: `services::calibration::latest::list_latest_flat_*`, `server::v03::response::latest_calibrations_response::converts_report_metadata`, `calibrations::report::tests::flat_component_joins_the_parts_it_has`.
- `/v0.2` is unchanged except that two splits of one flat component are one entry: every existing `daemon/tests/server/v02/` test passes without edits; the store keeps one entry per flat component: `calibrations::latest::{put_replaces_the_entry_of_another_split_of_its_key, put_if_newer_keeps_a_newer_entry_of_another_split, spawn_keeps_only_the_newest_of_two_splits_of_one_key}`.

## Idempotence and Recovery

The copy and `sed` steps assume `server/v03/` does not exist yet; to redo them, `git checkout -- daemon/src/server/v03 daemon/tests/server/v03` or delete those directories and copy again. `cargo update --workspace` only touches workspace members and is safe to rerun; if `Cargo.lock` shows more than the one added line, `git checkout -- Cargo.lock` and rerun with `--offline`. Each milestone is one commit, so `git revert` of either undoes it cleanly. Until M2 lands, the `test` job fails the v03 coverage gate; that is expected on the source-only head.
