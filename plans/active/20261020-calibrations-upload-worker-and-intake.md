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
2. An intake service that stamps the receive time and current deployment, buffers it, wakes the worker and returns `{ reported_at }`.

After the final slice, a running agent has `<data>/calibrations/` (default `/var/lib/miru/calibrations/` on Linux) with `queue.json` and `content/`, and the worker drains anything found there at startup. The service has no caller until the Device API route lands; its behavior is proven by tests.

## Progress

- [x] Slice 1 `calibrations-buffer` (PR #324, already open): generic `queue::DirQueue` and `calibrations::DirQueue`.
- [x] Slice 2 `buffer-park` (2026-10-20): `DirQueue::park`, plus a `move_failure_keeps_the_entry` test for the rename error path.
- [x] Slice 3 `calibrations-sender` (2026-10-20): token errors are wrapped in a never-terminal `TokenErr` (see Decision Log); fixtures in `agent/tests/test_utils/calibrations.rs`.
- [x] Slice 4 `calibrations-uploader` (2026-10-20): `calibrations/uploader.rs`, `mocks/calibration_sender.rs`, `SleepController::attempted_sleeps`; 25 uploader tests.
- [x] Slice 5 `calibration-intake-service` (2026-10-20): `services/calibration/` at 100% region coverage, so its `.covgate` is `100.00`.
- [x] Slice 6 `calibrations-wiring` (2026-10-20): extra test `app::run::runs_without_calibrations_when_the_buffer_cannot_open`.
- [x] Review rework (2026-10-20, review of #331): intake validation dropped; the `SharedBuffer` (mutex plus `Notify`) and the `workers/calibrations.rs` driver were replaced by the `calibrations::Uploader` actor, which owns the buffer.

## Surprises & Discoveries

- `AuthnErr` forwards `is_terminal()` to its inner `HTTPErr`, so a token request Miru answers with, say, 403 is "terminal". Passed through unchanged, that would make the worker park every buffered report while token issuance fails. Evidence: `crate::impl_error!(AuthnErr { .., HTTPErr, .. })` in `agent/src/authn/errors.rs`.

## Decision Log

- Decision: Deduplication (`report_id` or idempotency key) is out of scope; a response lost mid-upload, or a shutdown between Miru's 200 and the local delete, can store a calibration twice. Date/Author: 2026-10-19, user.
- Decision: Keep the field name `calibrations::Job::calibrated_at`; it means "when the agent received the report". The sender maps it to the backend request's `reported_at`, and the service returns it as `reported_at`. Date/Author: 2026-10-20, user brief.
- Decision: A report Miru rejects with a terminal 4xx is parked: its item file moves to `<data>/calibrations/failed/<entry id>.json`, its queue entry is dropped, and an error is logged. Parking is a generic `queue::DirQueue::park`, not calibration code. Date/Author: 2026-10-20, user brief.
- Decision: Retry with the data-upload backoff values (10 s base, ×2 growth, 1 h cap). Network errors wait a flat 10 s and do not grow the backoff; 5xx, 401, 408, 429 and other non-terminal errors grow it. Reports are never dropped for age or attempt count; the report cap (intake answers 507 when full) is the backstop. Rationale: the agent promised the caller durability. Date/Author: 2026-10-20, plan author.
- Decision: On a retryable failure the report stays at the head of the buffer and the worker waits out the backoff, instead of calling `DirQueue::requeue` (which moves the entry to the tail and stamps a per-entry deadline). Rationale: the brief also requires FIFO draining, and per-entry requeue would let every other ready report fire its own failing request while Miru is down (a burst of up to 1,000 requests every 10 s). Holding the head gives strict FIFO and one request per backoff interval. The backoff streak is in memory only, so a restart retries at once. Date/Author: 2026-10-20, plan author.
- Decision: The buffer is owned by one actor, `calibrations::Uploader`, in the style of `data_uploads::upload::Uploader`: intake sends it `Enqueue` over `mpsc` with a `oneshot` reply, and the actor uploads reports itself. While a send or a backoff is in flight it `select!`s on that and its command channel, so intake is answered at once and never waits on the network. Rationale: no lock, one owner, and the same pattern as data uploads; an earlier revision shared a `Mutex<Buffer>` plus a `Notify` wake-up. Date/Author: 2026-10-20, user (review of #331).
- Decision: "Reconnect" is `sync::syncer::SyncEvent::SyncSuccess` from `SyncerExt::subscribe`. The poller and backend sync requests over MQTT (`workers/mqtt.rs`) both drive syncs, and a successful sync proves Miru is reachable. A `SyncSuccess` ends any backoff wait. `AppState` subscribes and hands the receiver to the uploader, so the actor needs no syncer. Date/Author: 2026-10-20, plan author.
- Decision: The buffer is `queue::DirQueue`, the directory-backed variant of `queue::Queue`, beside it in the queue module the way `DirCache` sits beside `FileCache`. A full one returns the existing `QueueFullErr`. The 507 keeps its `calibration_buffer_full` code. Date/Author: 2026-10-20, user (review of #331).
- Decision: The buffer caps the number of reports and each report file's size, not the total bytes (1,000 reports of up to 2 MiB, so 2 GiB at worst). Rationale: simpler, with no per-item size tracking, and a low chance of failing. Date/Author: 2026-10-20, user.
- Decision: Intake does no validation of its own; Miru is the only judge, and a report it rejects is parked in `failed/` by the worker. Rationale: the checks duplicated Miru's rules, and the user preferred silent parking until a report-status endpoint exists over keeping two copies of the rules in sync. Date/Author: 2026-10-20, user (review of #331).
- Decision: An error reading the deployments cache fails intake (500) rather than stamping `deployment_id: None`, so a broken cache never silently drops the association. No deployed deployment is fine and stamps `None`. Date/Author: 2026-10-20, plan author.
- Decision: If the queue fails to open at boot, the error is logged and `AppState` falls back to `DirQueue::new`, so calibrations run without persistence and the agent still boots (fail-open, like the data upload queues, which run without their snapshot file). Reports still queued at a restart are then lost. `AppState::calibrations` is always set. Supersedes the earlier `Option` field that left calibrations unavailable. Date/Author: 2026-10-20, user (review of #331).
- Decision: The sender's token failures are `CalibrationsErr::TokenErr(TokenErr(AuthnErr))` rather than a bare `AuthnErr` variant. `TokenErr` keeps `is_network_conn_err()` but is never terminal, so only a rejection of the report itself (an HTTP 4xx on `POST /calibrations`, or an unsendable report) parks it. Date/Author: 2026-10-20, implementer.
- Decision: The worker drops (removes) a queue entry whose report file is missing, since there is nothing to send or park and leaving it at the head would block every later report until restart. A report file that fails to parse is parked (terminal `InvalidReportErr`), and any other read error is a counted retry. The private `Outcome` is `Resolved | Retry { network }`: sent, parked and dropped all reset the streak, and only the logs tell them apart. Date/Author: 2026-10-20, implementer.
- Decision: `ServiceErr::CalibrationsErr` holds a `Box<CalibrationsErr>`, because the unboxed variant grows `ServiceErr` to 128 bytes and trips clippy's `result_large_err` across `server/`. The size and syntax failures of `content.data` name the field `content.data`; a `content` that isn't a `{ format, data }` object names `content`. Date/Author: 2026-10-20, implementer.
- Decision: Each send is one HTTP request (no `http::with_retry`); the worker's backoff is the only retry loop. If parking fails, the report is kept and handled like a counted retry. Date/Author: 2026-10-20, plan author.
- Decision: `HttpSender` treats a `POST /calibrations` whose response body cannot be parsed (`HTTPErr::UnmarshalJSONErr`, which `http::client::fetch` returns only after a 2xx) as sent, with a warning. Miru has stored the report, so retrying would store it again on every attempt and hold the head of the buffer forever, for example if the generated `Calibration` model drifts from Miru's response. Date/Author: 2026-10-20, review.
- Decision: `Queue` takes a `Persistence` (`BestEffort` or `Durable`) when built from a snapshot, replacing the separate `enqueue` and `enqueue_durable`. A queue's durability is fixed by what it promises its producers, so it is set once rather than per call. `DirQueue` now writes the queue entry first and the item file second, removing the entry if the file write fails; a crash between the two leaves an entry without a file, which `open` already drops. Date/Author: 2026-10-21, user (review of #331).
- Decision: Queue entry ids are UUIDv7, so they sort in enqueue order, and a `queue.json` rebuilt from item files orders entries by id. This replaces `DirQueueJob::received_at`, which existed only to order a rebuild and could not tell apart reports received in the same instant. `DirQueue` also drops its own `max_items` check, which repeated the inner `Queue`'s capacity check; the option is now `DirQueueOptions::capacity`. Date/Author: 2026-10-21, user (review of #331).
- Decision: No shared `actor` module for now. The calibrations uploader keeps its own channel, `serve_until` and `respond` plumbing, like the data upload uploader and the retention deleter on main. A shared module was tried (#343) and shelved. What the user wanted instead is each actor's state and logic in its own `<name>_impl.rs` file, apart from the worker that wraps it. Date/Author: 2026-10-21, user.
- Decision: A report Miru permanently rejects (or whose file can't be parsed) is dropped with an error log, not parked in `failed/`. `DirQueue::park` and `CalibrationsLayout::failed` are gone. Rationale: a rejection points to an agent bug, which is fixed by shipping a new agent, and a kept copy on the device doesn't help with that. Supersedes the parking decisions above. Date/Author: 2026-10-21, user (review of #331).
- Decision: `DirQueue` settles entries with `ack(id)` and `retry(id, attempts, next_attempt_at) -> bool`, matching `FileQueue` after #355, in place of `remove(id)` and `requeue(entry)`. `retry` of an id that is no longer queued returns `false`, so an acked report can't come back without its item file. Date/Author: 2026-10-21, user (review of #331).
- Decision: The calibrations queue's job type is `calibrations::ReportMetadata` in `calibrations/report.rs`, formerly `calibrations::Job` in `job.rs`, so the module speaks of reports rather than jobs. The generic queue keeps its `job` field names (`QueueEntry::job`, `Item::job`), so the on-disk format is unchanged. Date/Author: 2026-10-21, user (review of #331).
- Decision: `queue::DirQueue` is gone. The calibration uploader keeps report metadata in a plain durable `FileQueue<ReportMetadata>` (`calibrations/queue.json`) and each report's content in `calibrations/reports/<entry id>.json`, written, read and deleted by `UploaderImpl`. This drops the rebuild of `queue.json` from item files (and with it the need for UUIDv7 ids, so `FileQueue` is back to v4), the `.corrupt` set-aside (an unparseable report file is dropped), and the in-memory content fallback (with no queue file, reports are still written to `reports/`). The directory cache (`cache::DirCache`) was considered for the report files and not used: it rewrites a file on every read to update `last_accessed`, and evicts the least recently used files when over capacity. Supersedes the DirQueue, UUIDv7, `ack`/`retry` naming and Persistence-for-DirQueue decisions above where they conflict. Date/Author: 2026-10-21, user (review of #331).
- Decision: a report's format lives in its metadata. `ReportMetadata` carries `format: models::InstanceFormat` (a backend-mirrored `impl_status_enum!`), and `Report` is that metadata plus `data: String`. The report file `calibrations/reports/<entry id>` holds only the data, exactly as given, with no extension since the format varies. The sender builds `{ format, data }` from the two, so it cannot fail on a malformed content object; a report file that isn't UTF-8 is the only `InvalidReportErr`. The size cap measures the data alone, so it is Miru's 1 MiB. Intake's `CreateParams` takes `format` and `data` in place of `content`. Date/Author: 2026-10-22, user (review of #357).

## Outcomes & Retrospective

All six slices are implemented on `feat/calibrations-reference`. A booted agent opens `<data>/calibrations/` and spawns the `calibrations::Uploader` actor, which owns the buffer, answers intake at once even mid-upload, drains reports in intake order, backs off 10 s → 1 h on failures (flat 10 s while unreachable), resumes on a successful sync, parks Miru's terminal rejections (and unparseable report files) in `failed/`, and loses nothing on shutdown. `services::calibration::create` stamps and enqueues offline without validating; it has no caller until the Device API route lands. Region coverage (local run as root, so permission tests fail): `calibrations` 100%, `services/calibration` 100%, `buffer` 92.1%, `app` 94.4%, `server` 94.2%, `workers` 90.2%, all at or above their gates.

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

- `agent/src/queue/file.rs`: `FileQueue<J: QueueJob>`, a persisted FIFO of `QueueEntry<J> { id: Uuid, job, attempts, next_attempt_at }`. `next_ready(now)` returns the oldest entry that is due, without removing it.
- `agent/src/queue/dir.rs`: `DirQueue<J: QueueJob, C>` keeps one queue entry per item in `queue_file`, with the item's content in an *item file* `content_dir/<entry id>.json` holding `queue::Item { job, content }`. The API is `open(DirQueueOptions { capacity, max_item_bytes }, queue_file, content_dir)`, `new(options)` (in memory), `enqueue(Item) -> Uuid`, `next_ready`, `read(id)`, `ack(id)`, `retry(id, attempts, next_attempt_at)`, `len`, `iter`. When full, `enqueue` returns `DirQueueErr::QueueFullErr` and never evicts; an item file over `max_item_bytes` gives `ItemTooLargeErr`. There is no total byte cap. `open` deletes item files that have no queue entry and drops entries whose file is missing. The private helper `item_file(dir, id)` builds `<id>.json` paths. `DirQueue` is single-owner (`&mut self`) and has no `Debug` impl.
- `agent/src/calibrations/`: `Job { component_slug, role?, aspect?, serial_number?, deployment_id?, calibrated_at }` (`job.rs`); `DirQueue = queue::DirQueue<Job, serde_json::Value>`, `Report = queue::Item<Job, serde_json::Value>`, `DEFAULT_OPTIONS` (1,000 reports of up to 2 MiB each) and `open(&CalibrationsLayout, options)` (`buffer.rs`); `CalibrationsErr { QueueFullErr(CalibrationQueueFullErr) /* 507 calibration_buffer_full */, TooLargeErr(CalibrationTooLargeErr) /* 413 calibration_too_large */, DirQueueErr }` (`errors.rs`).
- `agent/src/disk/layout.rs`: `Layout::calibrations() -> CalibrationsLayout` with `root` (`<data>/calibrations`), `queue()` (`queue.json`) and `content()` (`content/`).
- `agent/src/http/calibrations.rs`: `create(client: &impl ClientI, CreateParams { payload: &CreateCalibrationRequest, token: &str }) -> Result<Calibration, HTTPErr>`, with a 60 s timeout. `libs/backend-api/src/models/create_calibration_request.rs`: `CreateCalibrationRequest { deployment_id?, component_slug, role?, serial_number?, aspect?, content: Box<InstanceContent>, reported_at: String }`. `InstanceContent { format: InstanceFormat, data: String }`. `InstanceFormat` is `json | yaml | jsonc | xml | text`, plus the catch-all `InstanceFormatUnknown` for any other string.
- Error classification, in `agent/src/http/errors.rs`: `RequestFailed::is_terminal()` is true for 4xx except 401, 408 and 429. Timeouts and connection failures report `is_network_conn_err() == true`. A *terminal* error means "Miru will never accept this request"; a *network* error means "Miru could not be reached".
- Patterns to copy: `agent/src/data_uploads/upload/executor.rs` (the `UploadExecutor` trait seam and `LiveExecutor<C: ClientI, T: TokenManagerExt>`); `agent/src/workers/poller.rs` (worker `run(…, sleep_fn, shutdown_signal)` that `tokio::select!`s a shutdown future against `run_impl`, and the syncer subscription via `crate::workers::next_sync_event`); `agent/src/cooldown/mod.rs` (`Backoff { base_secs, growth_factor, max_secs }`, `calc(&backoff, exp) -> i64` capped at `max_secs`).
- Shutdown: `agent/src/app/run.rs` turns the process's shutdown latch (`shutdown::Latch`: SIGTERM, ctrl-c or Windows SCM stop) into a `broadcast` channel. Each worker gets `shutdown_tx.subscribe()` as its shutdown future and registers its `JoinHandle` with `ShutdownManager`, which joins workers before `AppState::shutdown()` stops the token manager and storage.
- Deployment lookup: `disk::deployments::find_deployed(&disk::Deployments) -> Result<Option<models::Deployment>, DiskErr>`.
- Test mocks: `agent/tests/mocks/http_client.rs` `MockClient` already routes `POST /calibrations` to `create_calibration_fn` and records `Call::CreateCalibration` requests with their body and token. `mocks/stub_token_manager.rs` `StubTokenManager::{ok, err}`. `mocks/token_manager.rs` `MockTokenManager`. `mocks/syncer.rs` `MockSyncer` (`get_transmitter()` sends `SyncEvent`s; `set_subscribe_fail`). `mocks/error.rs` `SleepController` (records each requested sleep duration; `await_sleep()` / `release()`). `mocks/upload_executor.rs` is the model for a scripted mock.

## Interfaces and Dependencies

No new crates. New public items:

    // agent/src/queue/dir.rs (slice 2)
    impl<J: QueueJob, C> DirQueue<J, C> {
        pub async fn park(&mut self, id: Uuid, dest_dir: &Dir) -> Result<Option<QueueEntry<J>>, DirQueueErr>;
    }

    // agent/src/disk/layout.rs (slice 3)
    impl CalibrationsLayout { pub fn failed(&self) -> filesys::Dir; } // <root>/failed

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
    SendActorMessageErr(..), ReceiveActorMessageErr(..) // slice 4, the cache module's actor errors

    // agent/src/calibrations/uploader.rs (slice 4)
    #[derive(Debug, Clone)] pub struct UploaderOptions { pub backoff: cooldown::Backoff } // default 10 / 2 / 3600
    pub struct UploaderArgs<S, F> { pub queue: DirQueue, pub failed: Dir, pub sender: Arc<S>,
        pub events: Option<watch::Receiver<SyncEvent>>, pub options: UploaderOptions, pub sleep_fn: F }
    #[derive(Debug)] pub struct Uploader { .. } // handle; the actor owns the buffer
    impl Uploader {
        pub fn spawn<S: Sender + 'static, F, Fut>(buffer_size: usize, args: UploaderArgs<S, F>) -> (Self, JoinHandle<()>)
            where F: Fn(Duration) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static;
        pub async fn enqueue(&self, report: Report) -> Result<Uuid, CalibrationsErr>; // a full buffer is the 507
        pub async fn len(&self) -> Result<usize, CalibrationsErr>;
        pub async fn shutdown(&self) -> Result<(), CalibrationsErr>;
    }

    // agent/src/services/errors.rs (slice 5)
    ServiceErr::CalibrationsErr(Box<calibrations::CalibrationsErr>)

    // agent/src/services/calibration/ (slice 5)
    pub struct CreateParams { pub component_slug: String, pub role: Option<String>, pub aspect: Option<String>,
                              pub serial_number: Option<String>, pub content: serde_json::Value }
    pub struct Created { pub reported_at: DateTime<Utc> }
    pub async fn create(uploader: &Uploader, deployments: &disk::Deployments,
                        params: CreateParams, now: DateTime<Utc>) -> Result<Created, ServiceErr>;

    // agent/src/app/state.rs (slice 6)
    AppState { pub calibrations: Option<Arc<calibrations::Uploader>>, .. }
    ServerErr::CalibrationsErr(Box<calibrations::CalibrationsErr>)

## Plan of Work

Each milestone is one slice: it merges on its own with CI green, depends only on earlier slices, and ends with one commit. Estimated changed lines include tests.

| # | Slug | Scope | ~Lines | Depends on |
|---|------|-------|--------|-----------|
| 1 | `calibrations-buffer` | Generic `queue::DirQueue` and `calibrations::DirQueue` (exists: PR #324) | 1,870 | none |
| 2 | `buffer-park` | Generic `DirQueue::park` moves an item aside and drops its entry | 140 | 1 |
| 3 | `calibrations-sender` | `failed/` layout, `Sender`/`HttpSender`, error variants | 340 | 1 |
| 4 | `calibrations-uploader` | `calibrations::Uploader` actor: owns the buffer, serves intake, drains, backs off, parks, shuts down | 1,000 | 2, 3 |
| 5 | `calibration-intake-service` | `services::calibration::create`: stamp and enqueue, no validation | 220 | 4 |
| 6 | `calibrations-wiring` | `AppState` opens the buffer, spawns the uploader and stops it on shutdown; docs | 150 | 4, 5 |

### Milestone 2: `buffer-park`

In `agent/src/queue/dir.rs`, add `DirQueue::park` in the Mutators section after `remove`. If `id` isn't in the queue, return `Ok(None)` and touch nothing. Otherwise: `dirs::create_if_absent(dest_dir)`, then `files::move_to(&self.item_file(id), &item_file(dest_dir, id), Overwrite::Allow)`, then `self.queue.remove(id)`, and return the entry. The file moves first, so a crash between the two steps leaves an entry whose file is gone; the next `open` drops that entry, and the parked copy survives. Removing the entry first would leave an orphan that `open` deletes. Any error before the entry is removed is returned, with the entry and its item file unchanged. The doc comment states this ordering and that `open` never looks inside `dest_dir`. Add one sentence to the `buffer` paragraph of `ARCHITECTURE.md`.

Tests, in a new `pub mod park` in `agent/tests/queue/dir.rs`, using the file's existing `TestJob` and helpers:

- `moves_the_item_file_and_drops_the_entry`: enqueue `a` and `b`, then park `a` into `<tmp>/failed`. It returns `Some(entry a)`, and `entries()` holds only `b`. `failed/<a>.json` parses as `a`'s `Item`, and `content/<a>.json` is gone.
- `parked_items_stay_out_after_reopen`: park `a`, reopen: `entries()` holds only `b`, and the parked file still exists.
- `unknown_id_is_none`: returns `Ok(None)`; `len` unchanged; `dest_dir` not created.
- `failure_keeps_the_entry`: a regular file occupies the `dest_dir` path, so park returns `Err`. `len` and `content/<a>.json` are unchanged.

### Milestone 3: `calibrations-sender`

`agent/src/disk/layout.rs`: add `CalibrationsLayout::failed()` returning `self.root.subdir("failed")`, and extend `calibrations_layout` in `agent/tests/disk/layout.rs` with a `failed_dir` assertion (`under_root(&["calibrations", "failed"])`).

`agent/src/calibrations/sender.rs` (new): the `Sender` trait, with a doc comment on cancel safety. The uploader drops an in-flight `send` on shutdown, and the report stays buffered and is re-sent next boot (it may be stored twice). `new_request` maps the job's fields one-to-one, sets `reported_at` to `calibrated_at.to_rfc3339()`, and deserializes `report.content` with `InstanceContent::deserialize(&report.content)`. A failure becomes `InvalidReportErr`. `HttpSender::send` runs `new_request(report)?`, then `self.token_mngr.get_token().await?`, then a single `http::calibrations::create(..).await?`, and returns `Ok(())`.

`agent/src/calibrations/errors.rs`: add `InvalidReportErr { reason: String, trace: Box<Trace> }` with `is_terminal() -> true`. Add the variants `InvalidReportErr`, `HTTPErr` and `AuthnErr` to `CalibrationsErr`, with `From` impls and entries in `impl_error!`. `agent/src/calibrations/mod.rs`: `pub mod sender;` and re-export `Sender`, `HttpSender` and `InvalidReportErr`. `agent/tests/mocks/http_client.rs`: add `set_create_calibration(f)`, mirroring `set_confirm_upload`. `ARCHITECTURE.md`: extend the `calibrations/` paragraph.

Tests (`calibrations/.covgate` is 100%, so cover every branch), in `agent/tests/calibrations/sender.rs`, declared in `agent/tests/calibrations/mod.rs`:

- sender: `new_request_maps_the_job` (all optional fields set; `reported_at` equals `calibrated_at.to_rfc3339()`; content equals the parsed `InstanceContent`). `new_request_omits_absent_fields` (serialized JSON has no `deployment_id`, `role`, `aspect` or `serial_number` keys). `new_request_rejects_unparseable_content` (content `{"offset": 1}` gives `InvalidReportErr`, `is_terminal()`). `send_posts_one_request_with_the_token` (`MockClient` and `StubTokenManager::ok("tok")`: exactly one `Call::CreateCalibration` request, token `tok`, body containing `"reported_at"`). `send_passes_a_422_through_as_terminal`. `send_reports_a_network_error` (`MockErr { is_network_conn_err: true }`). `send_returns_a_token_error` (`StubTokenManager::err`, no request made). `send_rejects_unparseable_content_before_any_request`.

### Milestone 4: `calibrations-uploader`

`agent/src/calibrations/uploader.rs` (new; `pub mod uploader;` and re-export `Uploader`, `UploaderArgs` and `UploaderOptions` in `calibrations/mod.rs`). It follows `data_uploads/upload/uploader.rs`: a private `Worker` owns the receiver, the `DirQueue`, the `failed` dir, `Arc<S>`, the optional sync-event receiver, the options, `sleep_fn` and the backoff streak. Each function stays under 50 lines.

- `UploaderOptions` with `Default` `backoff: cooldown::Backoff { base_secs: 10, growth_factor: 2, max_secs: 3600 }`, mirroring `upload::UploaderOptions::default().backoff`.
- `enum Command { Enqueue { report: Box<Report>, respond_to }, Len { respond_to }, Shutdown { respond_to } }`. A closed channel counts as shutdown.
- `Outcome { Resolved, Retry { network: bool } }` and `struct Streak(u32)` with `next_wait`: `Resolved` resets to 0 and returns `None`; a network retry returns `base_secs` and leaves the streak; any other retry increments it and returns `cooldown::calc(backoff, streak - 1)`.
- `run`: loop on `buffer.next_ready(Utc::now())`. With a report, `attempt(id)`; with none, `idle()`. Stop on `Flow::Shutdown`.
- `upload(id)`: `buffer.read(id)`. A missing file drops the entry; a `ParseJSONErr` becomes `InvalidReportErr` (terminal); other read errors retry. The send runs as an owned future (cloned `Arc<S>` and the read report) through `serve_until`, which `select!`s it against the command channel. `None` means a shutdown arrived; the report stays buffered.
- `resolve(id, result)`: `Ok` removes it. A terminal error parks it in `failed/`; a failed park retries without the network flag. Any other error retries, keeping the entry at the head.
- `back_off(wait)`: pin `sleep_fn(wait)` once, then `select!` the sleep, commands, and sync events. `SyncSuccess` ends it early; an ended stream sets `events = None`. The sleep is never restarted, so intake can't extend it.
- `idle()`: `select!` a command or a sync event.
- The `Uploader` handle: `spawn`, `enqueue`, `len` and `shutdown` round-trip through `send_command`, mapping a closed channel to `SendActorMessageErr` and a dropped reply to `ReceiveActorMessageErr` (new `CalibrationsErr` variants, aliases of the cache module's types).

`agent/tests/mocks/calibration_sender.rs` (new; declared in `agent/tests/mocks/mod.rs`): `MockCalibrationSender`, a scripted `VecDeque<Step>` (`Ok`, `Terminal` = 422, `Server` = 500, `Network`, `Hang(oneshot::Receiver<()>)`; an empty script succeeds). It records each sent report and signals an unbounded `started` channel on every call.

Tests in `agent/tests/calibrations/uploader.rs`, with a 60 s hang guard. The harness opens the buffer itself, so a test can enqueue into it or damage its files before `spawn_with(buffer)`. Sync events come from a `watch` channel the harness holds. Cases: drain order of reports buffered before start; intake wakes an idle uploader; intake is answered during an in-flight send and during a backoff (which it doesn't extend); a full buffer is 507; offline then `SyncSuccess` resumes in order; other events don't end a wait; idle events are ignored; backoff `[10, 20, …, 2560, 3600, 3600]`; network waits a flat 10 s; a success resets the streak; terminal rejections and unreadable files are parked; a missing file is dropped; a failed park and a read error retry; shutdown mid-send and mid-backoff keep the report for the next boot; dropping every handle stops the actor; commands after shutdown fail with `SendActorMessageErr`, and one queued behind shutdown fails with `ReceiveActorMessageErr`; an abandoned reply doesn't stop the actor; no events, or an ended stream, still drains.

### Milestone 5: `calibration-intake-service`

`agent/src/services/errors.rs`: add the `ServiceErr` variant `CalibrationsErr(Box<CalibrationsErr>)` with its `From` impl and `impl_error!` entry.

`agent/src/services/calibration/create.rs` (new): `CreateParams`, `Created` and `create`. Intake does not validate (see Decision Log). `create` takes the deployment id from `disk::deployments::find_deployed(deployments).await?.map(|d| d.id)`, builds a `Report` whose `Job` has `calibrated_at: now` and whose content is the caller's value unchanged, calls `uploader.enqueue(report).await?`, and returns `Created { reported_at: now }`. `now` is a parameter, so tests control time; the future route passes `Utc::now()`. `agent/src/services/calibration/mod.rs`: `mod create; pub use create::*;`. Add `pub mod calibration;` to `agent/src/services/mod.rs`, add `agent/src/services/calibration/.covgate` (measured: `100.00`), and list `calibration` among the `services/` submodules in `ARCHITECTURE.md`.

Tests in `agent/tests/services/calibration/{mod.rs, create.rs}` (declare `pub mod calibration;` in `agent/tests/services/mod.rs`):

- create (an `Uploader` over a temp buffer and `MockCalibrationSender`): `accepted_while_offline` (no HTTP client exists in the call; returns `reported_at == now`; the uploaded job has `calibrated_at == now` and the caller's content). `stamps_the_current_deployment` (a `Deployed` deployment `dpl_1` in a temp `Deployments` store gives `deployment_id == Some("dpl_1")`). `no_deployment_is_fine` (`None`). `a_deployments_cache_error_fails_intake` (500, nothing enqueued). `buffer_full_is_507` (the first upload hangs so the report stays buffered; (`code().as_str() == "calibration_buffer_full"`, status 507). `an_oversized_report_is_413` (`calibration_too_large`, nothing enqueued).

### Milestone 6: `calibrations-wiring`

`agent/src/app/state.rs`: add the field `pub calibrations: Option<Arc<calibrations::Uploader>>`. Add `init_calibrations(layout, http_client, token_mngr, &syncer)`, which opens `calibrations::open(&layout.calibrations(), DEFAULT_OPTIONS)`; on error it logs `"failed to open the calibration buffer; calibrations are unavailable until restart: {e}"` and returns `(None, None)` (fail-open). Otherwise it subscribes to the syncer (logging and passing `None` on error), and spawns the `Uploader` with an `HttpSender`, `layout.failed()`, default options and `tokio::time::sleep`. Its join handle joins the others in the state's shutdown future. `AppState::shutdown` stops the uploader first, since it uses the token manager and HTTP client and nothing depends on it. `agent/src/server/errors.rs`: add `ServerErr::CalibrationsErr(Box<CalibrationsErr>)` with its `From` impl. `agent/src/app/run.rs` is unchanged. Update `ARCHITECTURE.md`.

Tests in `agent/tests/app/state.rs`, module `init`:

- `calibration_uploader_spawned`: `state.calibrations` is `Some`, its `len()` is 0, and `<data>/calibrations/content` exists.
- `calibrations_degrade_when_buffer_path_unwritable`: a regular file at `layout.calibrations().root` gives `state.calibrations.is_none()`; init and shutdown succeed.

`agent/tests/app/run.rs` adds `runs_without_calibrations_when_the_buffer_cannot_open`; the existing start/shutdown tests exercise spawning and stopping the uploader.

## Concrete Steps

All commands run from the agent repo root, `/home/claude/agent`, on branch `feat/calibrations-reference`. Make the edits for a milestone as described in Plan of Work, validate it locally (see Validation), then commit it once.

1. Before milestone 2, confirm the plan is active: `git ls-files plans/active/20261020-calibrations-upload-worker-and-intake.md` prints the path.
2. Milestone 2 commit:

        git add agent/src/queue/dir.rs agent/tests/queue/dir.rs ARCHITECTURE.md
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
- A 422 from Miru drops the report and logs an error; the report is never re-sent and later reports still upload (`terminal_rejection_is_dropped_not_retried`).
- Shutdown during an upload loses nothing: after restart the report is sent again (`shutdown_mid_upload_keeps_the_report_for_replay`).
- `services::calibration::create` with Miru offline returns `{ reported_at: now }` and buffers the report with the current deployment id, or `None` when nothing is deployed (`accepted_while_offline`, `stamps_the_current_deployment`, `no_deployment_is_fine`).
- Intake accepts any report shape without checking it. A full buffer gives 507 `calibration_buffer_full`, and a report whose file would exceed 2 MiB gives 413 `calibration_too_large`.
- A booted agent has `<data>/calibrations/content/`. A blocked calibrations path still boots, with calibrations running in memory (`calibration_uploader_spawned`, `calibrations_run_in_memory_when_queue_path_unwritable`).

## Idempotence and Recovery

Every edit is additive except the new `CalibrationsErr`, `ServiceErr` and `Code` variants, which compile-break only exhaustive matches the plan already names (`agent/tests/errors/mod.rs`). Re-running a milestone's edits is safe; if a commit already exists, amend it rather than adding a second one. If a slice fails CI, fix it on that slice's branch; earlier slices are unaffected because no slice depends on a later one. Any milestone commit can be reverted alone, with its dependents reverted first (see the table).

The on-disk format is unchanged: `queue.json` and `content/<id>.json` keep the slice 1 shape, and `failed/` is a new sibling that `DirQueue::open` never reads. An older agent without the worker leaves buffered reports in place, and a newer one uploads them. Reverting milestone 6 stops uploads but keeps every buffered report on disk. Parked reports are never deleted automatically; remove `<data>/calibrations/failed/*.json` by hand once inspected.
