# Calibrations: upload worker and intake service

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (`mirurobotics/agent`) | read-write | All changes are in crate `miru-agent` under `agent/src/` and `agent/tests/`, plus `ARCHITECTURE.md`. |
| `api/specs/backend/v05.yaml`, `libs/backend-api/` (in this repo) | read-only | Source of the `POST /calibrations` request shape and validation rules. Generated; never edited. |

The plan lives in `agent/plans/` because every change is in this repository. All paths below are relative to the agent repo root (`/home/claude/agent`). This is the reference plan for the `mikado` workflow on branch `feat/calibrations-reference`: it is implemented whole as a draft reference PR, then carved into the slice PRs listed under Plan of Work. Slice 2 carries this plan file; the last slice moves it to `plans/completed/`.

Out of scope: the Device API route `POST /calibrations` (handler, `libs/device-api` regeneration, body limits, rejection mapping) and Windows end-to-end testing and release. Those are later PRs.

## Purpose / Big Picture

On-device applications will report calibrations to the local agent, which must accept them even while Miru is unreachable and deliver them later. Slice 1 (open PR #324) already stores accepted reports durably on disk. This plan adds the two pieces that make that store useful:

1. An upload worker that sends buffered reports to Miru's `POST /calibrations` one at a time, oldest first. It deletes each report once Miru answers 200, waits with capped backoff while Miru is unreachable or failing, resumes immediately when the agent reconnects, and moves any report Miru permanently rejects into `<data>/calibrations/failed/` with an error log line instead of retrying it forever.
2. An intake service that validates a report against Miru's rules (so permanent rejections are rare), stamps the receive time and current deployment, buffers it, wakes the worker and returns `{ reported_at }`.

After the final slice, a running agent has `<data>/calibrations/` (default `/var/lib/miru/calibrations/` on Linux) with `queue.json` and `content/`, and the worker drains anything found there at startup. The service has no caller until the Device API route lands; its behavior is proven by tests.

## Progress

- [x] Slice 1 `calibrations-buffer` (PR #324, already open): generic `buffer::Buffer` and `calibrations::Buffer`.
- [x] Slice 2 `buffer-park` (2026-10-20): `Buffer::park`, plus a `move_failure_keeps_the_entry` test for the rename error path.
- [x] Slice 3 `calibrations-shared-buffer-and-sender` (2026-10-20): token errors are wrapped in a never-terminal `TokenErr` (see Decision Log); fixtures in `agent/tests/test_utils/calibrations.rs`.
- [x] Slice 4 `calibrations-upload-worker` (2026-10-20): `workers/calibrations.rs`, `mocks/calibration_sender.rs`, `SleepController::attempted_sleeps`; 17 worker tests.
- [x] Slice 5 `calibration-intake-service` (2026-10-20): `services/calibration/` at 100% region coverage, so its `.covgate` is `100.00`.
- [x] Slice 6 `calibrations-wiring` (2026-10-20): the worker joins are moved into `ShutdownManager::join_upload_workers` to keep `shutdown_impl` under 50 lines; extra test `app::run::runs_without_calibrations_when_the_buffer_cannot_open`.

## Surprises & Discoveries

- `serde_json` stops at 128 levels of nesting, while Go's `encoding/json` allows 10,000, so intake rejects json data nested deeper than 128 levels that Miru would accept. Accepted as an edge case; lifting it needs serde_json's `unbounded_depth` feature.
- `AuthnErr` forwards `is_terminal()` to its inner `HTTPErr`, so a token request Miru answers with, say, 403 is "terminal". Passed through unchanged, that would make the worker park every buffered report while token issuance fails. Evidence: `crate::impl_error!(AuthnErr { .., HTTPErr, .. })` in `agent/src/authn/errors.rs`.

## Decision Log

- Decision: Deduplication (`report_id` or idempotency key) is out of scope; a response lost mid-upload, or a shutdown between Miru's 200 and the local delete, can store a calibration twice. Date/Author: 2026-10-19, user.
- Decision: Keep the field name `CalibrationJob::calibrated_at`; it means "when the agent received the report". The sender maps it to the backend request's `reported_at`, and the service returns it as `reported_at`. Date/Author: 2026-10-20, user brief.
- Decision: A report Miru rejects with a terminal 4xx is parked: its item file moves to `<data>/calibrations/failed/<entry id>.json`, its queue entry is dropped, and an error is logged. Parking is a generic `buffer::Buffer::park`, not calibration code. Date/Author: 2026-10-20, user brief.
- Decision: Retry with the data-upload backoff values (10 s base, ×2 growth, 1 h cap). Network errors wait a flat 10 s and do not grow the backoff; 5xx, 401, 408, 429 and other non-terminal errors grow it. Reports are never dropped for age or attempt count; the buffer cap (intake answers 507 when full) is the backstop. Rationale: the agent promised the caller durability. Date/Author: 2026-10-20, plan author.
- Decision: On a retryable failure the report stays at the head of the buffer and the worker waits out the backoff, instead of calling `Buffer::requeue` (which moves the entry to the tail and stamps a per-entry deadline). Rationale: the brief also requires FIFO draining, and per-entry requeue would let every other ready report fire its own failing request while Miru is down (a burst of up to 1,000 requests every 10 s). Holding the head gives strict FIFO and one request per backoff interval. The backoff streak is in memory only, so a restart retries at once. Date/Author: 2026-10-20, plan author.
- Decision: Intake and the worker share the buffer through `calibrations::SharedBuffer` (a `tokio::sync::Mutex<Buffer>` plus a `tokio::sync::Notify` wake-up), not a new actor. The worker never holds the lock across the HTTP call, so intake never waits on the network. Rationale: an actor would add a command enum, handle and send/receive errors (~150 lines) for no extra safety, since the buffer has exactly two users. Date/Author: 2026-10-20, plan author.
- Decision: "Reconnect" is `sync::syncer::SyncEvent::SyncSuccess` from `SyncerExt::subscribe`. The poller and backend sync requests over MQTT (`workers/mqtt.rs`) both drive syncs, and a successful sync proves Miru is reachable. A `SyncSuccess` ends any backoff wait. Date/Author: 2026-10-20, plan author.
- Decision: Intake mirrors Miru's own checks (backend `internal/configs/services/calibrations/clean.go`, `create.go`, `internal/configs/ugc/calibrations.go`; core `pkg/ugc/generic.go`, `sanitize.go`). `component_slug`, `role` and `aspect` all use core's `SanitizeSlug` regex `^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$`: 1–32 lowercase alphanumerics and hyphens, starting and ending alphanumeric (consecutive hyphens allowed), matched as given with no trimming. `serial_number` has no NUL and at most 255 Unicode scalar values (Go counts runes). `is_slug` is hand-written; no regex crate. Date/Author: 2026-10-20, from Miru's source.
- Decision: Content is checked by deserializing it into `backend_api::models::InstanceContent`; `format` must be `json`, `yaml`, `jsonc`, `xml` or `text` (`InstanceFormatUnknown` is rejected), and `data` must be 1 to 1,048,576 bytes (Miru measures `len(data)`; empty is rejected). Miru also rejects json, jsonc and yaml data that doesn't parse. Intake checks json with `serde_json`; the agent has no yaml or jsonc parser and no new crates are added, so those two rely on the worker parking Miru's terminal rejection. The caller's JSON value is buffered unchanged. Date/Author: 2026-10-20, from Miru's source.
- Decision: An error reading the deployments cache fails intake (500) rather than stamping `deployment_id: None`, so a broken cache never silently drops the association. No deployed deployment is fine and stamps `None`. Date/Author: 2026-10-20, plan author.
- Decision: If the buffer fails to open at boot, `AppState::calibrations` is `None`, the error is logged, the worker isn't spawned and the agent still boots (fail-open, like the uploader and deleter), so a bad calibrations directory can't block config deployment. The future route will answer with an error while it is `None`. Date/Author: 2026-10-20, plan author.
- Decision: The sender's token failures are `CalibrationsErr::TokenErr(TokenErr(AuthnErr))` rather than a bare `AuthnErr` variant. `TokenErr` keeps `is_network_conn_err()` but is never terminal, so only a rejection of the report itself (an HTTP 4xx on `POST /calibrations`, or an unsendable report) parks it. Date/Author: 2026-10-20, implementer.
- Decision: The worker drops (removes) a queue entry whose report file is missing, since there is nothing to send or park and leaving it at the head would block every later report until restart. A report file that fails to parse is parked (terminal `InvalidReportErr`), and any other read error is a counted retry. The private `Outcome` is `Resolved | Retry { network }`: sent, parked and dropped all reset the streak, and only the logs tell them apart. Date/Author: 2026-10-20, implementer.
- Decision: `ServiceErr::CalibrationsErr` holds a `Box<CalibrationsErr>`, because the unboxed variant grows `ServiceErr` to 128 bytes and trips clippy's `result_large_err` across `server/`. The size and syntax failures of `content.data` name the field `content.data`; a `content` that isn't a `{ format, data }` object names `content`. Date/Author: 2026-10-20, implementer.
- Decision: Each send is one HTTP request (no `http::with_retry`); the worker's backoff is the only retry loop. If parking fails, the report is kept and handled like a counted retry. Date/Author: 2026-10-20, plan author.

## Outcomes & Retrospective

All six slices are implemented on `feat/calibrations-reference`, one commit per milestone. A booted agent opens `<data>/calibrations/` and runs the upload worker, which drains reports in intake order, backs off 10 s → 1 h on failures (flat 10 s while unreachable), resumes on a successful sync, parks Miru's terminal rejections (and unparseable report files) in `failed/`, and loses nothing on shutdown. `services::calibration::create` validates against Miru's actual rules and buffers offline; it has no caller until the Device API route lands. Region coverage (with the root-only permission tests rerun as `nobody`): `calibrations` 100%, `services/calibration` 100%, `buffer` 94.0%, `workers` 92.0%, `services` 97.7%, `app` 94.8%, all at or above their gates.

Deviations from the original plan: token failures are a never-terminal `TokenErr`; a report whose file is missing is dropped rather than retried; the slug rule and content checks follow Miru's source (see Decision Log); `ServiceErr::CalibrationsErr` is boxed; `services/calibration/.covgate` is `100.00`. Open items: deduplication (out of scope), yaml/jsonc syntax is only checked by Miru, and the plan file stays in `plans/active/` until the slices are carved.

## Context and Orientation

The agent (`agent/src/main.rs`, library modules listed in `agent/src/lib.rs`) syncs device config from Miru and serves a local Device API. Conventions that bind every slice (from `AGENTS.md`):

- Import groups in this order, each with its comment: `// standard crates`, `// internal crates`, `// external crates`.
- Errors derive `thiserror::Error` and implement `crate::errors::Error` (`agent/src/errors/mod.rs`: `code()`, `http_status()`, `params()`, `is_network_conn_err()`, `is_terminal()`). Aggregating enums use `crate::impl_error!`, which forwards all five methods to the variant.
- Production functions and closures have at most 50 non-blank, non-comment body lines.
- Public-behavior tests go in `agent/tests/<module>/`, mirroring `agent/src/`, and are declared in the parent `mod.rs`. Tests that need private access go in an inline `#[cfg(test)] mod tests`. Shared mocks live in `agent/tests/mocks/`.
- Tests must be portable (Linux and the `windows-check` CI job) unless they assert Unix-only semantics. Use `crate::test_utils::filesys::dirs::temp(..)` for temp dirs. Add `#[serial]` only for fixed OS paths; nothing here uses one.
- Every module directory has a `.covgate` holding the minimum *region* coverage percentage, enforced by `scripts/covgate.sh` in CI. **`agent/src/calibrations/.covgate` is `100.00`**, so every branch added there (each `?`, each match arm, the `Debug` impl) needs a test. Others: `buffer` 91.91, `workers` 84.67, `services` 95.01, `app` 90.38.

Existing pieces this plan builds on:

- `agent/src/queue/mod.rs`: `Queue<J: QueueJob>`, a persisted FIFO of `QueueEntry<J> { id: Uuid, job, attempts, next_attempt_at }`. `next_ready(now)` returns the oldest entry that is due, without removing it.
- `agent/src/buffer/dir.rs`: `Buffer<J: BufferJob, C>` keeps one queue entry per item in `queue_file`, with the item's content in an *item file* `content_dir/<entry id>.json` holding `buffer::Item { job, content }`. The API is `open(queue_file, content_dir, BufferOptions)`, `enqueue(Item) -> Uuid`, `next_ready`, `read(id)`, `remove(id)`, `requeue(entry)`, `len`, `used_bytes`, `jobs`. When full, `enqueue` returns `BufferErr::BufferFullErr` and never evicts. `open` deletes item files that have no queue entry and drops entries whose file is missing. The private helper `item_file(dir, id)` builds `<id>.json` paths. `Buffer` is single-owner (`&mut self`) and has no `Debug` impl.
- `agent/src/calibrations/`: `CalibrationJob { component_slug, role?, aspect?, serial_number?, deployment_id?, calibrated_at }` (`job.rs`); `Buffer = buffer::Buffer<CalibrationJob, serde_json::Value>`, `Report = buffer::Item<CalibrationJob, serde_json::Value>`, `DEFAULT_OPTIONS` (1,000 reports / 256 MiB) and `open(&CalibrationsLayout, options)` (`buffer.rs`); `CalibrationsErr { BufferFullErr(CalibrationBufferFullErr) /* 507 calibration_buffer_full */, BufferErr }` (`errors.rs`).
- `agent/src/disk/layout.rs`: `Layout::calibrations() -> CalibrationsLayout` with `root` (`<data>/calibrations`), `queue()` (`queue.json`) and `content()` (`content/`).
- `agent/src/http/calibrations.rs`: `create(client: &impl ClientI, CreateParams { payload: &CreateCalibrationRequest, token: &str }) -> Result<Calibration, HTTPErr>`, with a 60 s timeout. `libs/backend-api/src/models/create_calibration_request.rs`: `CreateCalibrationRequest { deployment_id?, component_slug, role?, serial_number?, aspect?, content: Box<InstanceContent>, reported_at: String }`. `InstanceContent { format: InstanceFormat, data: String }`. `InstanceFormat` is `json | yaml | jsonc | xml | text`, plus the catch-all `InstanceFormatUnknown` for any other string.
- Error classification, in `agent/src/http/errors.rs`: `RequestFailed::is_terminal()` is true for 4xx except 401, 408 and 429. Timeouts and connection failures report `is_network_conn_err() == true`. A *terminal* error means "Miru will never accept this request"; a *network* error means "Miru could not be reached".
- Patterns to copy: `agent/src/data_uploads/upload/executor.rs` (the `UploadExecutor` trait seam and `LiveExecutor<C: ClientI, T: TokenManagerExt>`); `agent/src/workers/poller.rs` (worker `run(…, sleep_fn, shutdown_signal)` that `tokio::select!`s a shutdown future against `run_impl`, and the syncer subscription via `crate::workers::next_sync_event`); `agent/src/cooldown/mod.rs` (`Backoff { base_secs, growth_factor, max_secs }`, `calc(&backoff, exp) -> i64` capped at `max_secs`).
- Shutdown: `agent/src/app/run.rs` turns the process's shutdown latch (`shutdown::Latch`: SIGTERM, ctrl-c or Windows SCM stop) into a `broadcast` channel. Each worker gets `shutdown_tx.subscribe()` as its shutdown future and registers its `JoinHandle` with `ShutdownManager`, which joins workers before `AppState::shutdown()` stops the token manager and storage.
- Deployment lookup: `disk::deployments::find_deployed(&disk::Deployments) -> Result<Option<models::Deployment>, DiskErr>`.
- Test mocks: `agent/tests/mocks/http_client.rs` `MockClient` already routes `POST /calibrations` to `create_calibration_fn` and records `Call::CreateCalibration` requests with their body and token. `mocks/stub_token_manager.rs` `StubTokenManager::{ok, err}`. `mocks/token_manager.rs` `MockTokenManager`. `mocks/syncer.rs` `MockSyncer` (`get_transmitter()` sends `SyncEvent`s; `set_subscribe_fail`). `mocks/error.rs` `SleepController` (records each requested sleep duration; `await_sleep()` / `release()`). `mocks/upload_executor.rs` is the model for a scripted mock.

## Interfaces and Dependencies

No new crates. `tokio::sync::{Mutex, Notify}` are already available (the `sync` feature is enabled through `tokio-stream`). New public items:

    // agent/src/buffer/dir.rs (slice 2)
    impl<J: BufferJob, C> Buffer<J, C> {
        pub async fn park(&mut self, id: Uuid, dest_dir: &Dir) -> Result<Option<QueueEntry<J>>, BufferErr>;
    }

    // agent/src/disk/layout.rs (slice 3)
    impl CalibrationsLayout { pub fn failed(&self) -> filesys::Dir; } // <root>/failed

    // agent/src/calibrations/shared.rs (slice 3)
    #[derive(Clone)] pub struct SharedBuffer { buffer: Arc<Mutex<Buffer>>, failed: Dir, wake: Arc<Notify> }
    impl SharedBuffer {
        pub async fn open(layout: &CalibrationsLayout, options: BufferOptions) -> Result<Self, CalibrationsErr>;
        pub fn new(buffer: Buffer, failed: Dir) -> Self;
        pub async fn enqueue(&self, report: Report) -> Result<Uuid, CalibrationsErr>; // wakes on success
        pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, Buffer>;
        pub fn failed_dir(&self) -> &Dir;
        pub async fn woken(&self); // resolves after an enqueue since the last time it resolved
    }
    impl std::fmt::Debug for SharedBuffer { /* "SharedBuffer { failed: .. }" */ }

    // agent/src/calibrations/sender.rs (slice 3)
    pub trait Sender: Send + Sync {
        fn send(&self, report: &Report) -> impl Future<Output = Result<(), CalibrationsErr>> + Send;
    }
    pub struct HttpSender<C: ClientI, T: TokenManagerExt> { http_client: Arc<C>, token_mngr: Arc<T> }
    impl<C: ClientI, T: TokenManagerExt> HttpSender<C, T> { pub fn new(http_client: Arc<C>, token_mngr: Arc<T>) -> Self; }
    pub fn new_request(report: &Report) -> Result<CreateCalibrationRequest, CalibrationsErr>;

    // agent/src/calibrations/errors.rs (slice 3): new variants of CalibrationsErr
    InvalidReportErr(InvalidReportErr), // { reason: String, trace }; is_terminal() == true
    HTTPErr(http::HTTPErr),
    TokenErr(TokenErr),                 // TokenErr(pub authn::AuthnErr); never terminal

    // agent/src/workers/calibrations.rs (slice 4)
    #[derive(Debug, Clone)] pub struct Options { pub backoff: cooldown::Backoff } // default 10 / 2 / 3600
    pub async fn run<S: Sender, Y: SyncerExt, F, Fut>(
        options: &Options, buffer: &SharedBuffer, sender: &S, syncer: &Y,
        sleep_fn: F, shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
    ) where F: Fn(Duration) -> Fut, Fut: Future<Output = ()> + Send;

    // agent/src/errors/mod.rs (slice 5)
    Code::InvalidRequest // as_str() == "invalid_request"

    // agent/src/services/errors.rs (slice 5)
    pub struct InvalidRequestErr { pub field: &'static str, pub reason: String, pub trace: Box<Trace> }
    // code InvalidRequest, http_status 422, params {"field": field}
    ServiceErr::{InvalidRequestErr(InvalidRequestErr), CalibrationsErr(calibrations::CalibrationsErr)}

    // agent/src/services/calibration/ (slice 5)
    pub struct CreateParams { pub component_slug: String, pub role: Option<String>, pub aspect: Option<String>,
                              pub serial_number: Option<String>, pub content: serde_json::Value }
    pub struct Created { pub reported_at: DateTime<Utc> }
    pub async fn create(buffer: &SharedBuffer, deployments: &disk::Deployments,
                        params: CreateParams, now: DateTime<Utc>) -> Result<Created, ServiceErr>;
    pub mod validate { pub fn params(p: &CreateParams) -> Result<(), InvalidRequestErr>; pub fn is_slug(s: &str) -> bool; }

    // agent/src/app/state.rs (slice 6)
    AppState { pub calibrations: Option<calibrations::SharedBuffer>, .. }

## Plan of Work

Each milestone is one slice: it merges on its own with CI green, depends only on earlier slices, and ends with one commit. Estimated changed lines include tests.

| # | Slug | Scope | ~Lines | Depends on |
|---|------|-------|--------|-----------|
| 1 | `calibrations-buffer` | Generic `buffer::Buffer` and `calibrations::Buffer` (exists: PR #324) | 1,870 | none |
| 2 | `buffer-park` | Generic `Buffer::park` moves an item aside and drops its entry | 140 | 1 |
| 3 | `calibrations-shared-buffer-and-sender` | `failed/` layout, `SharedBuffer`, `Sender`/`HttpSender`, error variants | 400 | 1 |
| 4 | `calibrations-upload-worker` | `workers/calibrations.rs` drain loop, backoff, park, reconnect, shutdown | 450 | 2, 3 |
| 5 | `calibration-intake-service` | `invalid_request` code, validation, `services::calibration::create` | 380 | 3 |
| 6 | `calibrations-wiring` | Open the buffer in `AppState`, spawn and join the worker, docs | 150 | 4, 5 |

### Milestone 2: `buffer-park`

In `agent/src/buffer/dir.rs`, add `Buffer::park` in the Mutators section after `remove`. If `id` isn't tracked in `self.sizes` (which mirrors the queue after `open` and `enqueue`), return `Ok(None)` and touch nothing. Otherwise: `dirs::create_if_absent(dest_dir)`, then `files::move_to(&self.item_file(id), &item_file(dest_dir, id), Overwrite::Allow)`, then `self.queue.remove(id)`, then `self.untrack(id)`, and return the entry. The file moves first, so a crash between the two steps leaves an entry whose file is gone; the next `open` drops that entry, and the parked copy survives. Removing the entry first would leave an orphan that `open` deletes. Any error before the entry is removed is returned, with the entry and its item file unchanged. The doc comment states this ordering and that `open` never looks inside `dest_dir`. Add one sentence to the `buffer` paragraph of `ARCHITECTURE.md`.

Tests, in a new `pub mod park` in `agent/tests/buffer/mod.rs`, using the file's existing `TestJob` and helpers:

- `moves_the_item_file_and_drops_the_entry`: enqueue `a` and `b`, then park `a` into `<tmp>/failed`. It returns `Some(entry a)`, `jobs() == [b]`, and `used_bytes()` equals `b`'s file size. `failed/<a>.json` parses as `a`'s `Item`, and `content/<a>.json` is gone.
- `parked_items_stay_out_after_reopen`: park `a`, reopen: `jobs() == [b]`, and the parked file still exists.
- `unknown_id_is_none`: returns `Ok(None)`; `len` unchanged; `dest_dir` not created.
- `failure_keeps_the_entry`: a regular file occupies the `dest_dir` path, so park returns `Err`. `len`, `used_bytes` and `content/<a>.json` are unchanged.

### Milestone 3: `calibrations-shared-buffer-and-sender`

`agent/src/disk/layout.rs`: add `CalibrationsLayout::failed()` returning `self.root.subdir("failed")`, and extend `calibrations_layout` in `agent/tests/disk/layout.rs` with a `failed_dir` assertion (`under_root(&["calibrations", "failed"])`).

`agent/src/calibrations/shared.rs` (new): `SharedBuffer` as in Interfaces. `open` calls `calibrations::open(layout, options)` and `new(buffer, layout.failed())`. `enqueue` locks, calls `Buffer::enqueue`, converts the error with `CalibrationsErr::from` (so a full buffer stays the 507), and calls `wake.notify_one()` only on success. `Notify` keeps one permit when nobody is waiting, so a wake sent while the worker is busy is not lost. `woken` is `self.wake.notified().await`. The hand-written `Debug` prints `failed` and uses `finish_non_exhaustive()`, because `Buffer` has no `Debug` and `AppState` derives it.

`agent/src/calibrations/sender.rs` (new): the `Sender` trait, with a doc comment on cancel safety. The worker drops an in-flight `send` on shutdown, and the report stays buffered and is re-sent next boot (it may be stored twice). `new_request` maps the job's fields one-to-one, sets `reported_at` to `calibrated_at.to_rfc3339()`, and deserializes `report.content` with `InstanceContent::deserialize(&report.content)`. A failure becomes `InvalidReportErr`. `HttpSender::send` runs `new_request(report)?`, then `self.token_mngr.get_token().await?`, then a single `http::calibrations::create(..).await?`, and returns `Ok(())`.

`agent/src/calibrations/errors.rs`: add `InvalidReportErr { reason: String, trace: Box<Trace> }` with `is_terminal() -> true`. Add the variants `InvalidReportErr`, `HTTPErr` and `AuthnErr` to `CalibrationsErr`, with `From` impls and entries in `impl_error!`. `agent/src/calibrations/mod.rs`: `pub mod sender; pub mod shared;` and re-export `SharedBuffer`, `Sender`, `HttpSender` and `InvalidReportErr`. `agent/tests/mocks/http_client.rs`: add `set_create_calibration(f)`, mirroring `set_confirm_upload`. `ARCHITECTURE.md`: extend the `calibrations/` paragraph.

Tests (`calibrations/.covgate` is 100%, so cover every branch), in `agent/tests/calibrations/shared.rs` and `sender.rs`, both declared in `agent/tests/calibrations/mod.rs`:

- shared: `open_uses_the_calibrations_layout` (`content/` exists; `failed_dir() == layout.failed()`). `enqueue_buffers_and_wakes` (`woken()` resolves within a 5 s timeout; `lock().await.len() == 1`). `a_wake_before_waiting_is_kept` (enqueue first, then `woken()` resolves). `a_full_buffer_is_507_and_does_not_wake` (options `max_items: 1`; second enqueue gives `CalibrationsErr::BufferFullErr`; after draining the first permit, a second `woken()` does not resolve within 50 ms). `debug_names_the_type`.
- sender: `new_request_maps_the_job` (all optional fields set; `reported_at` equals `calibrated_at.to_rfc3339()`; content equals the parsed `InstanceContent`). `new_request_omits_absent_fields` (serialized JSON has no `deployment_id`, `role`, `aspect` or `serial_number` keys). `new_request_rejects_unparseable_content` (content `{"offset": 1}` gives `InvalidReportErr`, `is_terminal()`). `send_posts_one_request_with_the_token` (`MockClient` and `StubTokenManager::ok("tok")`: exactly one `Call::CreateCalibration` request, token `tok`, body containing `"reported_at"`). `send_passes_a_422_through_as_terminal`. `send_reports_a_network_error` (`MockErr { is_network_conn_err: true }`). `send_returns_a_token_error` (`StubTokenManager::err`, no request made). `send_rejects_unparseable_content_before_any_request`.

### Milestone 4: `calibrations-upload-worker`

`agent/src/workers/calibrations.rs` (new; add `pub mod calibrations;` to `agent/src/workers/mod.rs`). Structure, keeping each function under 50 lines:

- `Options` with `Default` `backoff: cooldown::Backoff { base_secs: 10, growth_factor: 2, max_secs: 3600 }`. The comment says these mirror `UploaderOptions::default().backoff`.
- `enum Outcome { Sent, Parked, Retry { network: bool } }` and a private `struct Streak(u32)` with `fn next_wait(&mut self, outcome: &Outcome, backoff: &cooldown::Backoff) -> Option<Duration>`. `Sent` and `Parked` reset the streak to 0 and return `None`. `Retry { network: true }` returns `base_secs` and leaves the streak unchanged. `Retry { network: false }` increments the streak and returns `cooldown::calc(backoff, streak - 1)`, which never exceeds `max_secs`.
- `pub async fn run(..)`: `tokio::select!` the pinned shutdown future against `run_impl(..)`, logging `"calibrations: upload worker shutdown complete"`, exactly like `workers/poller.rs`. A shutdown drops any in-flight send; the report is never removed before Miru's 200.
- `run_impl`: subscribe to the syncer (`Some(rx)`, or on error log and use `None`, as in the poller), then loop. `match attempt_next(buffer, sender).await`: on `Some(outcome)`, if `streak.next_wait(..)` is `Some(wait)`, call `back_off(wait, &sleep_fn, &mut events)`. On `None` (nothing buffered), call `idle(buffer, &mut events)`.
- `attempt_next`: under the lock, take `next_ready(Utc::now())?` and `read_report(&guard, id)`, then release the lock. Map a `FileSysErr::ParseJSONErr` from `Buffer::read` to `CalibrationsErr::InvalidReportErr`, because a corrupt file can never be sent. Without the lock, `sender.send(&report)`. Then `resolve(buffer, id, result)`.
- `resolve`: re-lock. On `Ok`, `remove(id)` and log `info` → `Sent`. On `Err(e)` with `e.is_terminal()`, call `park(id, buffer.failed_dir())`. If that succeeds, log `error!("calibrations: Miru rejected report {id}; parked as failed/{id}.json: {e}")` → `Parked`. If parking fails, log the error → `Retry { network: false }`. Otherwise log `warn` → `Retry { network: e.is_network_conn_err() }`. The entry stays at the head.
- `back_off`: pin `sleep_fn(wait)` once, then loop `tokio::select!` over the sleep (return) and `next_sync_event(events)`. `Some(SyncEvent::SyncSuccess)` returns early (reconnect). Other events continue. `None` logs, sets `*events = None` and continues. The sleep is never restarted, so intake activity can't extend a backoff.
- `idle`: `tokio::select!` over `buffer.woken()` and `next_sync_event(events)` (handling `None` as above), then return.

`agent/tests/mocks/calibration_sender.rs` (new; declare it in `agent/tests/mocks/mod.rs`): `MockCalibrationSender`, modeled on `MockUploadExecutor`. It has a scripted `VecDeque<Step>` (`Ok`, `Terminal` = `RequestFailed` 422, `Server` = `RequestFailed` 500, `Network` = `MockErr { is_network_conn_err: true }`, `Hang(oneshot::Receiver<()>)`), and when the script is empty it returns `Ok`. It records each report's `component_slug` and signals an unbounded `started` channel on every call.

Tests in `agent/tests/workers/calibrations.rs` (declared in `agent/tests/workers/mod.rs`). Each test uses a temp-dir `SharedBuffer`, `MockSyncer`, `SleepController::sleep_fn()`, a pending or oneshot shutdown future, and wraps waits in a 60 s hang guard:

- `drains_reports_buffered_before_start_in_order`: enqueue `a`, `b` and `c`, then spawn. The sends are `[a, b, c]`, then the buffer is empty and `content/` has no files.
- `intake_wakes_an_idle_worker`: spawn on an empty buffer, then `enqueue(a)`, and `a` is sent.
- `offline_then_online_uploads_in_order`: script `Network`, then default `Ok`. Enqueue `a` and `b`. After the first send, `await_sleep()` sees 10 s and the buffer still holds `[a, b]`. Send `SyncSuccess` without releasing the sleep. The sends become `[a, a, b]` and the buffer is empty.
- `retryable_errors_back_off_within_bounds`: script nine `Server` steps. Release each sleep, and the requested sleeps are `[10, 20, 40, 80, 160, 320, 640, 1280, 2560]`. Then script two more, and they are `3600` and `3600` (capped). Separately, three `Network` steps sleep `[10, 10, 10]`.
- `a_success_resets_the_backoff`: `Server`, `Ok` (`a`), `Server` (`b`) give sleeps `[10, 10]`.
- `terminal_rejection_is_parked_not_retried`: script `Terminal`, enqueue `a` with id `ia`. `failed/<ia>.json` exists, the buffer is empty and there was one send. Then enqueue `b`: `b` is sent and `a` is never re-sent.
- `an_unreadable_report_is_parked_without_sending`: enqueue `a`, overwrite `content/<id>.json` with `not json`, and spawn. The file is in `failed/` and there were zero sends.
- `shutdown_mid_upload_keeps_the_report_for_replay`: script `Hang`, enqueue `a`, await `started`, fire shutdown, and the join completes. A reopened `SharedBuffer::open` on the same layout still holds `a`. A second `run` with `Ok` sends `a` again (two sends in total). This is the crash-replay case; dedup is out of scope.
- `drains_without_a_syncer_subscription`: `set_subscribe_fail(true)`, enqueue `a`, and `a` is sent. A variant calls `drop_transmitter()` mid-backoff, and the worker still finishes the sleep and retries.

### Milestone 5: `calibration-intake-service`

`agent/src/errors/mod.rs`: add `Code::InvalidRequest` with `as_str() == "invalid_request"`. In `agent/tests/errors/mod.rs`, bump `EXPECTED_CODE_VARIANTS` to 7, add the case, and add the arm to the exhaustive match.

`agent/src/services/errors.rs`: add `InvalidRequestErr` (as in Interfaces; `#[error("invalid {field}: {reason}")]`) and the `ServiceErr` variants `InvalidRequestErr` and `CalibrationsErr`, with `From` impls and `impl_error!` entries.

`agent/src/services/calibration/validate.rs` (new) holds the constants `MAX_CONTENT_BYTES: usize = 1024 * 1024`, `MAX_SERIAL_NUMBER_CHARS: usize = 255` and `MAX_SLUG_CHARS: usize = 32`. It has:

- `pub fn is_slug(s: &str) -> bool`, Miru's slug rule from the Decision Log (1–32 chars), hand-written with no regex crate.
- `pub fn params(p: &CreateParams) -> Result<(), InvalidRequestErr>`, which checks in Miru's order: `component_slug` (slug); `role` and `aspect` when present (slug); `serial_number` when present (no NUL, `chars().count() <= 255`); then `content`. For `content`, `InstanceContent::deserialize(&p.content)` must succeed (field `content`), the format must not be `InstanceFormatUnknown` (field `content.format`), `1 <= data.len() <= MAX_CONTENT_BYTES` (field `content.data`), and json data must parse (field `content.data`).
- A private `fn invalid(field, reason) -> InvalidRequestErr` builder.

`agent/src/services/calibration/create.rs` (new): `CreateParams`, `Created` and `create`. `create` runs `validate::params(&params)?`. It takes the deployment id from `disk::deployments::find_deployed(deployments).await?.map(|d| d.id)`, builds a `Report` whose `CalibrationJob` has `calibrated_at: now` and whose content is the caller's value unchanged, calls `buffer.enqueue(report).await?` (which wakes the worker), and returns `Created { reported_at: now }`. `now` is a parameter, so tests control time; the future route passes `Utc::now()`. `agent/src/services/calibration/mod.rs`: `mod create; pub mod validate; pub use create::*;`. Add `pub mod calibration;` to `agent/src/services/mod.rs`, add `agent/src/services/calibration/.covgate` (measured: `100.00`), and list `calibration` among the `services/` submodules in `ARCHITECTURE.md`.

Tests in `agent/tests/services/calibration/{mod.rs, validate.rs, create.rs}` (declare `pub mod calibration;` in `agent/tests/services/mod.rs`). Assert each error through `code().as_str() == "invalid_request"`, `http_status() == 422` and `params() == Some(json!({"field": ..}))`.

- validate: `accepts_a_minimal_report`. `accepts_every_format` (json, yaml, jsonc, xml, text). `accepts_valid_slugs` (`arm`, `aligner-1`, `front-left`, `home-offsets`, `0`, `a--b`, a 32-character slug). `rejects_bad_slugs_in_each_slug_field`: `""`, `Arm`, `-arm`, `arm-`, ` arm`, `arm 1`, `arm_1`, `ärm` and a 33-character slug, each under `component_slug`, `role` and `aspect`, with the matching field. `content_of_exactly_1_mib_is_accepted`, `content_of_1_mib_plus_1_byte_is_rejected` and `empty_data_is_rejected` (field `content.data`). `serial_number_limit_counts_characters` (255 `é` accepted, 256 rejected) and `serial_number_rejects_nul`. `rejects_malformed_content` (not an object, missing `data`, non-string `data`: field `content`). `rejects_an_unknown_format` (`toml`: field `content.format`). `rejects_unparseable_json` (field `content.data`; the same text as `text` or `yaml` is accepted).
- create: `accepted_while_offline` (no HTTP client exists in the call; returns `reported_at == now`; the buffered job has `calibrated_at == now` and the caller's content). `stamps_the_current_deployment` (a `Deployed` deployment `dpl_1` in a temp `Deployments` store gives `deployment_id == Some("dpl_1")`). `no_deployment_is_fine` (`None`). `wakes_the_worker` (`woken()` resolves). `invalid_report_is_not_buffered` (422; buffer empty). `buffer_full_is_507` (`code().as_str() == "calibration_buffer_full"`, status 507).

### Milestone 6: `calibrations-wiring`

`agent/src/app/state.rs`: add the field `pub calibrations: Option<calibrations::SharedBuffer>`. Add `async fn init_calibrations(layout: &disk::Layout) -> Option<calibrations::SharedBuffer>`, which calls `SharedBuffer::open(&layout.calibrations(), calibrations::DEFAULT_OPTIONS)` and on error logs `error!("failed to open the calibration buffer; calibrations are unavailable until restart: {e}")` and returns `None`. Call it in `AppState::init` after the data-upload actors. `AppState::shutdown` is unchanged: the buffer has no actor, and the worker is joined before the app state.

`agent/src/app/run.rs`: add `init_calibration_upload_worker(app_state, shutdown_manager, shutdown_rx)`, called from `init_data_upload_workers`. It returns `Ok(())` when `app_state.calibrations` is `None`. Otherwise it spawns `workers::calibrations::run(&Options::default(), &buffer, &HttpSender::new(app_state.http_client.clone(), app_state.token_mngr.clone()), app_state.syncer.as_ref(), tokio::time::sleep, shutdown future)` and registers the handle as the new `ShutdownManager` field `calibration_upload_worker_handle` (initialized to `None`). In `ShutdownManager::shutdown`, join it right after the delete driver and before the app state, with the label `"calibration upload"`. Update `ARCHITECTURE.md`: list the worker under `workers/`, and add it to the shutdown-order notes.

Tests in `agent/tests/app/state.rs`, module `init`:

- `calibration_buffer_opened`: `state.calibrations.is_some()` and `<data>/calibrations/content` exists.
- `calibrations_degrade_when_buffer_path_unwritable`: a regular file at `layout.calibrations().root` path gives `state.calibrations.is_none()`, init succeeds and shutdown succeeds.

The existing `agent/tests/app/run.rs` start/shutdown tests (`shutdown_signal_received` and others) now exercise spawning and joining the worker.

## Concrete Steps

All commands run from the agent repo root, `/home/claude/agent`, on branch `feat/calibrations-reference`. Make the edits for a milestone as described in Plan of Work, validate it locally (see Validation), then commit it once.

1. Before milestone 2, confirm the plan is active: `git ls-files plans/active/20261020-calibrations-upload-worker-and-intake.md` prints the path.
2. Milestone 2 commit:

        git add agent/src/buffer/dir.rs agent/tests/buffer/mod.rs ARCHITECTURE.md
        git commit -m "feat(buffer): park a resolved item outside the buffer"

3. Milestone 3 commit:

        git add agent/src/disk/layout.rs agent/src/calibrations agent/tests/calibrations agent/tests/disk/layout.rs agent/tests/mocks/http_client.rs ARCHITECTURE.md
        git commit -m "feat(calibrations): shared buffer and backend sender"

4. Milestone 4 commit:

        git add agent/src/workers agent/tests/workers agent/tests/mocks
        git commit -m "feat(workers): calibration upload worker"

5. Milestone 5 commit:

        git add agent/src/errors agent/src/services agent/tests/errors agent/tests/services ARCHITECTURE.md
        git commit -m "feat(services): calibration intake service"

6. Milestone 6 commit, after setting Progress and Outcomes in this file:

        git add agent/src/app agent/tests/app ARCHITECTURE.md plans/active/20261020-calibrations-upload-worker-and-intake.md
        git commit -m "feat(app): open the calibration buffer and run its upload worker"

Each `git commit` prints one line such as `[feat/calibrations-reference abc1234] feat(workers): calibration upload worker`. Update Progress with a timestamp after each commit.

## Validation and Acceptance

Each milestone is validated locally before its commit: `cargo fmt -p miru-agent`; `cargo clippy --package miru-agent --all-features --all-targets` (clean outside the generated `libs/`); the custom linter `cargo run -q --manifest-path tools/lint/Cargo.toml -- --path agent/src --config .lint-imports.toml --assert-paths agent/tests agent/src` and the same with `--path agent/tests`; `RUST_LOG=off cargo test -p miru-agent --test mod -- <filters>`; and region coverage per touched module (`RUST_LOG=off cargo llvm-cov --no-report --no-fail-fast -p miru-agent`, then `cargo llvm-cov report --json`) against its `.covgate`. The sandbox runs as root, so tests asserting permission denial are also run as user `nobody`. CI (`.github/workflows/ci.yml`) then validates the pushed head of the reference PR and of each slice PR. The jobs are: `lint` (custom import, function-length and assertion linters; `cargo fmt --check`; clippy with `-D warnings`; machete), `test` (`./scripts/covgate.sh`: the whole suite under coverage, then every `.covgate` threshold, including `calibrations` at 100% and the new `services/calibration` at 95%), `windows-check` (the portable suite on Windows) and `tools`. A slice is done when `$preflight` reports `CLEAN` for it.

Acceptance, observable through the named tests:

- A report buffered before the worker starts reaches Miru as `POST /calibrations` with `reported_at` equal to its intake time, and its file disappears from `content/` (`drains_reports_buffered_before_start_in_order`, `send_posts_one_request_with_the_token`).
- While Miru is unreachable, reports stay in `content/` and the worker sends one request per 10 s. A successful sync makes it resume at once and upload in intake order (`offline_then_online_uploads_in_order`).
- Repeated 5xx responses back off 10 s, 20 s, … up to 1 h and never beyond (`retryable_errors_back_off_within_bounds`).
- A 422 from Miru moves the report to `<data>/calibrations/failed/<id>.json` and logs an error; the report is never re-sent and later reports still upload (`terminal_rejection_is_parked_not_retried`).
- Shutdown during an upload loses nothing: after restart the report is sent again (`shutdown_mid_upload_keeps_the_report_for_replay`).
- `services::calibration::create` with Miru offline returns `{ reported_at: now }` and buffers the report with the current deployment id, or `None` when nothing is deployed (`accepted_while_offline`, `stamps_the_current_deployment`, `no_deployment_is_fine`).
- `data` of 1,048,576 bytes is accepted, and 1,048,577 bytes, empty data or unparseable json gives 422 `invalid_request` with `params == {"field": "content.data"}`. Each bad slug (Miru's 1–32 character rule) or a serial number over 255 characters gives 422 naming its field. A full buffer gives 507 `calibration_buffer_full`.
- A booted agent has `<data>/calibrations/content/`. A blocked calibrations path still boots, with calibrations unavailable (`calibration_buffer_opened`, `calibrations_degrade_when_buffer_path_unwritable`).

## Idempotence and Recovery

Every edit is additive except the new `CalibrationsErr`, `ServiceErr` and `Code` variants, which compile-break only exhaustive matches the plan already names (`agent/tests/errors/mod.rs`). Re-running a milestone's edits is safe; if a commit already exists, amend it rather than adding a second one. If a slice fails CI, fix it on that slice's branch; earlier slices are unaffected because no slice depends on a later one. Any milestone commit can be reverted alone, with its dependents reverted first (see the table).

The on-disk format is unchanged: `queue.json` and `content/<id>.json` keep the slice 1 shape, and `failed/` is a new sibling that `Buffer::open` never reads. An older agent without the worker leaves buffered reports in place, and a newer one uploads them. Reverting milestone 6 stops uploads but keeps every buffered report on disk. Parked reports are never deleted automatically; remove `<data>/calibrations/failed/*.json` by hand once inspected.
