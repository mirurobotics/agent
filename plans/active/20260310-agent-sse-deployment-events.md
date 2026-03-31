# Add SSE Deployment Event Stream with Replay

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | New `events` module, SSE handler, syncer/deployment event emission, OpenAPI spec |

This plan lives in `agent/.ai/exec-plans/` because all code changes are within the agent repository. Work happens on the `feat/sse` branch.

## Purpose / Big Picture

Add a streaming endpoint on the Miru agent Unix socket so local consumers (CLI, frontend) can subscribe to deployment lifecycle updates in real time and reliably catch up after reconnects.

After this work, a consumer can call `GET /v0.2/events` with `Accept: text/event-stream` and receive deployment-related events as they happen, then reconnect with `Last-Event-ID` (or `?after=`) to replay missed events.

A developer can verify it works by running the agent and connecting:
```bash
curl -N --unix-socket /run/miru/miru.sock \
  -H "Accept: text/event-stream" \
  "http://localhost/v0.2/events?limit=5"
```

This plan targets production-safe semantics: monotonic event IDs, at-least-once delivery, durable local event history, explicit replay window behavior, and predictable event envelope versioning.

### Background

A previous implementation existed on the `sse` branch but diverged significantly from `main` due to API library renames (`openapi-*` → `backend-api`/`device-api`), import normalization, shutdown manager refactoring, and syncer cleanup. Rather than rebasing, we are reimplementing from scratch on `main`, reusing the proven design from the `sse` branch.

## Progress

- [x] (2026-03-10) Drafted new ExecPlan based on learnings from `sse` branch implementation and current `main` codebase.
- [ ] M1: Event module — model, durable store, and error types.
- [ ] M2: Event hub — broadcast coordinator combining store + broadcast channel.
- [ ] M3: SSE HTTP handler and route wiring.
- [ ] M4: Integration — wire EventHub into app state, server state, and syncer event emission.
- [ ] M5: OpenAPI spec updates — tracked separately in `openapi/.ai/exec-plans/active/20260310-device-api-sse-events-endpoint.md`.

## Surprises & Discoveries

- Observation: axum 0.8.3 (current workspace dependency) includes SSE support natively via `axum::response::sse::Sse` — no additional feature flag required.
  Evidence: `agent/Cargo.toml:21` `axum = { version = "0.8.3" }`.

- Observation: The sync subsystem has an internal event broadcast (`SyncEvent`) via `tokio::sync::watch`, which publishes `SyncSuccess`, `SyncFailed`, and `CooldownEnd` events. This is a natural hook for SSE sync events.
  Evidence: `agent/src/sync/syncer.rs:37-48` (`SyncEvent` enum) and `syncer.rs:93-94` (`subscriber_tx`/`subscriber_rx` watch channel).

- Observation: There is no observer pattern in the deploy module. Deployment state transitions happen via pure functions (`fsm::deploy()`, `fsm::remove()`, `fsm::error()`) called from `deploy/apply.rs`, and outcomes are processed in `sync/deployments.rs::apply_deployments()`.
  Evidence: `agent/src/deploy/mod.rs` lists only `apply`, `errors`, `filesys`, `fsm` submodules. `deploy/apply.rs:30-34` defines `Outcome { deployment, wait, error }`.

- Observation: The sync cycle in `sync/deployments.rs::sync()` follows a pull-apply-push pattern. The `apply_deployments()` function (lines 283-324) iterates over `Outcome` structs returned by `deploy::apply::apply()`, making it the ideal point to emit per-deployment SSE events.
  Evidence: `agent/src/sync/deployments.rs:283-324`.

- Observation: Existing file helpers in `filesys/` are overwrite-oriented (atomic replace). Event journaling needs explicit append I/O — the store will use `std::fs` directly since it lives behind a `tokio::sync::Mutex`.
  Evidence: `agent/src/filesys/file.rs` write methods use atomic overwrite.

- Observation: The `server::State` struct holds `storage`, `http_client`, `syncer`, `token_mngr`, and `activity_tracker`. It is constructed in `app/run.rs::init_socket_server()` (line 300-306).
  Evidence: `agent/src/server/state.rs:12-18` and `agent/src/app/run.rs:300-306`.

- Observation: API version routing uses `device_api::models::ApiVersion::API_VERSION` (dynamic constant), not a hardcoded string.
  Evidence: `agent/src/server/serve.rs:47`.


