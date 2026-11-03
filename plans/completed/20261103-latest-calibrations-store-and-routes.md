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
| `calibrations::LatestEntry` (fields `metadata: ReportMetadata`, `is_uploaded`) | `LatestCalibration`, `StoredCalibration` | One entry of the index, as `QueueEntry` is one entry of the queue. `Calibration` is the device wire type. |
| `calibrations::ListedEntry` (fields `metadata`, `upload_status`) | `LatestCalibration`, `LatestItem` | A latest entry as `LatestStore::list` returns it, with the derived status; `Calibration` is the device wire type. |
| `calibrations::LatestFilter` (fields `component`, `role`, `aspect`) and `LatestFilter::is_match` | `LatestQuery`, `CalibrationFilter`; `matches` | Domain filter, apart from the wire query below. `is_match` reads as a question (shared rule). |
| `calibrations::LatestCalibrations` (fields `is_confirmed`, `entries`) | `LatestList`, `LatestListing` | Mirrors the wire `LatestCalibrationsResponse`. |
| `calibrations::LatestContent` (fields `format`, `data`) | `CalibrationContent`, `Content` | The content of a latest entry; `InstanceContent` already names the request's `{format, data}` wire type. |
| `CalibrationManager::list_latest`, `CalibrationManager::latest_content` | `latest`, `get_latest`; `content` | Verb first like `enqueue`/`len`; `latest_content` says which content. |
| `UploaderArgs::latest` (a prebuilt `LatestStore`) | `latest_index`/`latest_contents` | The `Worker` owns the store, so `AppState` and the fixtures build it and `CalibrationManager::spawn` only hands it over. |
| `calibrations::latest::DEFAULT_CAPACITY` (1,000 keys) | `UploaderOptions::latest_capacity`, `LatestStore::default` | The cap sits beside the store it bounds, as `events::store::DEFAULT_MAX_RETAINED` does; `UploaderImpl` never reads it. |
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

The store is owned by the calibration uploader's actor (its `Worker`, beside the `UploaderImpl` that holds the queue), not by a new actor: intake, write-through and upload marks then run on one thread with no locks, `enqueue` can roll the queue back when the store write fails, the uploader marks an upload without messaging a second actor, and a listing can ask the queue which reports are still queued. A separate store actor was ruled out because the service would have to undo a queued report across two actors. Reads go through the same actor, which serves commands during uploads and backoffs, so they never wait on Miru.

### Left out

