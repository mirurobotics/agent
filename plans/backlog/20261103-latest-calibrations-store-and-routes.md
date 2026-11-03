# Keep the latest calibrations on the device and serve them on the Device API

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent at `/home/user/agent`; crate `miru-agent` in its `agent/` directory) | read-write | The latest calibrations store, write-through on `POST /v0.2/calibrations`, upload status from the uploader, `GET /v0.2/calibrations/latest`, `GET /v0.2/calibrations/{report_id}/content`, the reserved `null` slug, tests and `ARCHITECTURE.md`. |
| `openapi/` | read-only | The Device API contract (openapi#416), already vendored in this branch. |

This plan lives in `agent/plans/` because all code changes are in the agent. It is "agent PR A" of Linear ENG-1726, which implements "RFC: Latest Calibrations on the Device". Base branch `main`; working branch `claude/project-thread-j97cpg`. Every path below is relative to the agent repo root.

## Purpose / Big Picture

Device apps (ATI, Monarch's Dinghy) load calibrations at startup, often offline. Today an app can report a calibration with `POST /v0.2/calibrations`, but nothing on the device can read one back: the agent deletes a report once it uploads. After this change the agent keeps the newest calibration per (component, role, aspect) on disk and serves it:

- `POST /v0.2/calibrations` writes the report into the latest store before it returns, so the next read includes it. A role or aspect of `null` is rejected with 422.
- `GET /v0.2/calibrations/latest?component=arm&role=null` returns `{"is_confirmed": false, "calibrations": [...]}`, one item per key, each with `report_id`, `upload_status` (`queued`, then `uploaded` once Miru has it, or `dropped` if the agent gives up), the slugs, `serial_number`, `reported_by` and `reported_at`. It never calls Miru.
- `GET /v0.2/calibrations/{report_id}/content` returns the reported bytes with `Content-Type` set from the format, and 404 for any `report_id` that is not a latest entry.

`is_confirmed` stays false in this PR: only "agent PR B" (the initial load from Miru during sync) sets the store's `loaded_at`.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `calibrations::LatestStore` (`agent/src/calibrations/latest.rs`) | `LatestCalibrations`, `CalibrationCache` | "Store" is the RFC's word. `Cache` would read as the `cache` module's LRU caches, which evict; this never evicts. |
| `calibrations::LatestIndex` (fields `loaded_at`, `entries`) and `LatestIndexFile` | `LatestSnapshot`, `LatestMetadata` | The RFC and ENG-1726 call it the index. `QueueSnapshot`/`QueueSnapshotFile` set the type + `StateFile` alias pattern. |
| `calibrations::LatestEntry` (fields `metadata: ReportMetadata`, `upload_status`) | `LatestCalibration`, `StoredCalibration` | One entry of the index, as `QueueEntry` is one entry of the queue. `Calibration` is the device wire type. |
| `calibrations::LatestFilter` (fields `component`, `role`, `aspect`) and `LatestFilter::is_match` | `LatestQuery`, `CalibrationFilter`; `matches` | Domain filter, apart from the wire query below. `is_match` reads as a question (shared rule). |
| `calibrations::LatestCalibrations` (fields `is_confirmed`, `entries`) | `LatestList`, `LatestListing` | Mirrors the wire `LatestCalibrationsResponse`. |
| `calibrations::LatestContent` (fields `format`, `data`) | `CalibrationContent`, `Content` | The content of a latest entry; `InstanceContent` already names the request's `{format, data}` wire type. |
| `Uploader::list_latest`, `Uploader::latest_content` | `latest`, `get_latest`; `content` | Verb first like `enqueue`/`len`; `latest_content` says which content. |
| `UploaderArgs::latest_index`, `UploaderArgs::latest_contents` | `latest: LatestStore` (prebuilt); `index_file`/`contents` | Mirror `snapshot_file`/`reports`, so `UploaderImpl::new` builds the store as it builds the queue. |
| `UploaderOptions::latest_capacity` | reuse `capacity`; `max_latest_keys` | Same shape as `capacity`; a separate field lets the queue and store caps differ and be tested apart. |
| `models::CalibrationUploadStatus` (`Queued`, `Uploaded`, `Dropped`) | `UploadStatus`, `LatestStatus` | The device twin's name; `UploadStatus` is the backend upload ledger's enum. |
| `Layout::latest_calibrations()` → `LatestCalibrationsLayout` with `index()` (`resources/calibrations/index.json`) and `contents()` (`resources/calibrations/contents/`) | `metadata.json` (as `config_instances`), `latest.json` | `CalibrationsLayout` is the sibling; `contents/` matches `config_instances`; `index.json` is the RFC's word, and the file holds `loaded_at` too, not only metadata. |
| `CalibrationStoreFullErr`, `Code::CalibrationStoreFull` (`calibration_store_full`, 507, params `max_keys`) | reuse `calibration_buffer_full`; `latest_calibrations_full` | Pairs with `CalibrationBufferFullErr`. A separate code because a full queue drains as uploads finish and a full store never does. See Open questions. |
| `CalibrationsErr::StoreFullErr`, `CalibrationsErr::NotFoundErr` | `LatestFullErr`; `LatestNotFoundErr` | Follow `QueueFullErr`/`TooLargeErr`: the variant names the failure, the struct says whose. |
| `CalibrationNotFoundErr` (`resource_not_found`, 404) | `LatestCalibrationNotFoundErr`; reuse `cache::errors::CacheElementNotFound` | Same code and status as `CacheElementNotFound`, but owned by `calibrations`. |
| `services::calibration::list_latest`, `services::calibration::get_content` | `latest`, `content` | `get_*`/verb-first like `deployment::get_current`. |
| `server::request::LatestCalibrationsQuery` | `ListLatestCalibrationsQuery`, `CalSearchQuery` | Mirrors `sse::EventsQuery` and the response name. |
| `handlers::list_latest_calibrations`, `handlers::get_calibration_content` | `get_latest_calibrations`, `latest_calibrations`; `calibration_content` | The spec's operation ids `listLatestCalibrations` and `getCalibrationContent`. |

### Precedent

| New file | Mirrors |
|----------|---------|
| `agent/src/calibrations/latest.rs` | `agent/src/queue/file.rs` (`FileQueue`: in-memory state, optional `StateFile`, `verify_capacity`) and `agent/src/calibrations/report.rs` (record types and the `StateFile` alias); `prune_orphans` and one file per entry from `agent/src/calibrations/uploader_impl.rs`. |
| `agent/src/models/calibration_upload_status.rs` | `agent/src/models/instance_format.rs` (one enum per file) with the agent-only `impl_status_enum!` form of `DeviceStatus` (`agent/src/models/device.rs`). |
| `agent/src/services/calibration/latest.rs`, `agent/src/services/calibration/content.rs` | `agent/src/services/calibration/create.rs`; one file per operation like `agent/src/services/deployment/{get,current}.rs`. |
| `agent/tests/calibrations/latest.rs` | `agent/tests/calibrations/report.rs` (raw-JSON wire pin) and `agent/tests/calibrations/uploader.rs` (temp-dir fixture). |
| `agent/tests/models/calibration_upload_status.rs` | `agent/tests/models/instance_format.rs` (`StatusEnumFixture` + `status_enum_tests!`), agent twin only like `DeviceStatus`. |
| `agent/tests/services/calibration/latest.rs`, `agent/tests/services/calibration/content.rs` | `agent/tests/services/calibration/create.rs`. |

The null-slug check copies agent#416 (commit 7af92c8, reverted by 60f93de) in `agent/src/server/request.rs`, with its request and route tests. No existing agent route serves raw bytes, so the content response mirrors Miru's `GET /config_instances/{id}/content`: `Content-Type` from `mirurobotics/core` `schemas.InstFormat.MediaType()` (json `application/json`, yaml `application/yaml`, jsonc `application/jsonc`, xml `application/xml`, csv `text/csv`, text `application/octet-stream`).

The store is owned by the calibration uploader's actor, beside its queue, not by a new actor: intake, write-through and status updates then run on one thread with no locks, `enqueue` can roll the queue back when the store write fails, and the uploader sets status without messaging a second actor. A separate store actor was ruled out because the service would have to undo a queued report across two actors. Reads go through the same actor, which serves commands during uploads and backoffs, so they never wait on Miru.

### Left out

- The initial load from Miru (backend client for the Agent API `GET /calibrations/latest` and content, the sync step, merging queued reports, writing `loaded_at`): agent PR B. The store's API leaves room for a `LatestStore::load(entries, loaded_at)` that writes each content file, then the index with every entry and `loaded_at` in one atomic write, then prunes unreferenced content.
- Re-vendoring `api/specs/device/v02.yaml` and `api/specs/backend/v05.yaml` from tagged releases: follow-up once openapi#416 merges and `device`/`agent` are tagged.
- Ordering the list: entries come back in index order (a key keeps its place when replaced, a new key goes last). The spec documents no order.
- Reconciling the store with the queue at startup: a crash between the queue write and the store write leaves a queued report missing from the store. The app got no 200, so it reports again and the newer report wins. PR B's load also covers a reset.
- A `reset` change or test: `disk::setup::reset` already deletes `resources/`, and `wipes_resources_subtree` in `agent/tests/disk/setup.rs` covers it.
- Repeated query keys (`?component=a&component=b`): the spec uses `style: form, explode: false`, a comma-separated list. A repeated key is rejected with 422.
- Fanning out a pattern: this change has one resource, so there is no exemplar/sibling split.

### Open questions

- How many keys may the store hold, and what happens past it? Default: `UploaderOptions::latest_capacity` 1,000 (the queue's cap, RFC lean); a report for a new key past it is rejected with 507 `calibration_store_full` and nothing is written; a report for an existing key always replaces it.
- Should a full store reuse `calibration_buffer_full`? Default: no, `calibration_store_full`, since retrying after uploads drain never helps.
- Which drops mark an entry `dropped`? Default: every drop (Miru rejected it, out of attempts, or still failing 90 days after `reported_at`), since none of them ever reaches Miru.
- What does `GET /calibrations/{report_id}/content` return for a `report_id` that is not a UUID? Default: 404 `resource_not_found`, like any other id that is not a latest entry.

## Progress

- [ ] Milestone 1: model enum, layout, errors and `LatestStore` (source).
- [ ] Milestone 2: uploader write-through, upload status, handle commands, `AppState` wiring (source).
- [ ] Milestone 3: services, request/response conversions, handlers, routes, `ARCHITECTURE.md` (source).
- [ ] Milestone 4: tests.
- [ ] Milestone 5: push and reach `CLEAN` via `$preflight` before the PR leaves draft.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

(Add entries as you go.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent (`agent/src/`) is a Rust daemon on a robot. Its Device API is an axum router (`agent/src/server/routes.rs`) served over a Unix socket and loopback TCP; handlers live in `agent/src/server/handlers.rs` and call the service layer (`agent/src/services/`). Handlers never convert wire types: request conversions live in `agent/src/server/request.rs` and response conversions in `agent/src/server/response.rs`. Wire types are generated into `libs/device-api/src/models/` from `api/specs/device/v02.yaml`; never edit them. Commit 4edf425 on this branch already vendored openapi#416 (snapshot build 6b8fc0d) and regenerated `libs/`, adding the device `Calibration`, `CalibrationUploadStatus`, `LatestCalibrationsResponse` and `CalSearch` models and the two routes in the spec. In code, `device_api::models` is imported as `device_server` (handlers) or `agent_server` (models).

Calibration intake today: `POST /v0.2/calibrations` → `handlers::create_calibration` → `CreateParams::try_from` (request.rs, shape checks only) → `services::calibration::create` (`agent/src/services/calibration/create.rs`), which mints a `report_id`, stamps `reported_at` and the deployed deployment, and calls `Uploader::enqueue`. `Uploader` (`agent/src/calibrations/uploader.rs`) is an actor: a `Worker` task owns `UploaderImpl` (`agent/src/calibrations/uploader_impl.rs`) and serves `Command`s (`Enqueue`, `Len`, `Shutdown`) even while an upload or backoff is in progress (`serve_until`, `back_off`). `UploaderImpl` holds a `Queue` (`queue::FileQueue<ReportMetadata>`, persisted to `calibrations/queue.json` through `QueueSnapshotFile`, a `filesys::state_file::StateFile` that writes atomically) and writes each report's data to `calibrations/reports/<entry id>`. `UploaderImpl::finish_attempt` removes a sent report (`remove`: ack the entry, then delete the file) and on failure asks `RetryPolicy::on_failure` for a `Verdict`: `Retry`, or a drop (`Terminal`, `OutOfAttempts`, `TooOld` — still failing 90 days after `reported_at`). `prune_orphans` deletes report files with no queue entry at startup. `ReportMetadata` (`agent/src/calibrations/report.rs`) holds `report_id`, `component`, `format` (`models::InstanceFormat`), `role`, `aspect`, `serial_number`, `reported_by`, `deployment_id` and `reported_at`. `AppState::init_calibration_uploader` (`agent/src/app/state.rs`) opens `queue.json`, falling back to an in-memory queue (`snapshot_file: None`) with an error log.

Disk layout: `disk::Layout` (`agent/src/disk/layout.rs`) names every path under the data root. Everything under `resources/` is a cache of Miru; `disk::setup::reset` (provisioning, reprovisioning, upgrades) deletes `resources/` and keeps `calibrations/`. The store goes under `resources/calibrations/` so `reset` wipes it with no change.

Errors: leaf errors derive `thiserror::Error` and implement `crate::errors::Error` (`code`, `http_status`, `params`); `Code` (`agent/src/errors/mod.rs`) holds the wire codes; `CalibrationsErr` (`agent/src/calibrations/errors.rs`) aggregates calibration errors with `impl_error!`, and flows through `ServiceErr` and `ServerErr` to the response status.

Status enums with a wire twin use `impl_status_enum!` (`agent/src/models/status.rs`); the agent-only form (`agent_type` only) generates the wire `Deserialize`, `as_str`, and `From<&T>` for the device type. Each gets a `StatusEnumFixture` in `agent/tests/models/`.

Terms: a "key" is the (component, role, aspect) triple, where role and aspect may be absent. A "latest entry" is the store's entry for a key. `loaded_at` is when the store was last loaded from Miru (PR B); `is_confirmed` is `loaded_at` being set.

## Plan of Work

### Write order and failure handling

These orders make every crash point leave a readable store and a queue that never loses a report the app was told is safe.

- Intake writes the queue entry and report file first, then the store. A failed store write removes the queued report (the existing `UploaderImpl::remove`) and fails POST, so nothing uploads that the app was told failed. Writing the store first would need the old entry and its content file restored when the queue write fails, and a crash between the two would leave a `queued` entry that never uploads. In this order a crash between the two leaves a report that uploads but is missing locally; the app got no 200, so it reports again and the newer report wins.
- The store writes the new content file (named by `report_id`, so it never overwrites the content the index still names), then the index atomically, then deletes the replaced entry's content file. A crash leaves at most an unreferenced file, which `LatestStore::prune_orphans` deletes at startup, as `UploaderImpl::prune_orphans` does for report files.
- The uploader sets an entry's status before it removes the report from the queue. A crash in between re-sends the report on the next boot (Miru records it once by `report_id`) and sets the status again; the reverse order could leave an uploaded report `queued` forever.

### Milestone 1: model, layout, errors and the store

`agent/src/models/calibration_upload_status.rs` (new): `pub enum CalibrationUploadStatus { #[default] Queued, Uploaded, Dropped }` with InstanceFormat's derives (`Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash`, `serde(rename_all = "snake_case")`) and `impl_status_enum!` in the agent-only form: `default: Queued`, `label: "calibration upload status"`, `log: warn`, `agent_type: agent_server::CalibrationUploadStatus`, mappings `queued`/`uploaded`/`dropped` to `CALIBRATION_UPLOAD_STATUS_*`. Declare it in `agent/src/models/mod.rs` and re-export `CalibrationUploadStatus`.

`agent/src/disk/layout.rs`: add `Layout::latest_calibrations(&self) -> LatestCalibrationsLayout` returning `LatestCalibrationsLayout::new(self.resources().subdir("calibrations"))`, and `LatestCalibrationsLayout { pub root }` after `CalibrationsLayout` with `new`, `index()` (`index.json`) and `contents()` (`contents/`), and a one-line doc comment saying it is the latest store, a cache of Miru that `reset` wipes.

`agent/src/errors/mod.rs`: add `Code::CalibrationStoreFull` → `"calibration_store_full"`.

`agent/src/calibrations/errors.rs`: add `CalibrationStoreFullErr { pub capacity: usize, pub trace }` (507, `Code::CalibrationStoreFull`, params `{"max_keys": capacity}`, doc: nothing was written) and `CalibrationNotFoundErr { pub report_id: String, pub trace }` (404, `Code::ResourceNotFound`, message "no latest calibration has report id {report_id}"). Add variants `StoreFullErr` and `NotFoundErr` to `CalibrationsErr`, `From` impls, and the `impl_error!` list.

`agent/src/calibrations/latest.rs` (new; `pub mod latest;` in `agent/src/calibrations/mod.rs`, re-exporting the types below):

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct LatestEntry { pub metadata: ReportMetadata, pub upload_status: CalibrationUploadStatus }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    pub struct LatestIndex {
        /// When the store was last loaded from Miru; `None` until then.
        #[serde(default)] pub loaded_at: Option<DateTime<Utc>>,
        #[serde(default)] pub entries: Vec<LatestEntry>,
    }
    impl Patch<LatestIndex> for LatestIndex { replace, as QueueSnapshot does }
    pub type LatestIndexFile = StateFile<LatestIndex, LatestIndex>;

    /// Empty means any; `None` in `role`/`aspect` matches an entry without one.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LatestFilter { pub component: Vec<String>, pub role: Vec<Option<String>>, pub aspect: Vec<Option<String>> }
    impl LatestFilter { pub fn is_match(&self, metadata: &ReportMetadata) -> bool }

    pub struct LatestCalibrations { pub is_confirmed: bool, pub entries: Vec<LatestEntry> }
    pub struct LatestContent { pub format: InstanceFormat, pub data: Vec<u8> }

    pub struct LatestStore { index: LatestIndex, index_file: Option<LatestIndexFile>, contents: Dir, capacity: usize }

`LatestStore` methods, each with a short doc comment:

- `new(index_file: Option<LatestIndexFile>, contents: Dir, capacity: usize) -> Self`: the index is read from the file, or empty without one (in memory, like `Queue::new`).
- `verify_capacity(&self, metadata: &ReportMetadata) -> Result<(), CalibrationStoreFullErr>`: Ok when the key already has an entry or there is room; otherwise log a warn and return the error. Keys compare `component`, `role` and `aspect` (a private `has_key(entry, metadata) -> bool` helper).
- `async put(&mut self, metadata: ReportMetadata, data: &[u8]) -> Result<(), CalibrationsErr>`: `verify_capacity`; write `contents/<report_id>` with `files::write_bytes(.., WriteOptions::OVERWRITE_ATOMIC)`; build the next index, replacing the key's entry in place or appending, with `upload_status: Queued`; persist it (`index_file.write`, atomic); on a persist error delete the new content file and return the error with the in-memory index unchanged; then set the in-memory index and delete the replaced entry's content file (log a warn on failure).
- `async set_upload_status(&mut self, report_id: Uuid, status: CalibrationUploadStatus)`: find the entry with that `report_id`; none (a newer report replaced it, or `reset` wiped the store) is a no-op; otherwise set the status and persist, logging a warn on failure like `FileQueue::ack`.
- `list(&self, filter: &LatestFilter) -> LatestCalibrations`: `is_confirmed: index.loaded_at.is_some()`, entries that `filter.is_match`, in index order.
- `async content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr>`: `CalibrationNotFoundErr` unless an entry has that `report_id`; else `files::read_bytes` of its content file, with the entry's format. A missing content file is `CalibrationNotFoundErr` too.
- `async prune_orphans(&self)`: with an index file only, delete every file in `contents/` not named by an entry's `report_id`, as `UploaderImpl::prune_orphans` does (warn per deletion and on listing failure).

Commit (see Concrete Steps).

### Milestone 2: the uploader writes through and marks status

`agent/src/calibrations/uploader_impl.rs`:

- `UploaderOptions` gains `latest_capacity: usize` (doc: the most keys the latest store holds; a new key past it is rejected), default 1000; update the `Default` doc comment.
- `UploaderImpl` gains `latest: LatestStore`; `new` takes `latest_index: Option<LatestIndexFile>` and `latest_contents: Dir` and builds `LatestStore::new(latest_index, latest_contents, options.latest_capacity)`.
- `prune_orphans` also awaits `self.latest.prune_orphans()` (keep the queue's early returns from skipping it: call the store first, or restructure the queue part into its own helper).
- `enqueue`: after `verify_size`, call `self.latest.verify_capacity(&report.metadata)?`; enqueue a clone of the metadata; write the report file as today; then `self.latest.put(metadata, &bytes)`, and on error `self.remove(id).await` and return it. Update the doc comment: the report is in the latest store when this returns Ok, and on error nothing is queued or stored.
- `finish_attempt`: on `Ok`, `self.latest.set_upload_status(entry.job.report_id, Uploaded)` before `self.remove`. In `handle_failure`, each drop arm (`Terminal`, `OutOfAttempts`, `TooOld`) sets `Dropped` before `self.remove`; add a small `drop_report(&mut self, entry: &QueueEntry)` helper if the three arms would repeat two lines.
- Add `pub(crate) fn list_latest(&self, filter: &LatestFilter) -> LatestCalibrations` and `pub(crate) async fn latest_content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr>`, delegating to the store.
- Update the inline test `Fixture` (`uploader_on`) to pass a latest index and contents dir under the temp dir so existing tests compile.

`agent/src/calibrations/uploader.rs`:

- `UploaderArgs` gains `latest_index: Option<LatestIndexFile>` (doc: where the latest store's index persists; `None` keeps it in memory) and `latest_contents: Dir` (doc: one content file per latest entry); `spawn` passes them to `UploaderImpl::new`.
- `Command` gains `ListLatest { filter: LatestFilter, respond_to: oneshot::Sender<LatestCalibrations> }` and `LatestContent { report_id: Uuid, respond_to: oneshot::Sender<Result<LatestContent, CalibrationsErr>> }`, served in `Worker::handle`.
- `Uploader` gains `pub async fn list_latest(&self, filter: LatestFilter) -> Result<LatestCalibrations, CalibrationsErr>` and `pub async fn latest_content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr>` via `send_command`. Update `enqueue`'s doc: it also writes the latest store, and a full store is the 507 `StoreFullErr`. Update the `Uploader` doc to say it also owns the latest store.

`agent/src/app/state.rs` `init_calibration_uploader`: open `layout.latest_calibrations().index()` with `LatestIndexFile::new_with_default(.., Default::default())`, falling back to `None` with an error log as for the queue; pass it and `latest_calibrations().contents()` in `UploaderArgs`. Update the doc comment.

Keep the other `UploaderArgs` construction sites compiling by adding the two fields, under each fixture's temp dir: `agent/tests/test_utils/calibrations.rs` (`uploader`: `latest_index: None`, `latest_contents` under `dir`), `agent/tests/calibrations/uploader.rs` (`Harness::spawn_on`), `agent/tests/services/calibration/create.rs` (`Fixture::with_options`). Build the paths with `LatestCalibrationsLayout::new(..)` under the fixture's temp dir.

Commit.

### Milestone 3: services, server and docs

`agent/src/services/calibration/latest.rs` (new): `pub async fn list_latest(uploader: &Uploader, filter: LatestFilter) -> Result<LatestCalibrations, ServiceErr>`, doc: answers from the device's store and never calls Miru.

`agent/src/services/calibration/content.rs` (new): `pub async fn get_content(uploader: &Uploader, report_id: &str) -> Result<LatestContent, ServiceErr>`: parse `report_id` as a `Uuid`; a parse failure is `CalibrationsErr::NotFoundErr` (404); else `uploader.latest_content(id)`.

`agent/src/services/calibration/mod.rs`: declare and `pub use` both; update the module doc to "calibration intake and the latest calibrations".

`agent/src/server/request.rs`:

- Copy 7af92c8's null check into `TryFrom<CreateCalibrationRequest> for CreateParams`, with its doc comment ("Checks the request's shape and the reserved `null` slug, which the slug filters use to match a missing role or aspect; Miru judges the rest.") and message `"{field} 'null' is reserved"`.
- Add `#[derive(Debug, Default, Deserialize)] pub struct LatestCalibrationsQuery { pub component: Option<String>, pub role: Option<String>, pub aspect: Option<String> }` (doc: each is a comma-separated list, as the spec's `explode: false` sends it) and `impl From<LatestCalibrationsQuery> for LatestFilter`: split on `,`, trim, drop empty items (as `sse::parse_event_filter` does), and map `null` to `None` for role and aspect.

`agent/src/server/response.rs`:

- `impl From<&LatestEntry> for device_server::Calibration` (object `calibration::Object::Calibration`, `report_id`, `upload_status: (&entry.upload_status).into()`, slugs, `serial_number`, `reported_by`, `reported_at.to_rfc3339()`).
- `impl From<&LatestCalibrations> for device_server::LatestCalibrationsResponse`.
- `impl IntoResponse for LatestContent`: 200, `Content-Type` from a private `fn media_type(format: InstanceFormat) -> &'static str` with core's mapping (see Precedent), body the bytes. Comment the mapping's source in one line ("Miru's content downloads send the same types.").

`agent/src/server/handlers.rs`, under CALIBRATIONS:

- `list_latest_calibrations(AxumState(state), query: Result<Query<request::LatestCalibrationsQuery>, QueryRejection>)` through `handle`: a rejection is `InvalidRequestErr` with `rejection.body_text()` (422); else `calibration_svc::list_latest(&state.calibration_uploader, filter)` and `LatestCalibrationsResponse::from(&latest)`.
- `get_calibration_content(AxumState(state), Path(report_id): Path<String>) -> Response`: on Ok `content.into_response()`; on Err the JSON error. Extract the error arm of `handle` into `fn error_response(e: ServerErr, err_msg: &str) -> (StatusCode, Json<Value>)` and use it in both.

`agent/src/server/routes.rs`: under CALIBRATIONS add `GET /{api_version}/calibrations/latest` → `list_latest_calibrations` and `GET /{api_version}/calibrations/{{report_id}}/content` → `get_calibration_content`.

`ARCHITECTURE.md`, `calibrations/` paragraph: add that the uploader also owns the latest store at `resources/calibrations/` (`index.json` with `loaded_at` and one entry per key, `contents/<report_id>`), written through by intake before `POST` returns (queue first, store second, queue rolled back on a store failure), marked `uploaded`/`dropped` by the uploader before it dequeues, served by `GET /v0.2/calibrations/latest` and `/{report_id}/content` without calling Miru, capped at 1,000 keys (507 `calibration_store_full`), wiped by `reset` with the rest of `resources/`, and that `null` is a reserved role/aspect slug. In the `services/` list, extend `calibration` to "(calibration intake and the latest calibrations)".

Commit.

### Milestone 4: tests

Follow the repo's test rules (`CLAUDE.md` § Testing): route tests in `agent/tests/server/handlers.rs` check status codes and bodies only; storage behavior is tested at the store, uploader and service layers. Use `#[tokio::test]` and `test_dirs::temp(..)` fixtures as the siblings do. To force a write failure, create a directory at the file path, as `calibration_uploader_degrades_when_queue_path_unwritable` does.

`agent/tests/models/calibration_upload_status.rs` (new, `pub mod` in `agent/tests/models/mod.rs`): `StatusEnumFixture` with `type Backend = ()`, `type Agent = agent_server::CalibrationUploadStatus`, three cases, `status_enum_tests!(CalibrationUploadStatus, agent)`.

`agent/tests/disk/layout.rs`: `latest_calibrations_index` (`resources/calibrations/index.json`) and `latest_calibrations_contents` (`resources/calibrations/contents`).

`agent/tests/errors/mod.rs`: add `(Code::CalibrationStoreFull, "calibration_store_full")` to the code table.

`agent/tests/calibrations/errors.rs`: `a_full_store_is_507_with_its_cap` and `a_missing_latest_calibration_is_404`.

`agent/tests/calibrations/latest.rs` (new, `pub mod latest;` in `agent/tests/calibrations/mod.rs`), against `LatestStore` with an index file under a temp dir:

- `put_adds_a_queued_entry_and_its_content`
- `put_replaces_the_keys_entry_and_deletes_its_content`
- `role_and_aspect_are_part_of_the_key` (a missing role is its own key)
- `put_rejects_a_new_key_at_capacity_and_writes_nothing`, `put_replaces_an_existing_key_at_capacity`
- `put_fails_and_changes_nothing_when_the_index_cannot_be_written` (also: the new content file is gone)
- `set_upload_status_marks_the_matching_report`, `set_upload_status_ignores_a_replaced_report`
- `list_filters_by_component_role_and_aspect`, `list_matches_null_role_and_aspect`, `an_empty_filter_lists_every_entry`
- `is_confirmed_is_whether_loaded_at_is_set` (write an index with `loaded_at` and reopen)
- `content_returns_the_bytes_and_format`, `content_of_an_unknown_report_is_not_found`
- `the_index_survives_a_reopen`
- `prune_orphans_deletes_unreferenced_contents`, `an_in_memory_store_prunes_nothing`
- `mod wire`: `raw_json_index_loads`, pinning the shape of `index.json` as `raw_json_snapshot_loads` does for `queue.json`.

Inline tests in `agent/src/calibrations/uploader_impl.rs` (`mod tests`):

- `enqueue_writes_the_report_through_to_the_latest_store`
- `enqueue_rolls_back_the_queue_when_the_store_write_fails`
- `enqueue_rejects_a_new_key_when_the_store_is_full_and_queues_nothing`
- `a_sent_report_is_marked_uploaded`
- `a_dropped_report_is_marked_dropped` (cover `Terminal`, `OutOfAttempts` and `TooOld`)
- `a_newer_report_for_the_key_is_not_marked` (upload the older report after a newer one replaced it)
- `prune_orphans_also_prunes_latest_contents`

`agent/tests/calibrations/uploader.rs`: `latest_reads_are_served_during_an_upload` (hold an upload with the mock sender's `Step`, then `list_latest` and `latest_content` answer).

`agent/tests/app/state.rs`: in `calibration_uploader_spawned`, assert `layout.latest_calibrations().index()` exists; add `calibration_uploader_degrades_when_latest_index_path_unwritable` mirroring the queue test.

`agent/tests/services/calibration/latest.rs` and `content.rs` (new; `pub mod` in `agent/tests/services/calibration/mod.rs`), on `create.rs`'s fixture: `list_latest_includes_a_created_report`, `list_latest_applies_the_filter`; `get_content_returns_the_reported_bytes`, `get_content_of_an_invalid_report_id_is_not_found`. In `create.rs`, `create_fails_when_the_latest_store_is_full`.

`agent/tests/server/request.rs`: `a_null_role_is_rejected`, `a_null_aspect_is_rejected` (from 7af92c8); `latest_query_splits_comma_separated_values`, `latest_query_maps_null_to_none`, `an_empty_latest_query_filters_nothing`.

`agent/tests/server/response.rs`: `latest_entry_converts_to_calibration`, `latest_calibrations_convert_to_the_response`, `content_response_sets_the_content_type_for_each_format` (table over the six formats).

`agent/tests/server/handlers.rs`, `mod calibrations`: `create_calibration_returns_422_for_a_null_role` (from 7af92c8); `list_latest_calibrations_returns_200_with_a_reported_calibration` (is_confirmed false, one item); `list_latest_calibrations_returns_422_for_a_repeated_filter`; `get_calibration_content_returns_200_with_the_bytes_and_content_type`; `get_calibration_content_returns_404_for_an_unknown_report_id`; `create_calibration_returns_507_when_the_store_is_full`.

`agent/tests/server/errors.rs` and `agent/tests/services/errors.rs`: extend the calibration error mapping cases if they enumerate `CalibrationsErr` variants.

Commit.

### Milestone 5: validate in CI

Push the branch and run `$preflight` until it reports `CLEAN`. If `src/calibrations/.covgate` (99.64), `src/services/calibration/.covgate` (100.00), `src/server/.covgate` (87), `src/models/.covgate` (100) or `src/disk/.covgate` (96.79) fails, add the missing test; never lower a gate.

## Concrete Steps

All commands run from the agent repo root (`/home/user/agent`). No local build, lint or test commands: CI runs them.

1. Move the plan when implementation starts (the orchestrator does this): `git mv plans/backlog/20261103-latest-calibrations-store-and-routes.md plans/active/`.

2. Milestone 1 edits, then:

        git add agent/src/models agent/src/disk/layout.rs agent/src/errors/mod.rs agent/src/calibrations
        git commit -m "feat(calibrations): add the latest calibrations store"

3. Milestone 2 edits, then:

        git add agent/src/calibrations agent/src/app/state.rs agent/tests/test_utils/calibrations.rs agent/tests/calibrations/uploader.rs agent/tests/services/calibration/create.rs
        git commit -m "feat(calibrations): write reports through to the latest store"

4. Milestone 3 edits, then:

        git add agent/src/services agent/src/server ARCHITECTURE.md
        git commit -m "feat(server): serve the latest calibrations and their content"

5. Milestone 4 edits, then:

        git add agent/tests agent/src/calibrations/uploader_impl.rs
        git commit -m "test(calibrations): cover the latest store and its routes"

Each commit message ends with the two attribution lines the session gives (`Co-Authored-By: ...` and `Claude-Session: ...`). Check `git status` shows nothing unstaged under `agent/` after each commit.

6. Push and validate with `$preflight` (Milestone 5).

## Validation and Acceptance

Validation runs in CI (`.github/workflows/ci.yml`) on the pushed head, never locally: job `lint` (`scripts/lint.sh`: import linter, 50-line function limit, field-by-field assert check, `cargo fmt --check`, clippy `-D warnings`, machete, audit), job `test` (`scripts/covgate.sh`: the whole suite with per-module coverage gates), `windows-check`, and `tools`. The work is complete, and the PR leaves draft, only when `$preflight` reports `CLEAN` (CI green on the pushed branch head).

A green run shows:

- The new tests in Milestone 4 pass, including `put_replaces_the_keys_entry_and_deletes_its_content`, `enqueue_rolls_back_the_queue_when_the_store_write_fails`, `a_sent_report_is_marked_uploaded`, `a_newer_report_for_the_key_is_not_marked`, `raw_json_index_loads` and the route tests.
- Existing calibration tests still pass, so intake, retry and drop behavior is unchanged apart from the store.

Behavior a human can check on a device running this agent (Linux socket shown):

    curl -s --unix-socket /run/miru/miru.sock -X POST http://localhost/v0.2/calibrations \
      -H 'Content-Type: application/json' \
      -d '{"component":"arm","role":"front-left","content":{"format":"json","data":"{\"offset\":1}"}}'
    # 200 {"report_id":"<id>","reported_at":"..."}

    curl -s --unix-socket /run/miru/miru.sock 'http://localhost/v0.2/calibrations/latest?component=arm&aspect=null'
    # 200 {"is_confirmed":false,"calibrations":[{"object":"calibration","report_id":"<id>","upload_status":"queued",...}]}
    # upload_status becomes "uploaded" once the agent reaches Miru

    curl -si --unix-socket /run/miru/miru.sock http://localhost/v0.2/calibrations/<id>/content
    # 200, Content-Type: application/json, body {"offset":1}

Reporting the same key again returns a new `report_id`; the list then shows only the new one and the old id's content returns 404. A report with `"role":"null"` returns 422 `invalid_request`. With the network unplugged, all of the above still works and `upload_status` stays `queued`.

## Idempotence and Recovery

Every edit is a plain source change and safe to redo; each milestone is its own commit, so a bad one is reverted alone with `git revert <hash>`. Nothing migrates existing data: a device upgrading to this agent starts with no `resources/calibrations/` (upgrades run `reset`), and the index is created empty on first start. A corrupt `index.json` is copied aside and started empty by `StateFile::new_with_default`, and `prune_orphans` then deletes the content files it named; PR B's load restores them from Miru. If CI fails, fix from `gh run view --log-failed` and push again.