## Decision Log

- Decision: Reimplement SSE from scratch on `main` rather than rebasing the `sse` branch.
  Rationale: The `sse` branch diverged across 50+ commits with structural changes (API renames, import normalization, shutdown manager refactoring, macro-based status enums). A clean reimplementation is less error-prone and produces cleaner history.
  Date/Author: 2026-03-10

- Decision: Use SSE over the existing local socket server (`GET /v0.2/events`) rather than adding a second transport.
  Rationale: Reuses existing runtime, middleware, and lifecycle management; no extra process/listener complexity.
  Date/Author: 2026-02-18 (carried forward)

- Decision: Define one canonical JSON event envelope and send it unchanged inside SSE `data`.
  Rationale: Keeps payload compatibility with future webhook or queue transports and avoids transport-specific schemas.
  Date/Author: 2026-02-18 (carried forward)

- Decision: Use monotonic numeric event IDs and support both `Last-Event-ID` and `?after=<id>` replay cursors.
  Rationale: Browser-native SSE reconnection uses `Last-Event-ID`; explicit query cursor supports non-browser clients and debugging.
  Date/Author: 2026-02-18 (carried forward)

- Decision: Store events durably in local append-only JSONL with bounded retention, and keep a small in-memory pub/sub channel for low-latency fanout.
  Rationale: JSONL is simple to inspect/repair on devices; in-memory fanout avoids disk polling for live streams.
  Date/Author: 2026-02-18 (carried forward)

- Decision: Emit deployment events from `sync/deployments.rs::apply_deployments()` after processing each `Outcome`, rather than from a deploy observer (which does not exist in this codebase).
  Rationale: `apply_deployments()` is where deployment outcomes are iterated and already has access to the full `Outcome` struct.
  Date/Author: 2026-03-01 (carried forward)

- Decision: Do not emit sync-level events (`sync.completed`, `sync.failed`). Only emit deployment events.
  Rationale: Sync events are implementation details of how the agent updates state. Consumers care about what changed (deployment events), not that a sync cycle ran. The agent already has `GET /health` for health monitoring. Sync events can be added later if a real use case emerges.
  Date/Author: 2026-03-10

- Decision: Pass `EventHub` through `SyncerArgs` so the syncer can thread it to deployment sync for event emission.
  Rationale: The syncer is single-threaded by design (actor pattern); giving it direct access to the EventHub avoids cross-thread complexity.
  Date/Author: 2026-03-01 (carried forward)

- Decision: Version in the event type string (e.g. `deployment.deployed.beta1`) rather than a separate `schema_version` envelope field.
  Rationale: Event types evolve independently — changing the `deployment.deployed` payload shouldn't affect `sync.completed` consumers. Embedding the version in the type string is more explicit, avoids the "what does schema_version scope to" ambiguity (per-envelope vs per-event-type), and lets consumers subscribe to specific versions during migration. A global `schema_version` field is dropped from the envelope.
  Date/Author: 2026-03-10

- Decision: Support a `types` query parameter for server-side event filtering.
  Rationale: This is a public API — third-party applications connect to the agent socket. Consumers should be able to subscribe to only the event types they care about to avoid noise. Comma-separated exact match (e.g. `?types=deployment.deployed.beta1,sync.completed.beta1`). If omitted, all events are sent.
  Date/Author: 2026-03-10

- Decision: Use `.jsonl` file extension over `.ndjson` for the event log.
  Rationale: Both formats are identical (one JSON object per line). `.jsonl` is preferred as a cosmetic choice.
  Date/Author: 2026-03-10

- Decision: Drop `device_id` and `subject` from the event envelope.
  Rationale: No concrete consumer use case for either field yet. `device_id` is redundant when there's one agent per socket. `subject` is natural for deployment events but awkward for system-level events like `sync.completed`. Starting minimal — if consumers need resource filtering or device identification, add them in a v2 of the relevant event types.
  Date/Author: 2026-03-10

- Decision: Use `std::fs` (synchronous) for the event store rather than `tokio::fs`.
  Rationale: The store is behind a `tokio::sync::Mutex` so blocking I/O happens while holding the lock. Sync I/O is simpler and the writes are small (single JSONL lines). The mutex hold time is negligible.
  Date/Author: 2026-03-10

## Outcomes & Retrospective

Not started. This section will be filled when milestones are implemented and validated.

