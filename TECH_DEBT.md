# Tech Debt — Agent

Items are ordered by ID. Gaps in IDs are expected — never renumber.

| ID | Title | Category | Scope |
|----|-------|----------|-------|
| 1 | Events module has zero test coverage | test-coverage | `agent/src/events/`, `agent/src/server/sse.rs` |
| 2 | EventStore uses blocking synchronous I/O in async runtime | reliability | `agent/src/events/store.rs` |
| 3 | EventHub mutex serializes all event operations behind blocking disk I/O | performance | `agent/src/events/hub.rs` |
| 4 | SSE error_response uses generic error code for all event errors | correctness | `agent/src/server/sse.rs:148` |
| ~~5~~ | ~~sync events not emitted~~ — deliberate scoping decision, see active exec plan Decision Log | — | — |
| 6 | EventStore opens and closes log file on every append | performance | `agent/src/events/store.rs:72-76` |

---

## Details

### TD-1: Events module has zero test coverage

The `events/` module (store, hub, model, errors) and the SSE handler have no unit or integration tests. Existing sync tests pass `None` for the event hub, so event emission is never exercised. Store operations (append, replay, compaction, crash recovery), hub cursor validation, envelope construction, and the SSE handler (cursor resolution, type filtering, replay+live streaming, error responses) are all untested.

### TD-2: EventStore uses blocking synchronous I/O in async runtime

`EventStore::append()` performs `std::fs::OpenOptions::open()`, `writeln!`, and `flush()` — all blocking syscalls — directly inside the tokio async runtime. `compact()` rewrites the entire log synchronously. These run while holding a `tokio::sync::Mutex`, blocking the worker thread and serializing all event operations.

### TD-3: EventHub mutex serializes all operations behind blocking disk I/O

`EventHub::publish()` holds `Mutex<EventStore>` for the entire append+broadcast sequence. `replay_after()` also acquires the same mutex. Because append does blocking I/O, replay calls are blocked behind any in-progress write. Under concurrent SSE connections + event publication, this creates unnecessary contention.

### TD-4: SSE error_response uses generic error code

`error_response()` in `sse.rs` always sets `code: "events_error"` regardless of error variant. Clients cannot distinguish cursor_expired from malformed_cursor via the error code field, which is the standard pattern for programmatic error handling in the agent API.

### ~~TD-5~~ — Closed (deliberate decision)

The active exec plan (`agent/.ai/exec-plans/active/20260310-agent-sse-deployment-events.md`) explicitly decided not to emit sync-level events. See Decision Log entry dated 2026-03-10. Not tech debt.

### TD-6: EventStore opens and closes log file on every append

Each `append()` call opens the file, writes one line, flushes, and drops the handle. For the expected low frequency (events per sync cycle), this is functional but wasteful. A persistent file handle would eliminate repeated open/close syscalls.
