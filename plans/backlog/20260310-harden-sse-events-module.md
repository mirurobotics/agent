# Harden SSE Events Module — Tests, I/O Safety, Error Codes

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Tests for events module, store I/O redesign, SSE error response improvements |

This plan lives in `agent/.ai/exec-plans/` because all code changes are within the agent repository. It follows the first-pass implementation tracked in `agent/.ai/exec-plans/active/20260310-agent-sse-deployment-events.md`.

## Purpose / Big Picture

The SSE events module was implemented as a first pass with no test coverage and several I/O design shortcuts. This plan hardens the implementation so it is production-ready:

1. A developer can run `./scripts/test.sh` and see comprehensive tests covering the event store, hub, model, and SSE handler — including edge cases like crash recovery, compaction, cursor expiration, and type filtering.
2. The event store no longer blocks the tokio runtime during compaction or repeated file opens.
3. SSE error responses return specific error codes (`cursor_expired`, `malformed_cursor`) so programmatic clients can distinguish error types.

This plan resolves agent tech debt items TD-1 through TD-4 and TD-6.

## Progress

- [ ] M1: Add tests for `events/model.rs` and `events/errors.rs`.
- [ ] M2: Add tests for `events/store.rs` (append, replay, compaction, crash recovery).
- [ ] M3: Add tests for `events/hub.rs` (publish, subscribe, cursor validation, try_publish).
- [ ] M4: Add tests for `server/sse.rs` (cursor resolution, error responses, type filtering).
- [ ] M5: Redesign store I/O — persistent file handle, `spawn_blocking` for compaction.
- [ ] M6: Fix SSE error response codes.
- [ ] M7: Integration test — event emission through sync path.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Test the store and hub with real temp-dir-backed files, not mocks.
  Rationale: The store's crash recovery and compaction logic depend on real filesystem behavior (partial writes, atomic renames). Mocking would test the wrong thing. Use `tempfile::TempDir` for isolation.
  Date/Author: 2026-03-10

- Decision: Address I/O improvements (M5) after test coverage (M1-M4) so refactoring is protected by tests.
  Rationale: Tests-first ensures the redesign doesn't introduce regressions. The current I/O works correctly — the issue is performance under edge cases, not correctness.
  Date/Author: 2026-03-10

- Decision: Use `RwLock` instead of `Mutex` in EventHub to allow concurrent replays.
  Rationale: `replay_after()` only needs immutable access to the in-memory event vec. Multiple SSE clients replaying simultaneously shouldn't block each other. `publish()` takes a write lock. `broadcast_tx.send()` is already thread-safe and doesn't need the lock.
  Date/Author: 2026-03-10

## Outcomes & Retrospective

Not started. This section will be filled when milestones are implemented and validated.

## Context and Orientation

This plan operates on code introduced by the SSE implementation plan (`20260310-agent-sse-deployment-events.md`). Read that plan for full architectural context.

Key files being hardened:

- `agent/src/events/mod.rs`: Module root. Exports `EventHub`.
- `agent/src/events/model.rs`: `Envelope` struct with `deployment_deployed()` and `deployment_removed()` constructors. Uses `device_api::models` for event payload types (`DeploymentDeployedBeta1Event`, `DeploymentRemovedBeta1Event`). `status_str()` helper serializes status enums.
- `agent/src/events/store.rs`: `EventStore` with JSONL persistence. `init()` loads from disk tolerating malformed lines. `append()` assigns monotonic ID, opens file, writes, flushes, drops handle. `compact()` rewrites log via temp file + atomic rename when > `max_retained`. `replay_after()` uses `partition_point` for binary search.
- `agent/src/events/hub.rs`: `EventHub` wraps `Mutex<EventStore>` + `broadcast::Sender<Envelope>`. `publish()` locks, appends, broadcasts. `replay_after()` locks, validates cursor, replays. `try_publish()` is fire-and-forget.
- `agent/src/events/errors.rs`: `EventsErr` enum with `IoErr`, `SerializationErr`, `CursorExpiredErr`, `MalformedCursorErr`. Implements `crate::errors::Error` trait.
- `agent/src/server/sse.rs`: SSE handler. Resolves cursor from query/header. Subscribes before replay. Chains replay stream with live broadcast stream. Deduplicates by ID. Filters by type. 30s heartbeat keep-alive.
- `agent/src/sync/deployments.rs`: `apply_deployments()` emits `deployment.deployed.beta1` and `deployment.removed.beta1` events via `hub.try_publish()` for successful outcomes with `Deployed` or `Archived` activity status.

Test infrastructure:
- Tests live in `agent/tests/`. Module structure mirrors `agent/src/`.
- Test runner: `./scripts/test.sh` → `RUST_LOG=off cargo test --features test -- --test-threads=1`.
- `tempfile` crate is already a workspace dependency.

## Plan of Work

### Milestone M1: Model and Error Tests