## Context and Orientation

This work happens in the `agent` submodule at `./agent` (repo root for implementation commands below).

Key current components:

- `agent/src/server/serve.rs`: Axum router with Unix socket server. Routes use `device_api::models::ApiVersion::API_VERSION`. Handles systemd socket activation via `LISTEN_FDS` and direct socket binding.
- `agent/src/server/handlers.rs`: HTTP handlers for health, version, device, deployments, releases, and git commits.
- `agent/src/server/state.rs`: `server::State` struct with fields: `storage`, `http_client`, `syncer`, `token_mngr`, `activity_tracker`.
- `agent/src/app/state.rs`: `AppState` struct with same five fields. `AppState::init()` creates storage, token manager, syncer, and activity tracker.
- `agent/src/app/run.rs`: Orchestrates startup. `ShutdownManager` uses `register_handle()` with closures. `init_socket_server()` constructs `server::State::new(...)` at line 300.
- `agent/src/storage/layout.rs`: `Layout` struct with `filesystem_root`. `root()` returns `/var/lib/miru/`. No events methods yet.
- `agent/src/sync/syncer.rs`: Single-threaded syncer using actor pattern (mpsc channel). `SingleThreadSyncer` holds `watch::Sender<SyncEvent>`. `SyncerArgs` has: `storage`, `http_client`, `token_mngr`, `deploy_opts`, `backoff`, `agent_version`.
- `agent/src/sync/deployments.rs`: `SyncArgs` has: `http_client`, `storage`, `opts`, `token`. `apply_deployments()` calls `deploy::apply::apply()` which returns `Vec<Outcome>`.
- `agent/src/deploy/apply.rs`: `Outcome` struct: `deployment: models::Deployment`, `wait: Option<TimeDelta>`, `error: Option<DeployErr>`.
- `agent/src/errors/mod.rs`: `Error` trait with `code()`, `http_status()`, `params()`, `is_network_conn_err()`. `impl_error!` macro for enum delegation.

Terms:

- SSE (Server-Sent Events): One-way HTTP stream where server emits newline-delimited event frames (`id`, `event`, `data`).
- Replay cursor: Last event ID a client has processed; server returns only newer events.
- At-least-once delivery: A client may see duplicates after reconnect; clients must dedupe by event ID.
- Event envelope: Versioned JSON object carrying metadata (`id`, `type`, `occurred_at`, `subject`) and payload (`data`).

## Interfaces and Dependencies

Planned event envelope (JSON fields):

- `object`: always `"event"` — identifies the envelope as an event object.
- `id`: monotonic `u64` rendered as string in SSE `id` field and JSON payload.
- `type`: versioned event type string (e.g. `deployment.deployed.beta1`, `sync.completed.beta1`, `sync.failed.beta1`). Version is embedded in the type rather than a separate field — event types evolve independently.
- `occurred_at`: RFC3339 UTC timestamp.
- `data`: event-specific object (contains resource IDs, status fields, etc. as needed per event type).

Planned storage files (under `Layout::root()/events/`, i.e. `/var/lib/miru/events/`):

- `events.jsonl`: append-only one-envelope-per-line history.
- `metadata.json`: small metadata file (`next_event_id`).

Planned endpoint contract:

- `GET /v0.2/events`
- Headers: `Accept: text/event-stream`, optional `Last-Event-ID: <u64>`
- Query params: optional `after=<u64>` (takes precedence over `Last-Event-ID`), optional `types=<comma-separated>` (server-side filter, e.g. `types=deployment.deployed.beta1,sync.completed.beta1`; if omitted, all events are sent)
- `200` + streaming body (`text/event-stream`) when cursor is valid.
- `400` for malformed cursor values.
- `410` when cursor is older than earliest retained event.

New dependency: `tokio-stream = "0.1"` (workspace-level).

## Plan of Work

### Milestone M1: Event Module — Model, Store, Errors

Create `agent/src/events/` with:

