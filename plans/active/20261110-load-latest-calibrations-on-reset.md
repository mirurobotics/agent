# Keep the latest calibrations store across upgrades and load it on reprovision

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | The `Calibration` → `ReportMetadata` conversion; `calibrations::overwrite_latest` into a given store; reprovision wiping `calibrations/` and loading Miru's latest into it; a `ProvisionErr` variant; mocks, tests and `ARCHITECTURE.md`. |
| `openapi/` (mirurobotics/openapi) | read-only | Source of the vendored Agent API spec `api/specs/backend/v05.yaml`, which already has `GET /calibrations/latest` and `GET /calibrations/{calibration_id}/content`. Nothing is regenerated and `libs/` is not touched. |

This plan lives in `agent/plans/` because every change is in the agent repo. It is part of ENG-1726 (RFC: Latest Calibrations on the Device). Branch `claude/project-thread-212l69`, based on `main` at `de3967d`. All paths are relative to the agent repo root, `/home/user/agent`. The file name keeps its first title so links to it still work.

## Purpose / Big Picture

The device keeps the newest calibration report for each component, role and aspect in its latest calibrations store. On `main` the store is under `resources/latest_calibrations/`, and `disk::setup::reset` wipes `resources/` on every upgrade, provision and reprovision, so after an upgrade a robot reads no calibrations until it reports again. The Device API planned an `is_confirmed` flag to say so.

After this change the store lives at `calibrations/latest/`, beside the upload queue, which `reset` already keeps, so an upgrade leaves it as it is and pulls nothing. Provision assumes the device has no data, so it neither loads nor clears the store. Reprovision is the only time the agent pulls calibrations from Miru: once Miru accepts the new public key and `bootstrap` installs it, it issues a device token from the new keys, deletes `calibrations/` (dropping the upload queue) and loads Miru's latest calibration for every key into the empty store. If the load fails, reprovision fails and is rerun.

Observable: upgrade an agent whose device has calibrations in its store; `/var/lib/miru/calibrations/latest/index.json` is unchanged and the agent makes no calibrations request. Run `miru-agent reprovision` on an online device with calibrations in Miru; the log shows `calibrations: loaded N latest calibrations from Miru`, `calibrations/` holds only `latest/`, and `calibrations/latest/index.json` holds one entry per key with `contents/<report_id>` for each. If Miru answers the list with an error, reprovision fails with `failed to load the latest calibrations: ...` and `calibrations/` holds no queue and no store entries.

## Decisions

### Design