- The initial load from Miru (backend client for the Agent API `GET /calibrations/latest` and content, the sync step, merging queued reports, writing `loaded_at`): agent PR B. The store's API leaves room for a `LatestStore::load(entries, loaded_at)` that writes each content file, then the index with every entry and `loaded_at` in one atomic write, then prunes unreferenced content. Its merge keeps every entry already in the store and fills only the keys the store lacks: after a reset the store holds only this device's write-through reports, which are never older than Miru's, while a report that uploads during the load has left the queue and could otherwise be replaced by an older row Miru returned before the upload. Comparing `reported_at` was ruled out because it comes from the device clock.
- Ordering the list: entries come back in key order (component, then role, then aspect, a missing slug first). The spec documents no order.
- Reconciling the store with the queue at startup: a crash between the queue write and the store write leaves a queued report missing from the store. The app got no 200, so it reports again and the newer report wins. PR B's load also covers a reset.
- A `reset` change or test: `disk::setup::reset` already deletes `resources/`, and `wipes_resources_subtree` in `agent/tests/disk/setup.rs` covers it.
- Repeated query keys (`?component=a&component=b`): the spec uses `style: form, explode: false`, a comma-separated list. A repeated key is rejected with 422.
- Fanning out a pattern: this change has one resource, so there is no exemplar/sibling split.
- Sharing one content file between the upload queue and the store: a report's bytes sit in both `calibrations/reports/` and `resources/calibrations/contents/` until it uploads, so worst-case disk use roughly doubles while reports are queued. The queue deletes its copy on upload and `reset` deletes the store's, so one shared file (or a hard link) would tie the two lifecycles together and make each delete and crash path depend on the other. Calibration files are small in practice (Monarch's are 1 to 17 KB).
- Renaming the `calibrations::Uploader` actor in this PR: [agent#423](https://github.com/mirurobotics/agent/pull/423) landed it first as `CalibrationManager` (`AppState::calibration_mngr`), and this branch merged it in.

### Open questions

- How many keys may the store hold, and what happens past it? Default: `latest::DEFAULT_CAPACITY` 1,000 (the queue's cap, RFC lean); a report for a new key past it is rejected with 507 `calibration_store_full` and nothing is written; a report for an existing key always replaces it.
- Should a full store reuse `calibration_buffer_full`? Default: no, `calibration_store_full`, since retrying after uploads drain never helps.
- Which drops mark an entry `dropped`? Default: every drop (Miru rejected it, out of attempts, or still failing 90 days after `reported_at`), since none of them ever reaches Miru. Settled: listing derives `dropped` for every report the queue let go unsent (see Decision Log).
- What does `GET /calibrations/{report_id}/content` return for a `report_id` that is not a UUID? Default: 404 `resource_not_found`, like any other id that is not a latest entry.

## Progress

- [ ] Milestone 1: model enum, layout, errors and `LatestStore` (source).
- [ ] Milestone 2: uploader write-through, upload status, handle commands, `AppState` wiring (source).
- [ ] Milestone 3: services, request/response conversions, handlers, routes, `ARCHITECTURE.md` (source).
- [ ] Milestone 4: tests.
- [ ] Milestone 5: push and reach `CLEAN` via `$preflight` before the PR leaves draft.
- [x] Source written
- [x] Tests written

## Surprises & Discoveries

- `agent/tests/errors/mod.rs` `test_code_as_str` matches `Code` exhaustively, so adding `Code::CalibrationStoreFull` breaks the test build. The source stage added the match arm only (in the Milestone 3 commit) so the suite compiles; the tests stage adds the `(Code::CalibrationStoreFull, "calibration_store_full")` case and bumps `EXPECTED_CODE_VARIANTS` from 8 to 9, without which that test fails its length check.

## Decision Log

- Keep the double copy of queued content and the `Uploader` name (see Left out), answering the AI design review of c1dfa7c and its thread r4451533096.
- `LatestStore::mark_uploaded` (formerly `set_upload_status`) sets the mark in memory even when the index write fails (logged at warn), as `FileQueue::ack` keeps its in-memory change; the next index write saves it.
- ~~`UploaderImpl::prune_orphans` now calls `LatestStore::prune_orphans`, then the queue's old body, renamed `prune_reports`, so the queue's early returns never skip the store.~~ Superseded: the `Worker` prunes the store, then the queue, each with its own `prune_orphans`.
- `LatestCalibrations` and `LatestContent` derive `Debug` and `PartialEq` so tests can compare them.
- The query helpers in `server/request.rs` are two functions: `split_list` (split on `,`, trim, drop empty items) and `split_slugs` (`split_list`, with `null` as `None`).
- `handlers::get_calibration_content` returns `Response`; its error arm and `handle` share the extracted `error_response`, as planned.
- Fixtures: `test_utils::calibrations::latest_layout(dir)` (a `LatestCalibrationsLayout` at `resources/calibrations` under the temp dir, beside `layout(dir)`) builds the store paths for every fixture. The inline `uploader_impl` fixture keeps the layout as `latest`, opens a latest index file in `uploader_with`, and `uploader_on` takes a `latest_index` argument (the two in-memory-queue call sites pass `None`). The `tests/calibrations/uploader.rs` harness (also keeping `latest`), `test_utils::calibrations::uploader` and the service `create.rs` fixture pass `latest_index: None`.
- `ARCHITECTURE.md`: the latest store is a new paragraph right after the `calibrations/` paragraph, not inside it, which is already long; the `calibrations/` paragraph's "checks only the request's shape" now also names the reserved `null` slug.
- Design review: the content media types are `InstanceFormat::media_type` in `models/instance_format.rs`, a property of the format, not a private function in `server/response.rs`.
- The user chose `CalibrationManager` for the actor; agent#423 renamed it on main, and the merge of main renamed this PR's new uses (the `list_latest` and `get_content` services and their handlers).
- Ready mode starts by moving `LatestStore` from `UploaderImpl` to the `Worker` (AI design review thread r4460354923). The `Worker` answers both reads from it, `UploaderImpl::enqueue`/`finish_attempt` borrow `&mut LatestStore`, `UploaderImpl::new` loses `latest_index`/`latest_contents`, and `prune_reports` goes back to `prune_orphans`.
- ~~`UploaderArgs` keeps `latest_index`/`latest_contents` and `UploaderOptions` keeps `latest_capacity`: they are `CalibrationManager::spawn`'s arguments, which now build the store, so the fixtures and `AppState` need no change. `UploaderImpl` carries `latest_capacity` in its options unused.~~ Superseded below.
- Ready mode also stops storing `upload_status` (AI design review thread r4460405511). A latest entry stores `is_uploaded: bool`, and listing derives the status: `queued` if the queue holds the `report_id`, `uploaded` if marked, otherwise `dropped`. `drop_report` goes, and only a successful upload writes the store, before the dequeue. This settles which drops count as `dropped` (all of them, including reports lost with a corrupt or in-memory queue). PR B's load marks Miru's entries uploaded.
- The derived status: `LatestStore::list(filter, queued)` takes the queued `report_id`s as a `HashSet<Uuid>` from `UploaderImpl::queued_report_ids`, so listing stays one pass over the entries, and returns `ListedEntry { metadata, upload_status }` items, which `server/response.rs` converts. `Uploaded` is checked first. `LatestEntry::is_uploaded` is `#[serde(default)]`, as `ReportMetadata`'s optional fields are.
- Design review: `LatestStore::prune_orphans` and `UploaderImpl::prune_reports` share `calibrations/cleanup.rs` (`prune(dir, kept)` deletes the files whose names `kept` lacks; `delete(file)` logs a failure), replacing their copies and both `delete` helpers. `filesys` had no such helper. Each caller keeps its in-memory guard, and the queue's keep set is the hyphenated entry ids, the names `report_file` writes.
- Design review: `sse::parse_event_filter` reuses `request::split_list` (now `pub(crate)`), which the latest query introduced, instead of repeating the split, trim and drop-empty parsing.
- AI review r4451473461: `LatestIndex::entries` is a `BTreeMap<LatestKey, LatestEntry>`, so a duplicate key can't be represented, and `has_key`/`replace_or_append` are gone. `indexmap` is only a transitive dependency, so a `BTreeMap` (key order; the spec defines none) avoids adding one. JSON object keys must be strings, so `index.json` stays a list of entries (a private `entry_list` serde module) and each is keyed again on load.
- Self-review (AI design review r4461671627): `UploaderArgs` takes a prebuilt `latest: LatestStore` instead of `latest_index`, `latest_contents` and `UploaderOptions::latest_capacity`, which `UploaderImpl` never read. `AppState::init_calibration_mngr` and the fixtures build the store; the cap is `latest::DEFAULT_CAPACITY` (1,000) beside the store, as `events::store::DEFAULT_MAX_RETAINED` is, and `UploaderOptions` is back to its `main` fields.
- Self-review: `LatestEntry::is_uploaded` and `LatestIndex::entries` drop `#[serde(default)]`, since every index write includes both (as `QueueSnapshot::entries` has none). This supersedes the `is_uploaded` default above; an index missing either is corrupt and is copied aside and started empty.
- Design review: `LatestStore::put` always deletes the replaced entry's content; the same-`report_id` guard existed only for fixtures. `test_utils::calibrations::report(slug)` keeps its deterministic `report_id`, which existing tests compare against, and no existing test puts one key twice; the Milestone 4 tests that do give the second report a fresh `Uuid::new_v4()` id.

## Outcomes & Retrospective

Tests written (Milestone 4), not yet validated in CI. They follow the Milestone 4 list, with these changes:

- `InstanceFormat::media_type` is a property of the format (Decision Log), so its six-format table is `media_type_matches_mirus_content_downloads` in `agent/tests/models/instance_format.rs`, and `agent/tests/server/response.rs` checks one format's response (`sends_the_bytes_with_the_content_type_of_the_format`).
- The layout tests are a `latest_calibrations_layout` module (`root_dir`, `index_file`, `contents_dir`), as `calibrations_layout` is.
- The request tests keep 7af92c8's names (`accepts_a_missing_role_and_aspect`, `rejects_a_null_role_or_aspect`); the query tests are `latest_calibrations_query::{splits_comma_separated_values, maps_a_null_role_or_aspect_to_none, an_empty_query_filters_nothing}`.
- The service fixture moved to `agent/tests/services/calibration/helpers.rs` (as `agent/tests/sync/helpers.rs`) so `create.rs`, `latest.rs` and `content.rs` share it, and takes the latest store's capacity. `create_fails_when_the_latest_store_is_full` is `a_full_latest_store_fails_intake`, after its sibling `an_uploader_rejection_fails_intake`. `list_latest_fails_once_the_uploader_stops` and `get_content_of_an_unknown_report_id_is_not_found` cover the services' error arms, which the 100% `services/calibration` gate needs.
- `test_utils::calibrations::uploader` takes the latest capacity, so the route test can fill the store with a capacity of 0; the handler fixture gains `get_with_content_type`, which `get` now calls.
- The actor harness in `agent/tests/calibrations/uploader.rs` opens a latest index file so startup pruning runs, and `commands_after_shutdown_fail` also checks `latest_content`.
- Extra store tests cover the error paths the `calibrations` gate needs: a content write failure, a mark the index can't save (kept in memory, per the Decision Log), a missing or unreadable content file, and an index entry without `is_uploaded` (corrupt, per the Decision Log).
- `agent/tests/server/errors.rs` and `agent/tests/services/errors.rs` don't list `CalibrationsErr` variants, so they are unchanged.

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
- The uploader marks an entry uploaded before it removes the report from the queue. A crash in between re-sends the report on the next boot (Miru records it once by `report_id`) and marks it again; the reverse order would list an uploaded report as `dropped`.

### Milestone 1: model, layout, errors and the store

`agent/src/models/calibration_upload_status.rs` (new): `pub enum CalibrationUploadStatus { #[default] Queued, Uploaded, Dropped }` with InstanceFormat's derives (`Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash`, `serde(rename_all = "snake_case")`) and `impl_status_enum!` in the agent-only form: `default: Queued`, `label: "calibration upload status"`, `log: warn`, `agent_type: agent_server::CalibrationUploadStatus`, mappings `queued`/`uploaded`/`dropped` to `CALIBRATION_UPLOAD_STATUS_*`. Declare it in `agent/src/models/mod.rs` and re-export `CalibrationUploadStatus`.

`agent/src/disk/layout.rs`: add `Layout::latest_calibrations(&self) -> LatestCalibrationsLayout` returning `LatestCalibrationsLayout::new(self.resources().subdir("calibrations"))`, and `LatestCalibrationsLayout { pub root }` after `CalibrationsLayout` with `new`, `index()` (`index.json`) and `contents()` (`contents/`), and a one-line doc comment saying it is the latest store, a cache of Miru that `reset` wipes.

`agent/src/errors/mod.rs`: add `Code::CalibrationStoreFull` → `"calibration_store_full"`.

`agent/src/calibrations/errors.rs`: add `CalibrationStoreFullErr { pub capacity: usize, pub trace }` (507, `Code::CalibrationStoreFull`, params `{"max_keys": capacity}`, doc: nothing was written) and `CalibrationNotFoundErr { pub report_id: String, pub trace }` (404, `Code::ResourceNotFound`, message "no latest calibration has report id {report_id}"). Add variants `StoreFullErr` and `NotFoundErr` to `CalibrationsErr`, `From` impls, and the `impl_error!` list.

`agent/src/calibrations/latest.rs` (new; `pub mod latest;` in `agent/src/calibrations/mod.rs`, re-exporting the types below):

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct LatestEntry { pub metadata: ReportMetadata, pub is_uploaded: bool }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    pub struct LatestIndex {
        /// When the store was last loaded from Miru; `None` until then.
        #[serde(default)] pub loaded_at: Option<DateTime<Utc>>,
        #[serde(with = "entry_list")] pub entries: BTreeMap<LatestKey, LatestEntry>,
    }
    impl Patch<LatestIndex> for LatestIndex { replace, as QueueSnapshot does }
    pub type LatestIndexFile = StateFile<LatestIndex, LatestIndex>;

    /// Empty means any; `None` in `role`/`aspect` matches an entry without one.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LatestFilter { pub component: Vec<String>, pub role: Vec<Option<String>>, pub aspect: Vec<Option<String>> }
    impl LatestFilter { pub fn is_match(&self, metadata: &ReportMetadata) -> bool }

    pub struct LatestCalibrations { pub is_confirmed: bool, pub entries: Vec<ListedEntry> }
    pub struct ListedEntry { pub metadata: ReportMetadata, pub upload_status: CalibrationUploadStatus }
    pub struct LatestContent { pub format: InstanceFormat, pub data: Vec<u8> }

    pub struct LatestStore { index: LatestIndex, index_file: Option<LatestIndexFile>, contents: Dir, capacity: usize }

`LatestStore` methods, each with a short doc comment:

- `new(index_file: Option<LatestIndexFile>, contents: Dir, capacity: usize) -> Self`: the index is read from the file, or empty without one (in memory, like `Queue::new`).
- `verify_capacity(&self, metadata: &ReportMetadata) -> Result<(), CalibrationStoreFullErr>`: Ok when the key already has an entry or there is room; otherwise log a warn and return the error. The key is a `LatestKey` (`component`, `role`, `aspect`), so this is `contains_key`.
- `async put(&mut self, metadata: ReportMetadata, data: &[u8]) -> Result<(), CalibrationsErr>`: `verify_capacity`; write `contents/<report_id>` with `files::write_bytes(.., WriteOptions::OVERWRITE_ATOMIC)`; build the next index, inserting the entry under its key with `is_uploaded: false`; persist it (`index_file.write`, atomic); on a persist error delete the new content file and return the error with the in-memory index unchanged; then set the in-memory index and delete the replaced entry's content file (log a warn on failure).
- `async mark_uploaded(&mut self, report_id: Uuid)`: find the entry with that `report_id`; none (a newer report replaced it, or `reset` wiped the store) is a no-op; otherwise set `is_uploaded` and persist, logging a warn on failure like `FileQueue::ack`.
- `list(&self, filter: &LatestFilter, queued: &HashSet<Uuid>) -> LatestCalibrations`: `is_confirmed: index.loaded_at.is_some()`, entries that `filter.is_match`, in key order, each a `ListedEntry` whose `upload_status` is `Uploaded` if marked, `Queued` if `queued` holds its `report_id`, else `Dropped`.
- `async content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr>`: `CalibrationNotFoundErr` unless an entry has that `report_id`; else `files::read_bytes` of its content file, with the entry's format. A missing content file is `CalibrationNotFoundErr` too.
- `async prune_orphans(&self)`: with an index file only, delete every file in `contents/` not named by an entry's `report_id`, as `UploaderImpl::prune_orphans` does (warn per deletion and on listing failure).

Commit (see Concrete Steps).

### Milestone 2: the uploader writes through and marks status

`agent/src/calibrations/uploader_impl.rs`:

- `latest.rs` gains `pub const DEFAULT_CAPACITY: usize = 1_000;`, the store's key cap, beside `LatestStore` as `events::store::DEFAULT_MAX_RETAINED` is beside `EventStore`. `UploaderOptions` is unchanged.
- `UploaderImpl` does not own the store: the actor's `Worker` does, and lends it to `enqueue` and `finish_attempt` as `latest: &mut LatestStore` (the first argument after `&mut self`). `new` and `prune_orphans` are unchanged.
- `enqueue`: after `verify_size`, call `latest.verify_capacity(&report.metadata)?`; enqueue a clone of the metadata; write the report file as today; then `latest.put(metadata, &bytes)`, and on error `self.remove(id).await` and return it. Update the doc comment: the report is in the latest store when this returns Ok, and on error nothing is queued or stored.
- `finish_attempt`: on `Ok`, `latest.mark_uploaded(entry.job.report_id)` before `self.remove`. Drops do not touch the store: `handle_failure` is unchanged.
- Add `pub(crate) fn queued_report_ids(&self) -> HashSet<Uuid>`, the `report_id` of every queued report, which `Worker::handle` passes to `LatestStore::list`.
- Update the inline test `Fixture` with a `latest_store()` that opens a store under the temp dir, and pass it to each `enqueue`/`finish_attempt` so existing tests compile.

`agent/src/calibrations/uploader.rs`:

- `UploaderArgs` gains `latest: LatestStore` (doc: the store the worker lends the uploader); `spawn` moves it into the `Worker`'s `latest` field, beside `uploader`.
- `Worker::run` awaits `self.latest.prune_orphans()`, then `self.uploader.prune_orphans()`.
- `Command` gains `ListLatest { filter: LatestFilter, respond_to: oneshot::Sender<LatestCalibrations> }` and `LatestContent { report_id: Uuid, respond_to: oneshot::Sender<Result<LatestContent, CalibrationsErr>> }`, which `Worker::handle` answers from `self.latest`.
- `Uploader` gains `pub async fn list_latest(&self, filter: LatestFilter) -> Result<LatestCalibrations, CalibrationsErr>` and `pub async fn latest_content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr>` via `send_command`. Update `enqueue`'s doc: it also writes the latest store, and a full store is the 507 `StoreFullErr`. Update the `Uploader` doc to say it also owns the latest store.

`agent/src/app/state.rs` `init_calibration_uploader`: open `layout.latest_calibrations().index()` with `LatestIndexFile::new_with_default(.., Default::default())`, falling back to `None` with an error log as for the queue; pass `LatestStore::new(latest_index, latest_calibrations().contents(), latest::DEFAULT_CAPACITY)` as `UploaderArgs::latest`. Update the doc comment.

Keep the other `UploaderArgs` construction sites compiling by adding an in-memory `latest: LatestStore::new(None, .., DEFAULT_CAPACITY)` with its contents under each fixture's temp dir: `agent/tests/test_utils/calibrations.rs` (`uploader`), `agent/tests/calibrations/uploader.rs` (`Harness::spawn_on`), `agent/tests/services/calibration/create.rs` (`Fixture::with_options`). Build the paths with `LatestCalibrationsLayout::new(..)` under the fixture's temp dir.

Commit.

### Milestone 3: services, server and docs

`agent/src/services/calibration/latest.rs` (new): `pub async fn list_latest(uploader: &Uploader, filter: LatestFilter) -> Result<LatestCalibrations, ServiceErr>`, doc: answers from the device's store and never calls Miru.

`agent/src/services/calibration/content.rs` (new): `pub async fn get_content(uploader: &Uploader, report_id: &str) -> Result<LatestContent, ServiceErr>`: parse `report_id` as a `Uuid`; a parse failure is `CalibrationsErr::NotFoundErr` (404); else `uploader.latest_content(id)`.

`agent/src/services/calibration/mod.rs`: declare and `pub use` both; update the module doc to "calibration intake and the latest calibrations".

`agent/src/server/request.rs`:

- Copy 7af92c8's null check into `TryFrom<CreateCalibrationRequest> for CreateParams`, with its doc comment ("Checks the request's shape and the reserved `null` slug, which the slug filters use to match a missing role or aspect; Miru judges the rest.") and message `"{field} 'null' is reserved"`.
- Add `#[derive(Debug, Default, Deserialize)] pub struct LatestCalibrationsQuery { pub component: Option<String>, pub role: Option<String>, pub aspect: Option<String> }` (doc: each is a comma-separated list, as the spec's `explode: false` sends it) and `impl From<LatestCalibrationsQuery> for LatestFilter`: split on `,`, trim, drop empty items (as `sse::parse_event_filter` does), and map `null` to `None` for role and aspect.

`agent/src/server/response.rs`:

- `impl From<&ListedEntry> for device_server::Calibration` (object `calibration::Object::Calibration`, `report_id`, `upload_status: (&entry.upload_status).into()`, slugs, `serial_number`, `reported_by`, `reported_at.to_rfc3339()`).
- `impl From<&LatestCalibrations> for device_server::LatestCalibrationsResponse`.
- `impl IntoResponse for LatestContent`: 200, `Content-Type` from `InstanceFormat::media_type` (`models/instance_format.rs`) with core's mapping (see Precedent), body the bytes.

`agent/src/server/handlers.rs`, under CALIBRATIONS:

- `list_latest_calibrations(AxumState(state), query: Result<Query<request::LatestCalibrationsQuery>, QueryRejection>)` through `handle`: a rejection is `InvalidRequestErr` with `rejection.body_text()` (422); else `calibration_svc::list_latest(&state.calibration_mngr, filter)` and `LatestCalibrationsResponse::from(&latest)`.
- `get_calibration_content(AxumState(state), Path(report_id): Path<String>) -> Response`: on Ok `content.into_response()`; on Err the JSON error. Extract the error arm of `handle` into `fn error_response(e: ServerErr, err_msg: &str) -> (StatusCode, Json<Value>)` and use it in both.

`agent/src/server/routes.rs`: under CALIBRATIONS add `GET /{api_version}/calibrations/latest` → `list_latest_calibrations` and `GET /{api_version}/calibrations/{{report_id}}/content` → `get_calibration_content`.

`ARCHITECTURE.md`, `calibrations/` paragraph: add that the uploader also owns the latest store at `resources/calibrations/` (`index.json` with `loaded_at` and one entry per key, `contents/<report_id>`), written through by intake before `POST` returns (queue first, store second, queue rolled back on a store failure), marked uploaded by the uploader before it dequeues and otherwise listed `queued` while the queue holds it, else `dropped`, served by `GET /v0.2/calibrations/latest` and `/{report_id}/content` without calling Miru, capped at 1,000 keys (507 `calibration_store_full`), wiped by `reset` with the rest of `resources/`, and that `null` is a reserved role/aspect slug. In the `services/` list, extend `calibration` to "(calibration intake and the latest calibrations)".

Commit.

### Milestone 4: tests

Follow the repo's test rules (`CLAUDE.md` § Testing): route tests in `agent/tests/server/handlers.rs` check status codes and bodies only; storage behavior is tested at the store, uploader and service layers. Use `#[tokio::test]` and `test_dirs::temp(..)` fixtures as the siblings do. To force a write failure, create a directory at the file path, as `calibration_uploader_degrades_when_queue_path_unwritable` does.

`agent/tests/models/calibration_upload_status.rs` (new, `pub mod` in `agent/tests/models/mod.rs`): `StatusEnumFixture` with `type Backend = ()`, `type Agent = agent_server::CalibrationUploadStatus`, three cases, `status_enum_tests!(CalibrationUploadStatus, agent)`.

`agent/tests/disk/layout.rs`: `latest_calibrations_index` (`resources/calibrations/index.json`) and `latest_calibrations_contents` (`resources/calibrations/contents`).

`agent/tests/errors/mod.rs`: add `(Code::CalibrationStoreFull, "calibration_store_full")` to the code table.

`agent/tests/calibrations/errors.rs`: `a_full_store_is_507_with_its_cap` and `a_missing_latest_calibration_is_404`.

`agent/tests/calibrations/latest.rs` (new, `pub mod latest;` in `agent/tests/calibrations/mod.rs`), against `LatestStore` with an index file under a temp dir:

- `put_adds_an_entry_not_uploaded_and_its_content`
- `put_replaces_the_keys_entry_and_deletes_its_content` (the second report has a fresh `report_id`, as intake mints)
- `role_and_aspect_are_part_of_the_key` (a missing role is its own key)
- `put_rejects_a_new_key_at_capacity_and_writes_nothing`, `put_replaces_an_existing_key_at_capacity`
- `put_fails_and_changes_nothing_when_the_index_cannot_be_written` (also: the new content file is gone)
- `mark_uploaded_marks_the_matching_report`, `mark_uploaded_ignores_a_replaced_report`
- `list_derives_the_upload_status` (uploaded if marked, else queued if `queued` holds the `report_id`, else dropped)
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
- `a_dropped_report_is_not_marked_uploaded` (cover `Terminal`, `OutOfAttempts` and `TooOld`)
- `queued_report_ids_are_the_queued_reports`
- `a_newer_report_for_the_key_is_not_marked` (upload the older report after a newer one replaced it)

`agent/tests/calibrations/uploader.rs`: `startup_prunes_unreferenced_latest_contents`; `latest_reads_are_served_during_an_upload` (hold an upload with the mock sender's `Step`, then `list_latest` and `latest_content` answer).

`agent/tests/app/state.rs`: in `calibration_uploader_spawned`, assert `layout.latest_calibrations().index()` exists; add `calibration_uploader_degrades_when_latest_index_path_unwritable` mirroring the queue test.

`agent/tests/services/calibration/latest.rs` and `content.rs` (new; `pub mod` in `agent/tests/services/calibration/mod.rs`), on `create.rs`'s fixture: `list_latest_includes_a_created_report`, `list_latest_applies_the_filter`; `get_content_returns_the_reported_bytes`, `get_content_of_an_invalid_report_id_is_not_found`. In `create.rs`, `create_fails_when_the_latest_store_is_full`.

`agent/tests/server/request.rs`: `a_null_role_is_rejected`, `a_null_aspect_is_rejected` (from 7af92c8); `latest_query_splits_comma_separated_values`, `latest_query_maps_null_to_none`, `an_empty_latest_query_filters_nothing`.

`agent/tests/server/response.rs`: `listed_entry_converts_to_calibration`, `latest_calibrations_convert_to_the_response`, `content_response_sets_the_content_type_for_each_format` (table over the six formats).

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