- `mod.rs`: public exports (`pub mod errors; pub mod hub; pub mod model; pub mod store;`).
- `errors.rs`: `EventsErr` enum, `CursorExpiredErr`, `MalformedCursorErr` structs. Implement `crate::errors::Error` with `http_status()` overrides (410 for CursorExpired, 400 for MalformedCursor). Use `crate::impl_error!` macro. `From` impls for `filesys::FileSysErr` and `serde_json::Error`.
- `model.rs`: `Envelope` struct with fields `object` (always `"event"`), `id`, `event_type` (serde rename `"type"`), `occurred_at`, `data`. No `schema_version`, `device_id`, or `subject` — keeping the envelope minimal. Factory methods: `deployment_deployed()`, `deployment_removed()`. Versioned event type constants: `DEPLOYMENT_DEPLOYED_BETA1`, `DEPLOYMENT_REMOVED_BETA1`.
- `store.rs`: `EventStore` with JSONL persistence. `init()` reads from disk tolerating malformed trailing lines. `append()` assigns monotonic ID, writes line, flushes, compacts when > max_retained. `replay_after(cursor)` returns all events with id > cursor. Compaction keeps 90% of max_retained via atomic rewrite. `DEFAULT_MAX_RETAINED = 10_000`.

Add `pub mod events;` to `agent/src/lib.rs` (after `errors`).

Add storage layout helpers to `agent/src/storage/layout.rs`:
- `events_dir()` → `self.root().subdir("events")`
- `events_log_file()` → `self.events_dir().file("events.jsonl")`
- `events_meta_file()` → `self.events_dir().file("metadata.json")`

Tests: `tests/events/{mod,model,store}.rs` — serde roundtrip, factory methods, monotonic IDs, replay, compaction, disk recovery, malformed line tolerance. Add `pub mod events;` to `tests/mod.rs`.

### Milestone M2: Event Hub (Broadcast Coordinator)

Create `agent/src/events/hub.rs`:

- `EventHub` struct: `store: tokio::sync::Mutex<EventStore>`, `broadcast_tx: broadcast::Sender<Envelope>`.
- `new(log_file, meta_file, max_retained, broadcast_capacity)` — inits store, creates broadcast channel.
- `publish()` — lock store, append (assigns ID), broadcast (ignore lagged), return envelope.
- `replay_after(cursor)` — lock store, validate cursor expiration (CursorExpiredErr if cursor > 0 && cursor < earliest_id), replay all retained events after cursor.
- `subscribe()` — returns broadcast receiver.
- `try_publish()` — fire-and-forget, logs errors.
- `DEFAULT_BROADCAST_CAPACITY = 256`.

Tests: `tests/events/hub.rs` — publish IDs, subscribe, replay, cursor expiration, try_publish safety, multiple subscribers, lagged receiver. Add `pub mod hub;` to `tests/events/mod.rs`.

### Milestone M3: SSE HTTP Handler + Route

Add `tokio-stream = "0.1"` to workspace `Cargo.toml` and `tokio-stream = { workspace = true }` to agent `Cargo.toml`.

Create `agent/src/server/sse.rs`:
- `EventsQuery`: `after: Option<String>`, `types: Option<String>`.
- Handler resolves cursor from `after` query param (precedence) or `Last-Event-ID` header, defaults to 0. Returns 400 for malformed.
- Type filter: parse `types` as comma-separated into `HashSet<String>`. If present, skip events whose `event_type` is not in the set (applies to both replay and live stream).
- Strategy: subscribe to broadcast BEFORE replay → replay historical (filtered) → chain live stream (dedup by id <= last replayed, filtered) → heartbeat every 30s.
- Uses `axum::response::sse::{Event as SseEvent, KeepAlive, Sse}`, `tokio_stream`, `futures::stream`.

Modify:
- `agent/src/server/mod.rs` — add `pub mod sse;`.
- `agent/src/server/state.rs` — add `pub event_hub: Arc<crate::events::EventHub>` to `State`, update `new()`.
- `agent/src/server/serve.rs` — add `GET /{api_version}/events` route after git_commits section.

Tests: `tests/server/sse.rs` — cursor parsing, 400/410 errors, replay order, type filtering, SSE format. Add `pub mod sse;` to `tests/server/mod.rs`.

### Milestone M4: Integration — Wire EventHub into App + Syncer

Modify `agent/src/app/state.rs`:
- Add `pub event_hub: Arc<crate::events::EventHub>` to `AppState`.
- In `init()`, create EventHub using layout paths before syncer init, pass into `SyncerArgs`.

Modify `agent/src/app/run.rs`:
- In `init_socket_server()`, pass `app_state.event_hub.clone()` to `server::State::new()`.