- **The store survives upgrades; only reprovision pulls it from Miru (the user's design).** Reports are created only on the device, so the store on the device is already the truth after an upgrade and pulling it would only repeat what the device holds. Only reprovision changes which device's data the store should hold, and reprovision already needs the network to register the new key, so it is the one place that pulls. Provision registers a new device, which has no calibrations in Miru, so it pulls nothing and starts empty.
- **The store moves to `calibrations/latest/`.** `disk::CalibrationsLayout::latest()` returns `LatestCalsLayout::new(self.root.subdir("latest"))`, so callers write `layout.calibrations().latest()`. `reset` keeps `calibrations/` (the upload queue lives there), so it no longer touches the store, and `reset`, `bootstrap` and `app::upgrade::reconcile_impl` stay exactly as on `main`, writing the agent-version marker last. Consequence: the store's on-disk format (`index.json`, `{"entries": [...]}` of `ReportMetadata` since #451, and `contents/<report_id>`) must now stay readable by newer agents, as `calibrations/queue.json`'s already must.
- **Provision leaves the store alone.** Provisioning assumes a new device has no data, so `provisioning::provision` is unchanged from `main`: it neither loads nor clears the store.
- **Reprovision wipes `calibrations/`, then loads in place.** Order in `provisioning::reprovision`: generate the temp keys → `reprovision_with_backend` (Miru accepts the new public key) → `bootstrap` (installs the keys) → `overwrite_calibrations`: `dirs::delete(calibrations/)`, issue a device token from the installed keys (`authn::issue_token`), and run `calibrations::overwrite_latest` into `calibrations/latest/`. The old upload queue, its reports and the old store go first ([review](https://github.com/mirurobotics/agent/pull/450#discussion_r4521677972)), so a failure after `bootstrap` never leaves the previous device's calibrations ([AI review](https://github.com/mirurobotics/agent/pull/450#discussion_r4521695982)); it leaves an empty or partial store until reprovision is rerun. A load failure is `ProvisionErr::CalibrationsErr` ("failed to load the latest calibrations: ..."), a token failure stays `ProvisionErr::AuthnErr`. No staging dir.
- **Reprovision fails if it can't load.** It already needs the network. The load runs after `bootstrap`, so a failed load leaves a reprovisioned device that can reach Miru, with its old store, until reprovision is rerun (AI design review on #450). Loading before `bootstrap` would leave the device on old keys Miru no longer accepts.
- **The loader spawns the store's actor for the length of the load.** `calibrations::overwrite_latest` calls `LatestStore::spawn` on the target layout, writes each report with the public `LatestStore::put`, then drops the handle and awaits the worker, so no task writes the store after the load. The loader never names `latest_impl`, as `AGENTS.md` requires. Alternative rejected: driving `LatestStoreImpl` directly, a second write path around the worker.
- **`overwrite_latest` takes the target `LatestCalsLayout`,** not `disk::Layout`, so tests can load into any dir.
- **One key at a time.** The loader keeps only metadata in memory and downloads and writes one content at a time, so memory stays bounded to one content of up to 1 MiB.
- **Miru only, no merge.** Reprovision wipes `calibrations/` and writes Miru's latest; the loader never reads the upload queue ([review](https://github.com/mirurobotics/agent/pull/450#discussion_r4521677972)).
- **Nothing is skipped** ([review](https://github.com/mirurobotics/agent/pull/450#discussion_r4521740894)). Every error fails the load: a failed list, a failed download (after `http::with_retry`), a store write error, or more keys than `latest::DEFAULT_CAPACITY` (the store's `CalibrationStoreFullErr`). A loaded store holds every calibration Miru has.

- **RFC deviation.** The RFC planned a sync step that loads the store after every reset and an `is_confirmed` flag until it has. Here the store is never wiped by an upgrade, and only reprovision loads, synchronously, so the store is never served half-loaded and `is_confirmed` is not needed.

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `calibrations/latest/` (`layout.calibrations().latest()`) | `calibrations/latest_calibrations/`, a top-level `latest_calibrations/` | Beside `queue.json` and `reports/` in the dir `reset` keeps; the parent already says "calibrations". |
| `calibrations::overwrite_latest(http_client, store, capacity, token)` (module `calibrations::overwrite`) | `calibrations::latest::load`, `restore_latest` | "Load" is the RFC's and the spec's word; a module of its own keeps HTTP out of `latest.rs`, as `sender.rs` holds the uploader's HTTP. |
| `capacity` parameter of `overwrite_latest` | always `DEFAULT_CAPACITY` | Matches `LatestStore::spawn(buffer_size, layout, capacity)`; tests need a small one. |
| `provisioning::reprovision::overwrite_calibrations` (private) | inline in `reprovision` | Keeps `reprovision` under 50 lines. Reprovision-only, so it lives in `reprovision.rs`, not `shared.rs`. |
| `ProvisionErr::CalibrationsErr` | `LoadCalibrationsErr` | Named after the wrapped type, as `AuthnErr(AuthnErr)` and `DiskErr(DiskErr)` are. |
| `http::calibrations::list_latest`, `ListLatestParams` | `list`, `list_latest_page` | The endpoint is `/calibrations/latest`; shape of `http::deployments::list`/`ListParams`. |
| `http::calibrations::list_all_latest`, `ListAllLatestParams` | `list_latest_all` | Mirrors `http::deployments::list_all`/`ListAllParams`. |
| `http::calibrations::get_content`, `GetContentParams` | `download_content` | Same as `http::config_instances::get_content`/`GetContentParams`. |
| `http::calibrations::CONTENT_TIMEOUT` (renamed from `CREATE_TIMEOUT`) | two equal constants | Both requests carry up to 1 MiB of content for the same reason. |
| `impl From<backend_api::models::Calibration> for ReportMetadata` | `ReportMetadata::from_backend` | Backend-to-agent conversions are `From` impls beside the agent type (`models/config_instance.rs`). |
| `ReportMetadata::key`, `report::Key<'_>`, `report::read_data`, `report::report_file` (all `pub(crate)`) | keep `latest_impl::key_parts`, `uploader_impl::read` | One name per concept, shared by `latest_impl`, `uploader_impl` and the loader. |
| Test mock: `Call::ListLatestCalibrations`, `Call::GetCalibrationContent`; `MockClient::{set_list_all_latest_calibrations, set_list_latest_calibrations_page, set_get_calibration_content}` | `set_latest_calibrations` | The operationIds; mirrors `set_list_all_deployments`, `set_list_deployments_page`, `set_get_config_instance_content`. |
| Test fixture `test_utils::calibrations::miru_calibration(slug)` | `backend_calibration(slug)` | Says it is Miru's record, beside `report(slug)`. |

### Precedent

| New file | Mirrors |
|----------|---------|
| `agent/src/calibrations/overwrite.rs` | `agent/src/calibrations/sender.rs` (the module's HTTP through `http::calibrations`, with a backend-model conversion) and the fetch in `agent/src/sync/deployments.rs` (`http::with_retry` around `list_all`, then each item's content). |
| `agent/tests/calibrations/overwrite.rs` | `agent/tests/calibrations/latest.rs` (a `Fixture` over a temp dir, read back through `test_utils::calibrations::latest_store`) with `MockClient` as in `agent/tests/calibrations/sender.rs`. Reprovision tests seed a queued report as `preloaded` does in `agent/tests/calibrations/uploader.rs`. |

`reprovision` deletes `calibrations/` with `filesys::dirs::delete`, as `reset` deletes `resources/`. No other new files.

### Left out

- Removing `is_confirmed`: a separate openapi spec change (the openapi thread), then agent#421, which owns the Device API list and content routes. Nothing here touches `api/specs/` or `libs/`.
- Rejected design: loading in the agent's startup reconcile, with `reset` deleting the agent-version marker first so every reset reloads before the agent serves. It pulled on every upgrade though the device already holds its reports, blocked upgrades on Miru, and changed `reset`'s marker contract.
- Rejected design: the RFC's sync step that loads after a reset, with `loaded_at`/`is_loaded` and an "fill only absent keys" store API.
- Migrating an existing `resources/latest_calibrations/`: the store has shipped only in the `v0.12.4` betas. Upgrading from one, `reset` deletes the old path with `resources/`, and the device starts with an empty store until it reports again or is reprovisioned.
- Keeping reports still in the upload queue across a reprovision: they are dropped with `calibrations/`, by the author's call.
- Writing the index once per load instead of once per key, and downloading contents concurrently: reprovision is rare; a follow-up if it proves slow.
- `expand=total_count`: paging stops on `has_more`, as `deployments::list_all` does.

### Open questions

- What if a load outlasts the device token? Default: one token per reprovision; if it expires mid-load the reprovision fails and the operator reruns it. A per-page token refresh is the follow-up if seen.
- What if Miru's `reported_at` does not parse? Default: log an error and use the Unix epoch, as `models::ConfigInstance` does for `created_at`.
- Should a failed load after Miru accepted the new key fall back to an empty store instead of failing? Default: no, the user's design fails reprovision so the operator can rerun it with the network up.

## Progress

- [x] Milestone 1 source: HTTP calls, `Calibration` conversion, `ReportMetadata::key`, `report::read_data`, `report::report_file`.
- [x] Milestone 2 source: `calibrations::overwrite_latest` into a given store.
- [x] Milestone 3 source: store at `calibrations/latest/`; `reset`, `bootstrap` and the upgrade reconcile as on `main`; provision as on `main`; reprovision wipes `calibrations/` and loads in place; `ProvisionErr::CalibrationsErr`.
- [x] Milestone 4: `ARCHITECTURE.md`.
- [x] Split (user's request): the store move ([agent#451](https://github.com/mirurobotics/agent/pull/451)), the shared report helpers ([agent#452](https://github.com/mirurobotics/agent/pull/452)) and the HTTP client ([agent#453](https://github.com/mirurobotics/agent/pull/453)) are on `main` with their tests. This PR keeps the `Calibration` conversion, the loader and the reprovision load; skip the test steps below for the parts already on `main`.
- [ ] Tests for the `Calibration` conversion, the loader and reprovision (tests stage).

## Surprises & Discoveries

- `filesys::dirs::move_to` with `Overwrite::Allow` already implements the swap (rename aside, rename in, delete trash, roll back on failure), so reprovision needs no swap helper.
- With nothing in Miru, the store worker writes nothing and never creates its dir; a missing index reads as an empty store.

## Decision Log

- Provision deleted all of `calibrations/` (AI design review on #450, comment 4521330847), until review moved it out (entry below).
- The loader reads `queue.json` read-only with `files::read_json` instead of opening it with `QueueSnapshotFile::new_with_default`, which creates a missing file and replaces a corrupt one: a reprovision must not rewrite a file the running agent's uploader owns (AI design review on #450, comment 4521321246).
- Reprovision loads after `bootstrap`, issuing the token from the installed keys, not before it from the temp keys. A failed load then costs the device only its store refresh, not its connection to Miru (AI design review on #450, comment 4521307928).
- Source stage (`$implement stage: source`) wrote only source, plus the test edits a signature change forced (`tests/http/calibrations.rs` uses `CONTENT_TIMEOUT`).
- `put_mirus_latest` takes `capacity` and computes `room` itself, so its warning can name the capacity. `newest_per_key` returns entries in `HashMap` order, so tests must not depend on the store's order for queued keys.
- Design review: "complete" means everything Miru can serve (non-network download failures and an unopenable queue file are skipped); queued reports go in first and Miru fills every other key with the room left; `ReportMetadata::key()`, `report::read_data` and `report::report_file` replace the private helpers. `latest_impl`'s inline test became `key_is_the_component_role_and_aspect_only`; the tests stage moves it to `report.rs`.
- Reshape (user's design): the earlier designs loaded the store after every reset, first in provision and reprovision, then only in the startup reconcile with `reset` deleting the agent-version marker first. The user ruled that the agent pulls calibrations from Miru only on reprovision: reports are created only on the device, so an upgrade has nothing to pull if the store survives it, and provision starts a device with no data. So the store moved out of `resources/` to `calibrations/latest/`; `reset`, `bootstrap`, `disk::agent_version` and `app::upgrade` (and their tests) went back to `main`, dropping `disk::agent_version::delete` and `UpgradeErr::CalibrationsErr`; `overwrite_latest` now takes the queue layout and a target store layout; provision deletes the store after `bootstrap`; reprovision loads into `tmp/latest_calibrations/` with a token from the new keys and swaps it in after `bootstrap`, failing with the new `ProvisionErr::CalibrationsErr`.
- 2026-11-10 (review, [agent#450](https://github.com/mirurobotics/agent/pull/450#discussion_r4521361846)): the store's layout comes from `CalibrationsLayout::latest()`, replacing `Layout::latest_cals()`, so callers write `layout.calibrations().latest()` and the path reads as part of the calibrations directory.
- 2026-11-10 (review, [agent#450](https://github.com/mirurobotics/agent/pull/450#discussion_r4521374689)): provision no longer deletes `calibrations/`. Provisioning assumes a new device has no data, so clearing leftovers is not provision's job; this supersedes the entry above that had provision delete the queue and the store.
- 2026-11-10 (user): split #450. #451, #452 and #453 landed the store move, the shared report helpers and the HTTP client on `main`; #450 was rebuilt on `main` with only the `Calibration` conversion, the loader and the reprovision load.
- 2026-11-10 (review, [agent#450](https://github.com/mirurobotics/agent/pull/450#discussion_r4521677972)): reprovision wipes `calibrations/` and writes Miru's latest, with no queue merge. The loader no longer reads the queue, and the staged dir replaces all of `calibrations/`, dropping reports not yet uploaded. This supersedes the earlier entries on queued reports winning and the read-only queue read.
- 2026-11-10 ([AI review](https://github.com/mirurobotics/agent/pull/450#discussion_r4521695982), pending the author): reprovision deletes `calibrations/` right after `bootstrap` and loads into it in place, with no staging dir. A load or token failure after `bootstrap` must not leave the previous device's calibrations or queue on a device that is already the new one; the cost is an empty or partial store until reprovision is rerun.
- 2026-11-10 (review, [agent#450](https://github.com/mirurobotics/agent/pull/450#discussion_r4521740894)): nothing is skipped. A download that fails for any reason, or more keys than the store holds, fails the load; this supersedes skipping an unservable calibration and Miru's keys past capacity.
- 2026-11-10 (review, [agent#450](https://github.com/mirurobotics/agent/pull/450#discussion_r4521790285)): `calibrations::load_latest` (module `load`) is renamed `calibrations::overwrite_latest` (module `overwrite`), and reprovision's helper `overwrite_calibrations`, so the names say existing entries are overwritten.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent is a Rust binary (`agent/src/`) that keeps a robot's configuration in sync with Miru, the backend. Terms used below:

- **Report**: a calibration an on-device app posts to the Device API. `agent/src/calibrations/report.rs` defines `Report { metadata: ReportMetadata, data: String }`; `ReportMetadata` holds `report_id: Uuid`, `component: String`, `format: InstanceFormat`, `role`, `aspect`, `serial_number`, `reported_by`, `deployment_id` (all `Option<String>`) and `reported_at: DateTime<Utc>`. A report's **key** is its component, role and aspect.
- **Actor**: a task that owns state and serves commands from a channel, with a cloneable handle. Its single-threaded logic is in a private `*_impl.rs` module that callers never name (rule in `AGENTS.md`).
- **Latest store**: `calibrations::LatestStore` (`latest.rs`) over the private `LatestStoreImpl` (`latest_impl.rs`), in a `disk::layout::LatestCalsLayout { index(), contents() }`, now `calibrations/latest/` (`layout.calibrations().latest()`): `index.json`, `{"entries": [...]}` of `ReportMetadata` since #451, and `contents/<report_id>`. `LatestStore::spawn(buffer_size, layout, capacity) -> (LatestStore, JoinHandle<()>)`; `put(metadata, &data)` (content, then index; a new key past `capacity` is `CalibrationsErr::StoreFullErr`); `list(&Filter)`; `content(report_id)`. Every call reads the index from disk; a missing index reads as empty. The worker exits when every handle is dropped. `latest::DEFAULT_CAPACITY` is 1,000.
- **Upload queue**: `calibrations/queue.json` (`CalibrationsLayout::queue()`), a `QueueSnapshot { entries: Vec<QueueEntry<ReportMetadata>> }` opened as a `calibrations::QueueSnapshotFile`, and each entry's data in `calibrations/reports/<entry id hyphenated>` (`report::report_file`). `reset` keeps `calibrations/`.
- **Reset**: `disk::setup::reset(layout, device, settings, agent_version)` writes `device.json`, `settings.json` and a blank `auth/token.json`, deletes `resources/`, recreates `events/` and last writes the agent-version marker. `disk::setup::bootstrap(.., private_key_file, public_key_file, version)` moves the new keys into `auth/` and calls `reset`. Callers: `app::upgrade::reconcile_impl` (agent start, when the marker differs from the running version), `provisioning::provision` and `provisioning::reprovision` (one-shot CLI commands). Unchanged by this plan.
- **Provisioning**: `provision` and `reprovision` (`agent/src/provisioning/`) generate a key pair in `layout.temp_dir()` (`tmp/`), register the public key with Miru using a provisioning token, call `bootstrap`, and always delete the temp key files (`shared::cleanup_temp_files`; `tmp/` itself is installer-owned on Windows). Errors are `ProvisionErr` (`provisioning/errors.rs`, `crate::impl_error!`, manual `From` impls). `authn::issue_token(http_client, private_key_file, public_key_file) -> Result<Token, AuthnErr>` mints a JWT from the key files and exchanges it for a device token.
- **Filesystem**: `filesys::dirs::{create, delete, move_to}`; `delete` accepts a missing dir; `move_to(src, dest, Overwrite::Allow)` renames `dest` aside, renames `src` into place, deletes the trash and rolls back on failure.
- **HTTP**: every backend call goes through `http::ClientI`. `http::deployments::{list, list_all}` page with `query::Page { limit: MAX_PAGE_LIMIT, offset }` until `has_more` is false. `http::with_retry` retries network errors up to three times.
- **Agent API** (generated models in `libs/backend-api/src/models/`): `GET /calibrations/latest` returns `CalibrationList { has_more, data: Vec<Calibration>, .. }`, the latest calibration per key for the calling device, paged, `expand=reported_by`, no content. `GET /calibrations/{calibration_id}/content` returns the raw content.
- **Tests**: integration tests in `agent/tests/` mirror `agent/src/`; unit tests are inline `#[cfg(test)] mod tests` reaching fixtures through `crate::test_utils`. `MockClient` (`agent/tests/mocks/http_client.rs`) panics on an unknown route; its default `issue_device_token_fn` returns an unparseable `expires_at`, so tests that issue a token set it (see `make_mock_client` in `agent/tests/app/upgrade.rs`). Provisioning tests use `Env`, `mock_ok_reprovision`, `StorageSnapshot` and `assert_temp_dir_cleaned` from `agent/tests/provisioning/shared.rs`.
- **CI-only validation**: never run cargo build, test, clippy, fmt or the repo's scripts locally. CI's `ci.yml` runs `lint` (import groups, production functions at most 50 body lines, field asserts, fmt, clippy `-D warnings`, machete), `test` (suite plus `scripts/covgate.sh`) and `windows-check`.

## Plan of Work

### Milestone 1: HTTP calls, the conversion and the shared helpers (source done)

`agent/src/http/calibrations.rs`: `CONTENT_TIMEOUT` (renamed from `CREATE_TIMEOUT`), `list_latest`, `list_all_latest`, `get_content` with `ListLatestParams`, `ListAllLatestParams`, `GetContentParams`, mirroring `http::deployments` and `http::config_instances::get_content`. `agent/src/calibrations/report.rs`: `impl From<backend_api::models::Calibration> for ReportMetadata` (an unparseable `reported_at` logs and is the epoch), `Key<'_>`, `ReportMetadata::key`, `report_file`, `read_data`. `latest_impl` and `uploader_impl` use them.

Tests (tests stage):

- `agent/tests/mocks/http_client.rs`: `Call::ListLatestCalibrations`, `Call::GetCalibrationContent`; `list_latest_calibrations_fn` (page level, default empty) and `get_calibration_content_fn` (`Fn(&str)`, default `Ok("{}")`); setters `set_list_all_latest_calibrations`, `set_list_latest_calibrations_page`, `set_get_calibration_content`; routes `GET /calibrations/latest` and `GET /calibrations/<id>/content`.
- `agent/tests/test_utils/calibrations.rs`: `miru_calibration(slug)` (`id: cal_<slug>`, `report_id: report_id(slug)`, `component_slug: slug`, JSON format, `reported_at: received_at().to_rfc3339()`); `latest_cals_layout(dir)` returns the store under `calibrations/latest`, matching `layout.calibrations().latest()`.
- `agent/tests/http/calibrations.rs`: `mod list_latest` (`success` with `limit`, `offset`, `expand=reported_by` and the token; `error_propagates`), `mod list_all_latest` (`single_page`, `multi_page_pagination`, `empty_result`, `error_propagates`), `mod get_content` (`success`, `error_propagates`, `timeout_is_network_error`).
- `agent/tests/calibrations/report.rs`: `mod from_backend`: `maps_every_field`, `reported_by_is_none_when_not_expanded_or_null`, `an_unparseable_reported_at_is_the_epoch`.
- Inline in `report.rs`: move `key_is_the_component_role_and_aspect_only` from `latest_impl.rs`; `read_data_returns_the_file_s_data`, `read_data_is_an_invalid_report_when_the_file_is_missing`, `read_data_is_an_invalid_report_when_the_file_is_not_utf8` (terminal), `read_data_returns_another_read_error`.

### Milestone 2: `calibrations::overwrite_latest` into a given store (source done)

`agent/src/calibrations/overwrite.rs`, re-exported as `calibrations::overwrite_latest`:

    pub async fn overwrite_latest<HTTPClientT: http::ClientI>(
        http_client: &HTTPClientT,
        store: LatestCalsLayout,
        capacity: usize,
        token: &str,
    ) -> Result<(), CalibrationsErr>

It spawns `LatestStore` on `store`, puts Miru's latest for each key (`put_mirus_latest`, any failure failing the call), drops the handle, awaits the worker and logs `calibrations: wrote {put} latest calibrations from Miru`.

Tests (tests stage):

- `agent/tests/calibrations/overwrite.rs` (add `pub mod overwrite;` to `agent/tests/calibrations/mod.rs`): a `Fixture` with a temp dir, a target store dir (for example `<dir>/target`) and `MockClient`; `overwrite(capacity)` calls `overwrite_latest(&client, LatestCalsLayout::new(target), capacity, "tok")` and reads back with `LatestStore::spawn` on the target. Cases: `writes_mirus_latest_calibrations_into_the_given_store`, `writes_nothing_when_miru_is_empty`, `more_keys_than_capacity_fail_the_call`, `a_failed_download_fails_the_call`, `a_failed_list_fails_the_call`, `a_network_error_on_a_download_fails_the_call`.

### Milestone 3: the store survives reset; provision empties it; reprovision loads it (source done)

- `agent/src/disk/layout.rs`: `Layout::latest_cals()` is replaced by `CalibrationsLayout::latest()`, returning `LatestCalsLayout::new(self.root.subdir("latest"))`; `LatestCalsLayout`'s doc says the store survives upgrades and its format must stay readable by newer agents.
- `agent/src/disk/setup.rs`, `agent/src/disk/agent_version.rs`, `agent/src/app/upgrade.rs`, `agent/src/app/errors.rs`: as on `main`.
- `agent/src/provisioning/errors.rs`: `ProvisionErr::CalibrationsErr(calibrations::CalibrationsErr)` with `#[error("failed to load the latest calibrations: {0}")]`, a `From` impl and an `impl_error!` entry.
- `agent/src/provisioning/provision.rs`: unchanged from `main`.
- `agent/src/provisioning/reprovision.rs`: after `reprovision_with_backend`, `bootstrap`; then `overwrite_calibrations(http_client, layout)`: `dirs::delete(&layout.calibrations().root)`, issue a token with `authn::issue_token` from `layout.auth()`'s keys, `calibrations::overwrite_latest(http_client, layout.calibrations().latest(), DEFAULT_CAPACITY, &token)`.

Tests (tests stage):

- `agent/tests/disk/layout.rs` `mod latest_cals_layout`: `root_dir`, `index_file` and `contents_dir` expect `calibrations/latest`, `calibrations/latest/index.json` and `calibrations/latest/contents`.
- `agent/tests/disk/setup.rs` `mod reset`: new `keeps_the_latest_calibrations_store` (seed `layout.calibrations().latest().index()`; it is unchanged after `reset`).
- `agent/tests/app/upgrade.rs` `mod reconcile_impl`: new `leaves_the_latest_calibrations_store_untouched` (seed a store entry; after `reconcile_impl` it is still listed and the mock saw no `ListLatestCalibrations` call).
- Inline `from_conversions` in `agent/src/provisioning/errors.rs`: `from_calibrations_err`.
- `agent/tests/provisioning/shared.rs`: `mock_ok_reprovision` sets a parseable `issue_device_token_fn` (as `make_mock_client` in `tests/app/upgrade.rs`) and an empty Miru list, so existing reprovision tests get through the load.
- `agent/tests/provisioning/provision.rs`: the mock saw no `ListLatestCalibrations` call.
- `agent/tests/provisioning/reprovision.rs`: `replaces_the_calibrations_with_mirus_latest` (Miru serves `miru_calibration("arm")`; the device has a queued `leg` report and a store entry for another key; after `reprovision` the store at `layout.calibrations().latest()` lists only arm, the queue file and reports dir are gone, the token request used the new keys, and `assert_temp_dir_cleaned`); `a_failed_load_wipes_the_calibrations_and_returns_calibrations_err` (seed a provisioned device, a queued report and a store entry; the list fails with a non-network error; `Err(ProvisionErr::CalibrationsErr(_))`, the queue and store entry are gone, and the device is reprovisioned); `a_failed_token_wipes_the_calibrations` (unparseable `expires_at`; `Err(ProvisionErr::AuthnErr(_))`, the queue and store entry are gone).

### Milestone 4: `ARCHITECTURE.md` (done)

The store paragraph names `calibrations/latest/index.json`, says the store sits outside `resources/` so `reset` keeps it and upgrades leave it as is, that its format must stay readable by newer agents, that provision assumes a new device has no data and leaves it alone, that reprovision deletes `calibrations/` after `bootstrap`, dropping the upload queue, and loads Miru's latest into the empty store with a token from the new keys, that any load failure fails reprovision with `ProvisionErr::CalibrationsErr` leaving an empty or partial store until rerun, with nothing skipped, and that nothing else loads it.

## Concrete Steps

All commands run from `/home/user/agent`. No build, lint or test command runs locally. Each commit message ends with the session's attribution trailers. The source commits are on the branch; the tests stage edits the test files named in each milestone and commits them as `test(...)` commits, then `$preflight` pushes and watches CI.

## Validation and Acceptance

Validation runs in CI only. `$preflight` pushes the branch and watches `ci.yml`; the PR leaves draft only when CI is green on the pushed head: `lint`, `test` (the suite plus `scripts/covgate.sh`, keeping `calibrations`, `http`, `app`, `provisioning` and `disk` at or above their gates) and `windows-check` (portable temp-dir fixtures; the swap's renames work on Windows).

Acceptance, as behavior the tests pin:

- The store is at `calibrations/latest/` and `reset` and the upgrade reconcile leave it untouched with no calibrations request (`latest_cals_layout::*`, `keeps_the_latest_calibrations_store`, `leaves_the_latest_calibrations_store_untouched`).
- Provision makes no calibrations request.
- Reprovision replaces `calibrations/` with Miru's latest, queue included; a failed load returns `ProvisionErr::CalibrationsErr` with the old calibrations already gone (`replaces_the_calibrations_with_mirus_latest`, `a_failed_load_wipes_the_calibrations_and_returns_calibrations_err`).
- `overwrite_latest` writes only the given store, and a failed list, a failed download or too many keys fails (`agent/tests/calibrations/overwrite.rs`).

## Idempotence and Recovery

Source edits are under git. On the device, provision and reprovision are safe to rerun: a reprovision that fails after `bootstrap` leaves the new keys and an empty or partial store, deletes its temp keys, and can be rerun once the network is up; the rerun deletes `calibrations/` again and reloads it. If CI fails, fix from `gh run view --log-failed` and push again; never bypass hooks.
