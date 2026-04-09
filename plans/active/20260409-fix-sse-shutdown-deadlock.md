# Fix SSE Shutdown Deadlock

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All code changes live here |

This plan lives in `agent/plans/` because all affected files are in the agent repo.

## Purpose / Big Picture

When `sudo systemctl stop` sends SIGTERM to the agent, it currently takes ~15 seconds to shut down instead of shutting down immediately. This happens because open SSE (`/v0.2/events`) connections block the axum HTTP server from completing its graceful shutdown, causing the shutdown to time out and force-exit.

After this change, the agent will shut down within a second of receiving SIGTERM regardless of how many SSE clients are connected. Each open SSE connection will self-terminate as soon as the shutdown signal is broadcast.

## Progress

- [ ] Update `agent/src/server/state.rs` — add `shutdown_tx` field
- [ ] Update `agent/src/app/run.rs` — pass `shutdown_tx` to `State::new`
- [ ] Update `agent/src/server/sse.rs` — terminate stream on shutdown
- [ ] Update `agent/tests/server/sse.rs` — pass `shutdown_tx` to fixture, add shutdown test
- [ ] Run `scripts/test.sh` and confirm all tests pass
- [ ] Commit milestone
- [ ] Run `scripts/lint.sh` and fix any issues
- [ ] Commit lint fixes if needed

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Add `shutdown_tx: broadcast::Sender<()>` to server `State` rather than passing a `CancellationToken`.
  Rationale: The existing shutdown infrastructure already uses `tokio::sync::broadcast::Sender<()>` throughout `app/run.rs`. Reusing the same type keeps the shutdown path consistent and avoids adding a new dependency (`tokio-util`).
  Date/Author: 2026-04-09

- Decision: Use `StreamExt::take_until` to terminate the SSE stream rather than changing shutdown order.
  Rationale: Changing the shutdown order so `AppState`/`EventHub` is torn down before the socket server would drop the broadcast sender, which ends streams, but it would also mean the server is serving requests with a partially-shutdown app state. `take_until` cleanly terminates only the long-lived SSE connections without affecting the shutdown ordering of other components.
  Date/Author: 2026-04-09

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### The shutdown deadlock

`agent/src/app/run.rs` contains the `ShutdownManager` which tears down components in `shutdown_impl()` (line ~428) in this order:

1. Token refresh worker
2. Poller worker
3. MQTT worker
4. **Socket server** ← hangs here indefinitely when SSE clients are connected
5. App state (EventHub shutdown) ← never reached

The socket server is run via `axum::serve(...).with_graceful_shutdown(signal)` in `agent/src/server/serve.rs` (line 141). When the shutdown signal fires, axum stops accepting new connections but **waits for all in-flight connections to drain** before returning. SSE connections are kept open indefinitely by the client.

The SSE handler at `agent/src/server/sse.rs` builds a stream backed by:

    BroadcastStream::new(event_hub.subscribe())

This stream only ends when:
- The client disconnects, OR
- All `broadcast::Sender<Event>` clones are dropped (i.e., the `EventHub` is destroyed)

The `broadcast::Sender<Event>` lives inside `AppState` → `EventHub` (`agent/src/events/hub.rs`), which is shut down in step 5. But step 5 is never reached because step 4 blocks waiting for SSE connections to close. Classic deadlock.

After 15 seconds, `max_shutdown_delay` (defined in `agent/src/app/options.rs`, default `Duration::from_secs(15)`) triggers and calls `std::process::exit(1)`.

### Key files

- `agent/src/server/state.rs` — server `State` struct; holds shared data passed to every handler
- `agent/src/server/sse.rs` — the `events` SSE handler; builds the `Sse<...>` response stream
- `agent/src/app/run.rs` — startup and shutdown orchestration; `init_socket_server` constructs `State`
- `agent/src/app/options.rs` — `LifecycleOptions` including `max_shutdown_delay`
- `agent/tests/server/sse.rs` — integration tests for the SSE handler; contains `Fixture` which constructs `State`

### Existing imports and types