Modify `agent/src/sync/syncer.rs`:
- Add `pub event_hub: Arc<crate::events::EventHub>` to `SyncerArgs` and `SingleThreadSyncer`.
- Store `event_hub: args.event_hub` in `SingleThreadSyncer::new()`.
- Pass `event_hub` ref into `deployments::SyncArgs` in `sync_impl()`.
- No sync-level event emission — only deployment events are published.

Modify `agent/src/sync/deployments.rs`:
- Add `pub event_hub: &'a crate::events::EventHub` to `SyncArgs`.
- In `apply_deployments()`, after each outcome, publish `deployment.deployed.beta1` or `deployment.removed.beta1` based on `activity_status` (only when `outcome.error.is_none()`).
- Thread `event_hub` from `sync_impl()` into `SyncArgs`.

Update existing syncer/deployment tests to include `event_hub` in args (test helper for temp-dir-backed EventHub).

### Milestone M5: OpenAPI Spec Updates

Tracked separately in `openapi/.ai/exec-plans/active/20260310-device-api-sse-events-endpoint.md`.

## Validation and Acceptance

Behavior is accepted when all of the following are true:

1. Live stream: `GET /v0.2/events` over the agent Unix socket returns `200` and stays open, sending heartbeat comments and new events.
2. Deployment transition emission: when deployment state changes occur (via sync cycle), the stream emits `deployment.deployed.beta1` / `deployment.removed.beta1` envelopes with monotonic IDs.
3. Replay with cursor: reconnecting with `Last-Event-ID` or `after` returns only newer events.
4. Cursor error handling: malformed cursor returns `400`; stale cursor older than retained history returns `410`.
5. Envelope stability: each event includes required fields (`object`, `id`, `type`, `occurred_at`, `data`). `object` is always `"event"`. Event types include version suffix (e.g. `deployment.deployed.beta1`).
6. Type filtering: `?types=deployment.deployed.beta1` returns only matching events in both replay and live stream.
7. Persistence across restart: after restarting agent, new event IDs continue from the last retained ID and replay still works for retained events.
8. Existing tests pass: `./scripts/test.sh` passes with no regressions.

## Idempotence and Recovery

- Safe to rerun: reapplying code edits and rerunning generation steps is idempotent when outputs are overwritten. Reconnecting SSE clients is safe and expected.
- Risky step: event log compaction (retention rewrite) can fail mid-write.
- Recovery strategy: write compaction output to temp file and atomically rename to `events.jsonl`. On startup, if `events.jsonl` has malformed trailing line (partial write), ignore only trailing invalid line and continue from last valid event ID.
- Rollback strategy: if SSE route causes operational issues, disable by removing `/v0.2/events` route wiring while leaving event publication internals intact.

## Key Files Summary

| File | Change |
|------|--------|
| `agent/src/events/mod.rs` | **New** — module root |
| `agent/src/events/model.rs` | **New** — Envelope, Subject, factory methods |
| `agent/src/events/store.rs` | **New** — JSONL persistence, compaction |
| `agent/src/events/hub.rs` | **New** — broadcast + store coordinator |
| `agent/src/events/errors.rs` | **New** — EventsErr enum |
| `agent/src/server/sse.rs` | **New** — SSE HTTP handler |
| `agent/src/lib.rs` | Add `pub mod events;` |
| `agent/src/storage/layout.rs` | Add `events_dir()`, `events_log_file()`, `events_meta_file()` |
| `agent/src/server/mod.rs` | Add `pub mod sse;` |
| `agent/src/server/state.rs` | Add `event_hub` field |
| `agent/src/server/serve.rs` | Add `/events` route |
| `agent/src/app/state.rs` | Init EventHub, add to AppState |
| `agent/src/app/run.rs` | Pass event_hub to server::State |
| `agent/src/sync/syncer.rs` | Add event_hub to SyncerArgs/SingleThreadSyncer, async handle_sync_*, emit sync events |
| `agent/src/sync/deployments.rs` | Add event_hub to SyncArgs, emit deployment events |
| `Cargo.toml` | Add `tokio-stream` workspace dep |
| `agent/Cargo.toml` | Add `tokio-stream` dep |
| `agent/tests/events/*.rs` | **New** — model, store, hub tests |
| `agent/tests/server/sse.rs` | **New** — SSE handler tests |
| `agent/tests/mod.rs` | Add `pub mod events;` |
| `agent/tests/server/mod.rs` | Add `pub mod sse;` |
