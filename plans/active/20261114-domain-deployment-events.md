# Store deployment events as domain structs, not v0.2 wire structs

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Define the two deployment event structs in `agent/src/events/model.rs` over the domain status enums; pin their JSON against the v0.2 generated structs in tests; drop two stale route comments |

This plan lives in `agent/plans/` because every change is in the agent repo. Paths are relative to the agent repo root (the directory holding the workspace `Cargo.toml` and `ARCHITECTURE.md`); the crate is in `agent/`. Branch: `refactor/domain-deployment-events`, based on `main` at `4ca00ca2` (agent#485, "host Device API versions side by side"). It is PR 1 of two; PR 2 (`refactor/version-neutral-status-enums`, plan `plans/active/20261114-version-neutral-status-enums.md`) stacks on it and removes the v0.2 twins from the status enums.

## Purpose / Big Picture

The agent serves a local HTTP API, the Device API, to programs on the same device. Since agent#485 each Device API version is one generated crate (`libs/device-api-v02`, imported as `device_api_v02`) plus one module (`agent/src/server/v02/`). A later `/v0.3` will be a full copy of `server/v02/` over its own crate.

The event store is version-specific today: `agent/src/events/model.rs` builds each stored deployment event from a v0.2 generated struct. After this change it builds them from its own structs over the domain status enums, and `events/` no longer imports `device_api_v02`. Nothing a client sees changes: newly stored events have byte-for-byte the same JSON, events stored before the upgrade read back as before, and `GET /v0.2/events` streams the same data.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `events::model::DeploymentDeployedEvent`, `events::model::DeploymentRemovedEvent` (now structs, were aliases of the v0.2 structs) | `DeployedEvent` / `RemovedEvent`; `DeploymentDeployedData` / `DeploymentRemovedData` | Same public names as today, so `EventArgs::deployed`/`removed` keep their shape; they match the event type strings `deployment.deployed`/`deployment.removed` and the schema names whose JSON they reproduce. |

### Precedent

No new files. The two structs mirror the generated `libs/device-api-v02/src/models/deployment_deployed_event.rs` and `deployment_removed_event.rs`: same fields, order and `skip_serializing_if`, with domain enums in place of the v0.2 enums.

### Left out

- `Deserialize` on the new structs: the store keeps event data as `serde_json::Value` and nothing reads it back typed; the tests read it with the v0.2 structs.
- Storing `deployed_at`/`archived_at` as `DateTime<Utc>`: chrono serializes `...Z` where `to_rfc3339()` writes `...+00:00`, which would change the stored bytes.
- Removing `device_api_v02` from `agent/tests/events/model.rs`: the test pins the stored JSON to the v0.2 schema on purpose, because `/v0.2/events` serves it as is.
- The status enums' `agent_type` twins and `impl_status_enum!`: PR 2. The new structs hold domain enums directly, so they do not use the twins.

### Open questions

- Keep the structs `pub`? Default: yes, with `pub` fields, like `Event` and `EventArgs` beside them; they are the stored shape of a public event.

## Progress

- [x] M0: commit the stale route-order comment removal in `agent/src/server/v02/routes.rs`.
- [x] M1: source: event structs in `agent/src/events/model.rs`.
- [x] Source written
- [ ] M2: tests: `agent/tests/events/model.rs` uses the domain structs, and `agent/tests/server/v02/response.rs` covers the v0.2 event conversion, including its error paths.
- [ ] CI green on the pushed head (`$preflight` reports `CLEAN`).

## Surprises & Discoveries

- Until M2 lands, `agent/tests/events/model.rs` does not compile: it builds `miru_agent::events::model::DeploymentDeployedEvent` from v0.2 enums and reads data back into it, and the new structs hold domain enums and do not derive `Deserialize`. M2's import change fixes both, so a CI run on the source-only head fails the `test` job.

## Decision Log

- Review (agent#487, ben-miru): the payload must be compiler-checked against the v0.2 contract, not only tested. The author chose per-version conversion at serve time: the event structs are `pub` and derive `Deserialize`, and `server/v02/response.rs` replaces `From<&Event> for device_server::Event` with a `TryFrom` that parses `data` into the domain struct for its `event_type` and builds the generated v0.2 struct (`From<&events::model::DeploymentDeployedEvent>` and `...RemovedEvent`), so every field and enum mapping is checked by the compiler in the version that owns the contract. An unknown event type or unparseable data is an error, and the SSE stream skips that event with a warning, as it already did for a serialization failure. `/v0.3` writes its own conversion. The src comment no longer names a version.
- AI design review (D4): parsing stored data by `event_type` belongs to the event-log format, not to each version. `events::model` gets `pub enum EventPayload { Deployed(..), Removed(..) }` and `Event::payload() -> Result<EventPayload, EventsErr>`, with a new `UnknownEventTypeErr` variant for a type it doesn't know. Each version's `response.rs` matches `EventPayload` exhaustively, so a new event type is a compile error in every version module. The SSE stream converts through a small `to_sse_data` helper and still skips an event that fails, with a warning.
- AI design review (D8): suggested storing `Event` with a typed `EventPayload` instead of `data: Value`. Kept parse-at-serve (author's call). A typed log loader would drop lines of an event type this build doesn't know, such as one written before a downgrade, and the store picks the next event ID from the lines it kept, so a dropped newest event would let its ID be reused. Typed storage, with a fix for that, is a follow-up.
- Design review: the stale route-order comments in `server/v02/routes.rs` ship here because they were raised in agent#485's review after its last push, and this is the next PR on top of it.

(Add entries as you go.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

`agent/src/events/model.rs` imports `device_api_v02::models as device_server` and declares `pub type DeploymentDeployedEvent = device_server::DeploymentDeployedEvent;` and the same for `DeploymentRemovedEvent`. `EventArgs::deployed`/`removed(deployment: &models::Deployment)` fill them with `deployment_id`, `release_id`, `status: (&deployment.status()).into()`, `activity_status`, `error_status`, `target_status` (each `(&deployment.x).into()`), and `deployed_at` (resp. `archived_at`) `: deployment.<field>.map(|dt| dt.to_rfc3339())`. The `.into()` calls use conversions that `impl_status_enum!` (`agent/src/models/status.rs`) generates from its `agent_type` clause. `EventArgs::new` stores the struct as `serde_json::Value` (`serde_json::to_value`). The store (`agent/src/events/store.rs`) persists `Event { id, type, occurred_at, data: Value }`; the v0.2 SSE handler (`agent/src/server/v02/sse.rs`) sends `data` unchanged. Old events on disk are `Value`s, never read as these structs, so changing the structs cannot break reading them; it can only change newly written bytes. The only production caller is `agent/src/sync/deployments.rs`.

Domain types (`agent/src/models/deployment.rs`, re-exported from `crate::models`): `DplStatus`, `DplActivity`, `DplErrStatus`, `DplTarget`, all `Copy`, each deriving `Serialize` with `#[serde(rename_all = "snake_case")]`; `Deployment::status()` returns a `DplStatus` by value. The generated v0.2 structs have fields `deployment_id: String`, `release_id: String`, `status`, `activity_status`, `error_status`, `target_status`, and `deployed_at` (resp. `archived_at`) `: Option<String>` with `skip_serializing_if = "Option::is_none"`. Each v0.2 enum renames every variant to the same lowercase string as the domain enum (`drifted`, `staged`, `queued`, `deployed`, `removing`, `archived`, `failed`, `retrying`, `none`), plus an `#[serde(other)]` `...Unknown` variant used only when parsing. `serde_json` is built without `preserve_order`, so a `Value` object sorts its keys; keep the field order anyway.

`agent/tests/events/model.rs` builds expected data with `serde_json::json!(DeploymentDeployedEvent { ... })` (imported from `miru_agent::events::model`) using v0.2 enums imported from `device_api_v02::models`. Its `deployed_at_none_when_absent` and `archived_at_none_when_absent` only parse data back, so they do not prove the key is omitted rather than `null`.

Coverage: `agent/src/events/.covgate` is 93.06; never lower it.

CI: `.github/workflows/ci.yml` on pull requests runs `lint` (import-order linter, `cargo fmt --check`, clippy `-D warnings`, machete, audit), `test` (suite plus coverage gates), `windows-check`, `windows-agent-build`.

Uncommitted at plan time: `agent/src/server/v02/routes.rs` deletes two stale comments "/current before /{id} so ... isn't captured" (axum 0.8 on matchit 0.8 always prefers a static segment). It ships with this PR.

## Plan of Work

M0. Commit the existing working-tree change to `agent/src/server/v02/routes.rs` on its own.

M1, source. In `agent/src/events/model.rs`, delete the `device_api_v02` import and replace the two aliases with:

    // Stored as is and served by /v0.2/events, so this JSON must not change.
    #[derive(Debug, Serialize)]
    pub struct DeploymentDeployedEvent {
        pub deployment_id: String,
        pub release_id: String,
        pub status: models::DplStatus,
        pub activity_status: models::DplActivity,
        pub error_status: models::DplErrStatus,
        pub target_status: models::DplTarget,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub deployed_at: Option<String>,
    }

and `DeploymentRemovedEvent` with `archived_at` in place of `deployed_at`. In `EventArgs::deployed`/`removed`, use `deployment.status()` and `deployment.activity_status`, `deployment.error_status`, `deployment.target_status` directly instead of `.into()`; keep the `to_rfc3339()` mapping.

M2, tests. In `agent/tests/events/model.rs`, build expected data from the domain event structs (`miru_agent::events::model::DeploymentDeployedEvent` and `DeploymentRemovedEvent`, now `pub` and `Deserialize`) with domain enums, and drop the `device_api_v02` import; `deployed_at_none_when_absent` / `archived_at_none_when_absent` compare the whole `event.data` with a `json!` literal that has no timestamp key, which proves the key is omitted. Also in `agent/tests/events/model.rs`, test `Event::payload()`: each event type parses to its `EventPayload` variant, an unknown `event_type` returns `EventsErr::UnknownEventTypeErr`, and data that doesn't parse returns `EventsErr::SerializationErr`. In `agent/tests/server/v02/response.rs`, add an `event_response` module: `device_server::Event::try_from(&event)` for events from `EventArgs::deployed` and `EventArgs::removed` (timestamp set and unset) gives `data` equal to `serde_json::json!(device_server::DeploymentDeployedEvent { .. })` (resp. removed); an unknown `event_type` returns `Err`. In `agent/tests/server/v02/sse.rs`, if a fixture can append such an event, check the stream skips it.

## Concrete Steps

All commands run from the agent repo root, `/home/user/agent`, on `refactor/domain-deployment-events`.

M0:

    git add agent/src/server/v02/routes.rs
    git commit -m "refactor(server): drop stale route-order comments"

M1: edit `agent/src/events/model.rs`, then check:

    grep -n device_api_v02 agent/src/events/model.rs

Expected: no output.

    git add agent/src/events/model.rs
    git commit -m "refactor(events): store deployment events as domain structs"

M2: edit `agent/tests/events/model.rs`, then:

    git add agent/tests/events/model.rs
    git commit -m "test(events): pin stored deployment event JSON to the v0.2 schema"

Never run cargo, `scripts/lint.sh`, `scripts/test.sh` or `scripts/covgate.sh` locally; `$preflight` pushes and CI checks.

## Validation and Acceptance

Validation runs in CI (`.github/workflows/ci.yml`) on the pushed branch head and is complete only when `$preflight` reports `CLEAN`. The PR stays in draft, and the task is not reported complete, until then. Jobs `lint`, `test` (with `agent/src/events/.covgate` at or above 93.06), `windows-check` and `windows-agent-build` must pass.

What the green run demonstrates: `agent/tests/events/model.rs` `deployment_deployed::serializes_all_fields` and `deployment_removed::serializes_all_fields` pass with expected JSON built from the v0.2 generated structs, so newly stored data, which `/v0.2/events` streams as is, is unchanged; `deployed_at_none_when_absent` and `archived_at_none_when_absent` show the timestamp key is omitted when absent; `agent/tests/server/v02/sse.rs` passes unchanged for `/v0.2/events`; `agent/tests/events/store.rs` passes unchanged, so stored events read back.

## Idempotence and Recovery

Every step is a file edit plus a commit; re-applying an edit changes nothing. If CI fails, fix from `gh run view --log-failed` and push again. If the events coverage gate drops, add tests; never lower a `.covgate`. No on-disk format changes, so there is nothing to migrate or roll back on devices.