`tokio::sync::broadcast` is already used throughout `app/run.rs`. The workspace dependency `tokio-stream = { version = "0.1", features = ["sync"] }` (in root `Cargo.toml`) provides `StreamExt::take_until`.

`State::new` signature (current):

    pub fn new(
        storage: Arc<Storage>,
        http_client: Arc<http::Client>,
        syncer: Arc<sync::Syncer>,
        token_mngr: Arc<authn::TokenManager>,
        activity_tracker: Arc<activity::Tracker>,
        event_hub: events::EventHub,
    ) -> Self

### Import ordering convention

All source files follow this order, with groups separated by a blank line and a comment:

    // standard crates
    use std::...;

    // internal crates
    use crate::...;

    // external crates
    use tokio::...;

## Plan of Work

### Step 1 — `agent/src/server/state.rs`

Add `shutdown_tx: broadcast::Sender<()>` as a new field to the `State` struct. Add `shutdown_tx` as a final parameter to `State::new` and store it in the struct. Add `use tokio::sync::broadcast;` to the external crates import block.

### Step 2 — `agent/src/app/run.rs` (`init_socket_server`)

In `init_socket_server`, pass `shutdown_tx.clone()` as the final argument to `server::State::new(...)`. No other changes needed — `shutdown_tx` is already in scope.

### Step 3 — `agent/src/server/sse.rs` (`events_impl`)

In `events_impl`, after building `sse_stream`, add:

    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let sse_stream = sse_stream
        .take_until(async move { let _ = shutdown_rx.recv().await; });

Then return `Ok(Sse::new(sse_stream).keep_alive(...))` as before.

`StreamExt::take_until` yields items from the stream until the provided future resolves, then terminates the stream. When `shutdown_tx` is dropped or fires, `shutdown_rx.recv()` returns, the future resolves, and the SSE stream ends cleanly — allowing the axum server to drain the connection and complete its graceful shutdown.

`take_until` is provided by `tokio_stream::StreamExt`, which is **already imported** in `sse.rs` (`use tokio_stream::StreamExt;` — the existing imports include it for `filter_map` and `StreamExt`). No new import is needed for `take_until` itself.

Add `use tokio::sync::broadcast;` to the external crates block — this is new and required for `broadcast::Sender<()>` on `state.shutdown_tx`.

### Step 4 — `agent/tests/server/sse.rs`

**Update `Fixture::with_hub_opts`:** Add a `broadcast::channel::<()>(1)` and pass the sender to `State::new`. Store the sender in `Fixture` as `shutdown_tx: broadcast::Sender<()>` so tests can trigger shutdown.

**Update `Fixture::new`:** No change needed — it delegates to `with_hub_opts`.

**Add new test** in the `stream` module:

    #[tokio::test]
    async fn stream_closes_on_shutdown() {
        // Verify: when shutdown_tx is dropped/fired, an open SSE connection
        // terminates before the test timeout, rather than hanging until timeout.
    }

This test should:
1. Open an SSE connection.
2. Send the shutdown signal (`fixture.shutdown_tx.send(())` or drop it).
3. Assert the body stream ends within a short deadline (e.g., 500 ms) — not by timing out but by the stream completing on its own.

The test distinguishes the new behavior from the old: previously `request_sse` always ran to timeout because the stream never closed; now it should close promptly.

## Concrete Steps

All commands run from `agent/` (i.e., `/home/ben/miru/workbench1/agent/`) unless noted.

### Milestone 1 — Source changes

1. Edit `agent/src/server/state.rs`:
   - Add `use tokio::sync::broadcast;` in the external crates block.
   - Add `pub shutdown_tx: broadcast::Sender<()>,` field to `State`.
   - Add `shutdown_tx: broadcast::Sender<()>` as last parameter to `State::new`.
   - Add `shutdown_tx,` in the `State { ... }` constructor body.

2. Edit `agent/src/app/run.rs` (`init_socket_server`):
   - In the `server::State::new(...)` call, add `shutdown_tx.clone()` as the final argument.