Create `agent/tests/events/mod.rs` and `agent/tests/events/model.rs`.

Tests for `model.rs`:
- `envelope_deployment_deployed_has_correct_type`: Construct via `Envelope::deployment_deployed()`, assert `event_type == "deployment.deployed.beta1"`, `object == "event"`, `id == 0` (unassigned), `data` contains expected fields.
- `envelope_deployment_removed_has_correct_type`: Same for `deployment_removed()`.
- `envelope_serde_roundtrip`: Serialize an envelope to JSON and deserialize back, assert equality.
- `status_str_serializes_enum_variants`: Test `status_str()` with known enum values, assert correct string output.

Tests for `errors.rs` (can go in `model.rs` or separate `errors.rs`):
- `cursor_expired_returns_410`: Construct `CursorExpiredErr`, assert `http_status() == GONE`.
- `malformed_cursor_returns_400`: Construct `MalformedCursorErr`, assert `http_status() == BAD_REQUEST`.

Add `pub mod events;` to `agent/tests/mod.rs`.

### Milestone M2: Store Tests

Create `agent/tests/events/store.rs`.

Each test creates a `TempDir` and constructs an `EventStore` pointing at a file inside it.

Tests:
- `empty_store_has_no_events`: Init on non-existent file, assert `earliest_id() == None`, `latest_id() == None`, `replay_after(0)` returns empty vec.
- `append_assigns_monotonic_ids`: Append 3 envelopes, assert IDs are 1, 2, 3.
- `append_persists_to_disk`: Append 2 events, drop store, re-init from same file, assert both events are present with correct IDs.
- `replay_after_returns_events_after_cursor`: Append 5 events, `replay_after(2)` returns events 3, 4, 5.
- `replay_after_zero_returns_all`: Append 3 events, `replay_after(0)` returns all 3.
- `replay_after_latest_returns_empty`: Append 3 events, `replay_after(3)` returns empty.
- `compaction_keeps_90_percent`: Init with `max_retained = 10`, append 11 events. Assert len after compaction is 9 (90% of 10). Assert file on disk has 9 lines.
- `compaction_preserves_ids`: After compaction, `earliest_id()` should be the ID of the 3rd event (first 2 drained), `latest_id()` unchanged.
- `crash_recovery_tolerates_malformed_line`: Write a file with 2 valid JSON lines and 1 malformed line. Init store, assert 2 events loaded.
- `crash_recovery_tolerates_trailing_empty_lines`: Write a file with valid events and trailing newlines. Init store, assert correct count.
- `crash_recovery_continues_id_sequence`: Write a file with events having IDs 1-5. Init store, append new event, assert ID is 6.

### Milestone M3: Hub Tests

Create `agent/tests/events/hub.rs`.

Tests:
- `publish_assigns_id_and_returns_envelope`: Create hub, publish envelope, assert returned envelope has id > 0.
- `subscribe_receives_published_events`: Subscribe, publish 2 events, assert receiver gets both in order.
- `replay_after_returns_historical_events`: Publish 3 events, `replay_after(1)` returns events 2, 3.
- `replay_after_zero_with_empty_store_returns_empty`: New hub, `replay_after(0)` returns empty, no error.
- `replay_after_expired_cursor_returns_error`: Publish events, compact so oldest is ID 5, `replay_after(2)` returns `CursorExpiredErr`.
- `try_publish_does_not_panic_on_error`: This is harder to test directly without filesystem manipulation. At minimum, test the happy path doesn't return errors.
- `multiple_subscribers_all_receive`: Subscribe twice, publish, assert both receivers get the event.

### Milestone M4: SSE Handler Tests

Create `agent/tests/server/sse.rs`.

These tests construct an axum Router with the events route and use `axum::body::to_bytes` or `tower::ServiceExt::oneshot` to test HTTP behavior without a real socket.

Tests:
- `events_returns_200_with_event_stream_content_type`: GET `/v0.2/events`, assert 200 and `Content-Type: text/event-stream`.
- `events_returns_400_for_malformed_cursor`: GET `/v0.2/events?after=abc`, assert 400 with error body containing `"malformed_cursor"` code.
- `events_returns_410_for_expired_cursor`: Pre-populate and compact events so earliest is ID 100, GET `/v0.2/events?after=5`, assert 410 with `"cursor_expired"` code.
- `events_returns_503_when_hub_not_initialized`: Construct state with `event_hub: None`, GET `/v0.2/events`, assert 503.
- `events_replays_historical_events`: Publish 3 events, GET `/v0.2/events?after=0`, assert response body contains all 3 events as SSE frames.
- `events_filters_by_type`: Publish events of different types, GET `/v0.2/events?types=deployment.deployed.beta1`, assert only matching events in response.
- `last_event_id_header_works_as_cursor`: Publish 3 events, GET with `Last-Event-ID: 1`, assert events 2, 3 in response.
- `after_takes_precedence_over_last_event_id`: Publish 5 events, GET with `after=3` and `Last-Event-ID: 1`, assert events 4, 5 in response.

Add `pub mod sse;` to `agent/tests/server/mod.rs`.

### Milestone M5: Store I/O Redesign

Refactor `EventStore` to:

1. **Keep a persistent file handle.** Add `writer: Option<BufWriter<File>>` to `EventStore`. Lazily open on first `append()`. Reuse for subsequent appends. Only reopen after compaction (since the file was replaced).

2. **Use `spawn_blocking` for compaction.** Compaction rewrites the entire log, which can be slow for large retention windows. Move the compaction work to a blocking thread. Since compaction is triggered inside `publish()` which holds the mutex, the simplest approach is:
   - After `append()` detects the threshold is exceeded, release the current writer handle.
   - Call `tokio::task::spawn_blocking` with the compaction closure.
   - Re-open the writer handle after compaction completes.
   - Since `EventHub::publish()` is already async and holds a mutex, awaiting the spawn_blocking is safe.

   Alternatively: keep compaction synchronous but on the blocking thread by having `EventHub::publish()` check the threshold and spawn compaction after releasing the lock. This avoids holding the lock during compaction at the cost of slightly delayed compaction. Use this simpler approach.

3. **Switch from `Mutex` to `RwLock` in EventHub.** `replay_after()` only reads from the in-memory vec. Multiple SSE clients replaying concurrently should not block each other. `publish()` takes a write lock.

Changes:
- `agent/src/events/store.rs`: Add `writer: Option<BufWriter<File>>` field. `append()` reuses writer. `compact()` closes writer, rewrites, sets writer to None (will be lazily reopened). Add `fn needs_compaction(&self) -> bool`.
- `agent/src/events/hub.rs`: Change `Mutex<EventStore>` to `RwLock<EventStore>`. `publish()` uses `write().await`. `replay_after()` uses `read().await`. After publish, if `needs_compaction()`, spawn compaction on blocking thread outside the lock.

### Milestone M6: Fix SSE Error Response Codes

Change `error_response()` in `agent/src/server/sse.rs` to map error variants to specific codes:

    fn error_response(e: EventsErr) -> (StatusCode, Json<serde_json::Value>) {
        let status = e.http_status();
        let code = match &e {
            EventsErr::CursorExpiredErr(_) => "cursor_expired",
            EventsErr::MalformedCursorErr(_) => "malformed_cursor",
            _ => "internal_error",
        };
        ...
    }

This aligns with the agent API's pattern of machine-readable error codes in response bodies.

### Milestone M7: Integration Test — Sync Path Event Emission

Create `agent/tests/events/integration.rs` (or extend `tests/sync/deployments.rs`).

Test that the sync path actually emits events:
- Construct a `SyncArgs` with a real `EventHub` (temp-dir-backed).
- Set up mock HTTP client and storage with a deployment that will transition to `Deployed`.
- Run `deployments::sync()`.
- Assert the hub has a `deployment.deployed.beta1` event with the correct deployment ID.

This closes the gap where existing sync tests pass `None` for `event_hub`.

## Concrete Steps

All commands run from the `agent` submodule root (`/home/ben/miru/miru/agent`).

1. Create test scaffolding.

       mkdir -p agent/tests/events
       touch agent/tests/events/{mod.rs,model.rs,store.rs,hub.rs}
       touch agent/tests/server/sse.rs

   Expected: new empty test files.

2. Verify current tests pass before changes.

       ./scripts/test.sh

   Expected: all tests pass.

3. After each milestone, run tests:

       ./scripts/test.sh

   Expected: all tests pass, including new ones.

4. After M5 (I/O redesign), verify event persistence still works:

       # in a test: create hub, publish events, drop hub, re-create from same file, replay
       # This is covered by store tests from M2.

5. After M6, verify error codes in test output:

       # in sse tests: assert error body contains "cursor_expired" or "malformed_cursor"

## Validation and Acceptance

1. `./scripts/test.sh` passes with all new tests (model, store, hub, sse, integration).
2. No regressions in existing tests.
3. Store tests demonstrate: monotonic IDs, disk persistence, compaction, crash recovery.
4. Hub tests demonstrate: publish/subscribe, cursor validation, concurrent access.
5. SSE tests demonstrate: cursor resolution, error codes (400 with `malformed_cursor`, 410 with `cursor_expired`), type filtering, replay.
6. Integration test demonstrates: event emission through the sync path.
7. After M5: `EventStore` no longer opens/closes the file on every append. Compaction does not block the tokio runtime thread.

## Idempotence and Recovery

- All steps are idempotent: rerunning test creation or code edits produces the same result.
- Tests use `TempDir` for isolation — no shared state between tests.
- If M5 (I/O redesign) causes regressions, revert to the current synchronous implementation. The tests from M1-M4 will catch any issues.
- No destructive operations. No database changes. No infrastructure changes.