3. Edit `agent/src/server/sse.rs` (`events_impl`):
   - Add `use tokio::sync::broadcast;` in the external crates block if not already present.
   - After `let sse_stream = stream.filter_map(...)` in `events_impl`, insert:

         let mut shutdown_rx = state.shutdown_tx.subscribe();
         let sse_stream = sse_stream
             .take_until(async move { let _ = shutdown_rx.recv().await; });

4. Edit `agent/tests/server/sse.rs`:
   - Add `use tokio::sync::broadcast;` in the external crates block.
   - Add `shutdown_tx: broadcast::Sender<()>` to `Fixture`.
   - In `Fixture::with_hub_opts`, create `let (shutdown_tx, _) = broadcast::channel::<()>(1);` and pass `shutdown_tx.clone()` as last arg to `State::new`. Store `shutdown_tx` in the returned `Self`.
   - Add `stream_closes_on_shutdown` test to the `stream` module. Concrete implementation:

         #[tokio::test]
         async fn stream_closes_on_shutdown() {
             let f = Fixture::new("sse_stream_closes_on_shutdown").await;

             // Open the SSE connection but do NOT read the body yet
             let req = Request::builder()
                 .uri("/v0.2/events")
                 .header("Accept", "text/event-stream")
                 .body(Body::empty())
                 .unwrap();
             let response = f.app.clone().oneshot(req).await.unwrap();
             assert_eq!(response.status(), StatusCode::OK);

             // Signal shutdown
             f.shutdown_tx.send(()).unwrap();

             // The body stream should complete on its own within a tight deadline,
             // not hang until an external timeout fires.
             let mut body = response.into_body();
             let result = tokio::time::timeout(Duration::from_millis(500), async {
                 while let Some(Ok(_frame)) = body.frame().await {}
             })
             .await;

             assert!(
                 result.is_ok(),
                 "SSE stream should close promptly after shutdown signal, not hang"
             );
         }

5. Run tests:

       From: /home/ben/miru/workbench1/agent/
       Command: scripts/test.sh
       Expected: all tests pass, including the new `stream_closes_on_shutdown` test.

6. Commit:

       From: /home/ben/miru/workbench1/agent/
       Command: git add agent/src/server/state.rs agent/src/server/sse.rs agent/src/app/run.rs agent/tests/server/sse.rs
               git commit -m "fix(server): terminate SSE streams on shutdown to prevent graceful shutdown deadlock"

### Milestone 2 — Lint

7. Run lint:

       From: /home/ben/miru/workbench1/agent/
       Command: scripts/lint.sh
       Expected: clean pass with no errors.

8. If lint produces changes or errors, fix them and commit:

       git add -p   # stage only lint-related changes
       git commit -m "chore(lint): apply lint fixes"

## Validation and Acceptance

**Preflight must report `clean` before a PR is opened.** Do not push or open a PR if preflight reports `capped` or has remaining failures.

### Test-level acceptance

Run from `/home/ben/miru/workbench1/agent/`:

    scripts/test.sh

Expected: all tests pass. The new test `server::sse::stream::stream_closes_on_shutdown` must:
- **Fail before the fix** (stream does not close until timeout)
- **Pass after the fix** (stream closes promptly when shutdown signal fires)

### Behavioral acceptance

The SSE stream should end within milliseconds of the shutdown signal being sent — not after a 15-second timeout. The existing `request_sse` helper uses a timeout-based approach because "SSE streams never end"; the new `stream_closes_on_shutdown` test proves they now do end on shutdown.

## Idempotence and Recovery

All edits are additive or small targeted changes. If a step fails partway:

- **Compiler errors:** run `cargo check --package miru-agent` from `agent/` for fast feedback before running the full test suite.
- **Test failures:** revert changes to the failing file with `git checkout -- <file>` and retry.
- **Lint failures:** `scripts/lint.sh` may auto-fix some issues (fmt, clippy `--fix`); review `git diff` before committing.
- All changes are in a single feature branch (`fix/sse-shutdown-deadlock`) and do not touch any migration or shared infrastructure, so rollback is `git checkout main`.
