# Best-effort boot-time device system-metadata sync (local-cache)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Relocate the `SystemMetadata` type, add a JSON disk cache for it, and add a best-effort startup sync that PATCHes drift-prone fields to the backend. |
| `libs/backend-api/` | read-only | Generated client types (`Os`, `Arch`, `UpdateDeviceFromAgentRequest`, `Device`). Never hand-edited. |

This plan lives in `agent/plans/backlog/` because all code changes are in the agent repository.

This branch (`feat/agent-metadata-boot-sync`) is **stacked on `feat/agent-report-system-metadata`** (PR #259), which is unmerged. Base the PR against `feat/agent-report-system-metadata`, **not** `main`: `main` lacks `system_metadata()` / `SystemMetadata` and the `UpdateDeviceFromAgentRequest` metadata fields this plan depends on. When #259 merges, rebase this branch onto `main`.

## Purpose / Big Picture

The agent already reports host system metadata (os, arch, hostname, os_version, kernel_version) to the backend **only** at provision and reprovision time (PR #259). Between reprovisions the drift-prone fields — `hostname`, `os_version`, `kernel_version` — can go stale (a hostname change, an OS upgrade, a kernel patch) while `os`/`arch` are compile-time constants and effectively static.

After this change, every time the long-running agent daemon starts it will, **best-effort**, compare the live host metadata against a small local cache file. If they differ (or the cache is absent), it issues a device token, sends one `PATCH /devices/{id}` with just the five metadata fields, and — only on success — updates the cache so the next boot makes no network call. This keeps the backend's view of each device fresh without a per-boot write.

Observable behavior: on a device whose hostname changed since last boot, restarting `miru-agent` results in exactly one `PATCH /devices/{id}` carrying the new hostname, and the on-disk cache file (`system_metadata.json` under the data root) then equals the live metadata. On a device whose metadata is unchanged, restarting makes **zero** metadata network calls. This is observability data, never correctness-critical: any failure (token issue, network, disk) is logged at `warn`/`debug` and **never blocks or fails agent startup**.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) M1 — Relocate `SystemMetadata` to `crate::models`, widen derives, add `to_update_request`, repoint provisioning callers.
- [ ] M2 — Add `Layout::system_metadata()` + `disk::system_metadata` cache module and its round-trip tests.
- [ ] M3 — Add `app::metadata_sync` (tested core + best-effort boot wrapper), wire into `main.rs::run_agent`, add sync-decision tests.
- [ ] M4 — Validation: preflight reports `CLEAN` and CI is green on the pushed branch head.

Split partially completed work into "done" and "remaining" as needed. Use timestamps when steps complete.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Relocate `SystemMetadata` + its gatherer/mapping helpers from `agent/src/provisioning/shared.rs` to a new leaf module `agent/src/models/system_metadata.rs`, re-exported as `crate::models::SystemMetadata` and `crate::models::system_metadata`, rather than merely widening the existing `pub(super)` visibility.
  Rationale: The persistence layer (`disk::system_metadata`) must (de)serialize the type, and every existing `disk` submodule imports its domain types from `crate::models` (e.g. `disk::device` → `models::Device`), never from `provisioning`. A `disk → provisioning` dependency would be a layering inversion. `models` already depends on `backend_api` and `telemetry` is a leaf (no `crate::` imports), so no dependency cycle is introduced. The custom import linter (`.lint-imports.toml`) enforces only import-group ordering, not module boundaries, so it neither blocks nor prefers either option — layering cleanliness is the deciding factor.
  Date/Author: 2026-09-21 / Claude (authoring)

- Decision: Widen `SystemMetadata` derives from `#[derive(Debug, Default, PartialEq)]` to `#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]`.
  Rationale: `Eq` + `PartialEq` for the live-vs-cache comparison; `Serialize`/`Deserialize` for the JSON cache; `Clone` so the cache can be written after its fields are copied into the request. `backend_api::models::Os` and `Arch` already derive `Clone, Copy, Eq, PartialEq, Serialize, Deserialize`, and all other fields are `Option<String>`, so every derive is satisfiable.
  Date/Author: 2026-09-21 / Claude (authoring)

- Decision: The testable core is `sync_system_metadata(http_client, layout, token)`; a thin best-effort `run_on_boot(layout, backend_host)` wrapper (build client, issue token via `authn::issue_token`, call the core, log-and-swallow) is invoked from `main.rs::run_agent`, mirroring the existing `reconcile_agent_version` startup step.
  Rationale: Passing `token` into the core keeps unit tests free of RSA/JWT machinery (tests seed `device.json` and pass a fake token to the `MockClient`). Token issuance and client construction are the untested-by-construction wrapper's job, exactly as `reconcile_agent_version` in `main.rs` is untested and delegates to the tested `upgrade::reconcile_impl`.
  Date/Author: 2026-09-21 / Claude (authoring)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Read `agent/AGENTS.md` (conventions) and `agent/ARCHITECTURE.md` (module map) first. Key facts for this task, with full paths:

- **The runtime boot path.** The daemon entry is `agent/src/main.rs::run_agent(log_options, latch)`. Its sequence is: init logging → `await_activation(&layout, ...)` → `reconcile_agent_version(&layout, &latch)` (upgrade reconcile) → `read_settings(&layout)` → apply log level → `build_app_options(settings)` → `run(options, latch.wait())` (the long-running server in `agent/src/app/run.rs`). Provision/reprovision are separate `main.rs` paths (`run_provision`/`run_reprovision`) and already report metadata; **do not** touch them. `build_app_options(settings)` consumes `settings` by move (it moves `settings.backend.host` into `AppOptions`), so any use of the backend host must happen **before** that call.

- **The metadata type (today).** `agent/src/provisioning/shared.rs` defines `pub(super) struct SystemMetadata` with five fields — `os: Option<backend_client::Os>`, `arch: Option<backend_client::Arch>`, `hostname: Option<String>`, `os_version: Option<String>`, `kernel_version: Option<String>` — plus `pub(super) fn system_metadata() -> SystemMetadata` and private helpers `map_os`, `map_arch`, `non_empty`, `build_system_metadata`. `backend_client` is `backend_api::models`. Callers today: `agent/src/provisioning/provision.rs:89` and `agent/src/provisioning/reprovision.rs:60`, each `shared::system_metadata()` then moving `meta.os`/`meta.hostname`/etc. into a `ProvisionDeviceRequest` / `ReprovisionDeviceRequest`. `mod shared;` is private in `agent/src/provisioning/mod.rs`, which re-exports only `read_token_from_env`.

- **The cache pattern to mirror.** `agent/src/disk/agent_version.rs` is the canonical single-value on-disk marker:

		pub async fn read(file: &filesys::File) -> Result<Option<String>, DiskErr> {
		    if !file.exists() {
		        return Ok(None);
		    }
		    let raw = files::read_string(file).await?;
		    Ok(Some(raw.trim().to_string()))
		}

		pub async fn write(file: &filesys::File, version: &str) -> Result<(), DiskErr> {
		    let body = format!("{}\n", version.trim());
		    files::write_string(file, &body, WriteOptions::OVERWRITE_ATOMIC).await?;
		    Ok(())
		}

  Its `Layout` path method is `Layout::agent_version(&self) -> filesys::File` = `self.root().file("agent_version")` (in `agent/src/disk/layout.rs`). `disk::agent_version::read` returning `Ok(None)` for a missing file, and `upgrade::needs_upgrade` treating a read **error** as "missing", is the exact idempotence pattern this cache follows.

- **JSON helpers.** `agent/src/filesys/files.rs` provides `read_json::<T: DeserializeOwned>(file) -> Result<T, FileSysErr>` and `write_json::<T: Serialize>(file, &val, WriteOptions) -> Result<(), FileSysErr>`. `WriteOptions::OVERWRITE_ATOMIC` is defined in `agent/src/filesys/mod.rs`. `DiskErr` (`agent/src/disk/errors.rs`) has `From<filesys::FileSysErr>`, so `?` converts cleanly.

- **HTTP update.** `agent/src/http/devices.rs::update(client: &impl ClientI, params: UpdateParams) -> Result<Device, HTTPErr>` PATCHes `{base_url}/devices/{id}`. `UpdateParams<'a> { id: &'a str, payload: &'a UpdateDeviceFromAgentRequest, token: &'a str }`. In `libs/backend-api/src/models/update_device_from_agent_request.rs`, **every** field — including `agent_version` — is `#[serde(skip_serializing_if = "Option::is_none")]`, so `agent_version: None` is omitted from the body (the upgrade path owns `agent_version`, per PR #259; see `agent/src/app/upgrade.rs::update_device`, which sets the five metadata fields to `None` and `agent_version: Some(version)` — the mirror image of this plan's request).

- **Token + device id.** `agent/src/authn/issue.rs::issue_token(http_client: &impl http::ClientI, private_key_file: &File, public_key_file: &File) -> Result<Token, AuthnErr>`; `Token` has a `.token: String`. `agent/src/app/upgrade.rs::issue_token(http_client, layout)` shows the wrapper: read `layout.auth().private_key()` / `.public_key()`, call `authn::issue_token`. `agent/src/disk/device.rs::resolve_device_id(layout: &Layout) -> Result<String, DiskErr>` reads `device.json`, falling back to the JWT on file (re-exported as `disk::resolve_device_id`).

- **The mock for tests.** `agent/tests/mocks/http_client.rs::MockClient` implements `http::ClientI`. Relevant helpers: `set_update_device(|| Ok/Err(...))`, `set_get_device`, `num_update_device_calls()`, `call_count(Call::UpdateDevice)`, `requests()` (each `CapturedRequest` has `body`, `token`, `path`, `method`). Force a failure with `HTTPErr::MockErr(miru_agent::http::errors::MockErr { is_network_conn_err: true })`. The upgrade suite `agent/tests/app/upgrade.rs` is the template: `prepare_layout` (temp dir via `test_utils::filesys::dirs::temp`, real RSA keypair under `auth/`), `make_mock_client`, and per-error-path assertions using `matches!(err, UpgradeErr::HTTPErr(_))`.

- **Coverage gates (strict).** Per-directory `.covgate` files: `agent/src/models/.covgate` = `100`, `agent/src/disk/.covgate` = `96.79`, `agent/src/app/.covgate` = `90.38`. `scripts/covgate.sh` enforces them; it is the CI `test` job. New code in each directory must keep its directory at/above its threshold — `models` in particular must stay at **100%**, so every line of `models/system_metadata.rs` (including `to_update_request`) needs test coverage.

## Interfaces and Dependencies

New/changed public surface (exact signatures):

- `agent/src/models/system_metadata.rs` (new):
  - `#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct SystemMetadata { pub os: Option<backend_client::Os>, pub arch: Option<backend_client::Arch>, pub hostname: Option<String>, pub os_version: Option<String>, pub kernel_version: Option<String> }`
  - `pub fn system_metadata() -> SystemMetadata` (moved verbatim, plus its private `map_os`/`map_arch`/`non_empty`/`build_system_metadata` and inline tests)
  - `impl SystemMetadata { pub fn to_update_request(&self) -> backend_client::UpdateDeviceFromAgentRequest }` (new; `agent_version: None`, the five fields cloned/copied from `self`)
- `agent/src/models/mod.rs`: add `pub mod system_metadata;` and `pub use self::system_metadata::{system_metadata, SystemMetadata};`
- `agent/src/disk/layout.rs`: add `pub fn system_metadata(&self) -> filesys::File { self.root().file("system_metadata.json") }`
- `agent/src/disk/system_metadata.rs` (new), mirroring `disk::agent_version`:
  - `pub async fn read(file: &filesys::File) -> Result<Option<SystemMetadata>, DiskErr>`
  - `pub async fn write(file: &filesys::File, meta: &SystemMetadata) -> Result<(), DiskErr>`
- `agent/src/disk/mod.rs`: add `pub mod system_metadata;`
- `agent/src/app/metadata_sync.rs` (new):
  - `pub async fn sync_system_metadata(http_client: &impl ClientI, layout: &Layout, token: &str) -> Result<Synced, MetadataSyncErr>` (tested core)
  - `pub async fn run_on_boot(layout: &Layout, backend_host: &crate::network::BackendHost)` (best-effort; returns `()`)
  - `#[derive(Debug, PartialEq, Eq)] pub enum Synced { Unchanged, Updated }`
  - `#[derive(Debug, thiserror::Error)] pub enum MetadataSyncErr { DiskErr(#[from] disk::DiskErr), HTTPErr(#[from] http::HTTPErr) }` (defined in `agent/src/app/errors.rs`, mirroring `UpgradeErr`'s `#[from]`/`transparent` style; re-export from `agent/src/app/mod.rs`)
- `agent/src/app/mod.rs`: add `pub mod metadata_sync;`

## Plan of Work

**Milestone 1 — Relocate `SystemMetadata` into `crate::models`.**
Create `agent/src/models/system_metadata.rs`. Move from `agent/src/provisioning/shared.rs`: the `SystemMetadata` struct (with widened derives and `pub` fields), `map_os`, `map_arch`, `non_empty`, `build_system_metadata`, `system_metadata`, and the inline `#[cfg(test)] mod tests { mod system_metadata { ... } }` block (the `read_token_from_env` tests stay in `shared.rs`). Add `use serde::{Deserialize, Serialize};` and keep `use backend_api::models as backend_client;` and `use crate::telemetry;`. Add the `to_update_request` method and an inline unit test asserting the whole returned struct (with `agent_version: None`). In `agent/src/models/mod.rs` add the `pub mod` + `pub use`. In `agent/src/provisioning/shared.rs` delete the moved items and their imports if now unused (`telemetry`, `backend_client`); keep `read_token_from_env`, `cleanup_temp_dir`, `determine_settings`. Repoint callers: `agent/src/provisioning/provision.rs:89` and `agent/src/provisioning/reprovision.rs:60` change `shared::system_metadata()` → `crate::models::system_metadata()`; `provision.rs` already has `use crate::models;`, so `reprovision.rs` needs `use crate::models;` added in its internal-crates group. The field-move usage (`meta.os`, `meta.hostname`, …) is unchanged. Run `cargo fmt` and the import linter expectations (grouped imports with the `// internal crates` comment).

**Milestone 2 — Disk cache.**
Add `Layout::system_metadata()` in `agent/src/disk/layout.rs` (place it next to `agent_version()`), returning `self.root().file("system_metadata.json")`. Create `agent/src/disk/system_metadata.rs` mirroring `disk::agent_version` but JSON-typed: `read` returns `Ok(None)` when `!file.exists()`, else `files::read_json::<SystemMetadata>(file).await?` wrapped in `Some`; `write` calls `files::write_json(file, meta, WriteOptions::OVERWRITE_ATOMIC).await?`. Imports: `use crate::disk::errors::DiskErr; use crate::filesys::{self, files, PathExt, WriteOptions}; use crate::models::SystemMetadata;`. Add `pub mod system_metadata;` to `agent/src/disk/mod.rs`. Add integration tests `agent/tests/disk/system_metadata.rs` (declare `pub mod system_metadata;` in `agent/tests/disk/mod.rs`) modeled on `agent/tests/disk/agent_version.rs`: missing-file → `None`; write-then-read round-trips a fully-populated `SystemMetadata`; write-then-read round-trips one with `None` fields (asserting whole-struct equality).

**Milestone 3 — Boot sync + wiring.**
Add `MetadataSyncErr` to `agent/src/app/errors.rs` and re-export from `agent/src/app/mod.rs`. Create `agent/src/app/metadata_sync.rs`:

- `sync_system_metadata(http_client, layout, token)`: gather `let live = models::system_metadata();`. Read the cache via `disk::system_metadata::read(&layout.system_metadata())`, treating **both** `Ok(None)` and `Err(_)` as "no cache" (log the error case at `debug`/`warn`; do not propagate — matches "missing/unreadable cache → report"). If `cached.as_ref() == Some(&live)` → log `debug` and return `Ok(Synced::Unchanged)` (no device-id resolution, no network). Otherwise resolve `let id = disk::resolve_device_id(layout).await?;`, build `let payload = live.to_update_request();`, call `http::devices::update(http_client, http::devices::UpdateParams { id: &id, payload: &payload, token }).await?`, then on success `disk::system_metadata::write(&layout.system_metadata(), &live).await?` and return `Ok(Synced::Updated)`. Because the cache write follows the successful PATCH, a PATCH failure returns `Err(HTTPErr)` **before** the cache is written (retry next boot).
- `run_on_boot(layout, backend_host)`: best-effort, returns `()`. Build `http::Client::new(&backend_host.as_url())`; issue a token with `authn::issue_token(&client, &layout.auth().private_key(), &layout.auth().public_key())`; call `sync_system_metadata(&client, layout, &token.token)`. Log every failure at `warn` and swallow it (mirror `reconcile_agent_version`'s error handling). Never return an error, never panic.

Keep each production function within the 50-line limit (`lint:allow(funclen)` only if unavoidable). Wire into `agent/src/main.rs::run_agent`: after `read_settings` succeeds and the log level is applied, and **before** `let options = build_app_options(settings);`, insert `miru_agent::app::metadata_sync::run_on_boot(&layout, &settings.backend.host).await;`. Add integration tests `agent/tests/app/metadata_sync.rs` (declare in `agent/tests/app/mod.rs`) using the `upgrade.rs` harness (`MockClient`, temp `Layout`, seeded `device.json` with a known id so `resolve_device_id` succeeds without a token). Cover: (a) cache == live → `Synced::Unchanged`, `num_update_device_calls() == 0`, cache file unchanged; (b) cache absent and cache != live → `Synced::Updated`, exactly one `Call::UpdateDevice`, request body carries the live fields and **no** `agent_version`, cache file now equals live; (c) `set_update_device(|| Err(MockErr...))` → `Err(MetadataSyncErr::HTTPErr(_))`, cache file **not** written (`read` still `None`/prior value), no panic.

## Concrete Steps

All commands run from the worktree root `agent-wt2` unless noted; the binary crate is `miru-agent`.

**Milestone 1.**

1. Create `agent/src/models/system_metadata.rs` and edit `agent/src/models/mod.rs`, `agent/src/provisioning/shared.rs`, `agent/src/provisioning/provision.rs`, `agent/src/provisioning/reprovision.rs` as in Plan of Work M1.
2. Build + unit test the moved type:

		cargo test --package miru-agent models::system_metadata

   Expect the moved tests (`map_os_*`, `map_arch_*`, `build_*`, `system_metadata_reads_host_os_and_arch`) plus the new `to_update_request` test to pass.
3. `cargo fmt -p miru-agent -- --check` (expect no diff) and `cargo build --package miru-agent` (expect success; confirms provisioning callers compile against the relocated path).
4. Commit: `feat(models): relocate SystemMetadata into crate::models with cache-ready derives`.

**Milestone 2.**

5. Edit `agent/src/disk/layout.rs`, create `agent/src/disk/system_metadata.rs`, edit `agent/src/disk/mod.rs`; create `agent/tests/disk/system_metadata.rs` and edit `agent/tests/disk/mod.rs`.
6. Run:

		cargo test --package miru-agent disk::system_metadata

   Expect the round-trip and missing-file tests to pass.
7. Commit: `feat(disk): add JSON system-metadata cache mirroring agent_version marker`.

**Milestone 3.**

8. Edit `agent/src/app/errors.rs`, `agent/src/app/mod.rs`; create `agent/src/app/metadata_sync.rs`; edit `agent/src/main.rs`; create `agent/tests/app/metadata_sync.rs` and edit `agent/tests/app/mod.rs`.
9. Run:

		cargo test --package miru-agent app::metadata_sync

   Expect scenarios (a)/(b)/(c) to pass.
10. Full local gate before pushing:

		scripts/update-deps.sh
		scripts/preflight.sh

   `preflight.sh` runs lint + `covgate.sh` (tests + coverage) + tools lint/tests in parallel and prints each section; expect it to finish with all sections passing (this is the local "CLEAN" signal). If `covgate.sh` reports a directory below threshold (watch `models` = 100), add the missing test and re-run.
11. Commit: `feat(app): sync device system metadata on boot, best-effort via local cache`.

**Milestone 4 — push + CI.**

12. Push the branch and open a **draft** PR against `feat/agent-report-system-metadata`:

		git push -u origin feat/agent-metadata-boot-sync

   Then create the draft PR (base `feat/agent-report-system-metadata`). Use the `preflight` skill to drive the refine→push→CI-watch loop until green (see Validation).

## Validation and Acceptance

**Automated tests (behavior, before/after).**

- `disk::system_metadata` round-trip: `agent/tests/disk/system_metadata.rs` — `write(file, &meta)` followed by `read(file)` returns `Some(meta)` for a fully-populated value and for one with `None` fields (whole-struct `assert_eq!`); `read` on a path with no file returns `Ok(None)`. These tests do not exist before this change and pass after.
- `app::metadata_sync::sync_system_metadata` decision logic with `MockClient`:
  - (a) cache seeded equal to live → returns `Synced::Unchanged`; `mock.num_update_device_calls() == 0`; the cache file is byte-identical afterward.
  - (b) cache absent, live differs → returns `Synced::Updated`; exactly one `Call::UpdateDevice`; the captured request `body` contains the live `hostname`/`os_version`/`kernel_version` and does **not** contain `"agent_version"`; `disk::system_metadata::read` afterward equals live.
  - (c) `set_update_device` returns `HTTPErr::MockErr{is_network_conn_err:true}` → returns `Err(MetadataSyncErr::HTTPErr(_))` (asserted with `matches!`, never on message text); `disk::system_metadata::read` afterward is still `None` (cache not written); the test process does not panic.
- Run the whole package: `cargo test --package miru-agent --locked` → expect `0 failed` (this is exactly what the CI `windows-check` job runs, ensuring portability; keep new tests free of `#[cfg(unix)]` unless they assert Unix-only semantics).

**Coverage.** `scripts/covgate.sh` (the CI `test` job) must pass: `models` stays at `100`, `disk` at/above `96.79`, `app` at/above `90.38`.

**Manual/behavioral acceptance (optional, on a provisioned device).** Start the daemon; observe in the logs a single `PATCH /devices/{id}` on first boot after a hostname change and `system_metadata.json` appearing/updating under the data root; restart with no host change and observe zero metadata network calls and a `debug` "unchanged" line. Kill network connectivity and start the daemon: startup still completes (metadata sync logs a `warn` and is skipped), proving it never blocks boot.

**CI / preflight gate (required before the PR leaves draft or the task is reported complete).** The `preflight` skill's terminal state is `CLEAN`, meaning **CI is green on the pushed branch head**. The CI workflow is `.github/workflows/ci.yml` with jobs `lint` (`LINT_FIX=0 ./scripts/lint.sh`), `test` (`./scripts/covgate.sh`), and `windows-check` (`cargo test --package miru-agent --locked`); the Windows packaging jobs run only when `build/windows/**` changes (not touched here). The PR **must not** be marked ready for review, and this task **must not** be reported complete, until preflight reports `CLEAN` — i.e. all required CI jobs on the exact pushed commit are green. A locally-passing `scripts/preflight.sh` is necessary but not sufficient; the authoritative signal is CI on the branch head.

## Idempotence and Recovery

- **Cache reads/writes are idempotent.** `disk::system_metadata::read` returns `Ok(None)` for a missing file and the sync core treats a read **error** identically (report, do not fail) — a corrupt or partially written cache self-heals on the next boot by triggering a PATCH + fresh write. `write` uses `WriteOptions::OVERWRITE_ATOMIC`, so an interrupted write never leaves a torn file. Re-running the sync when metadata is unchanged is a no-op (no network, no write).
- **Ordering guarantees no false cache.** The cache is written **only after** a successful PATCH; a PATCH failure returns before the write, so the stale-then-retry invariant holds across restarts. Deleting `system_metadata.json` by hand simply forces one PATCH on the next boot — safe.
- **Boot is never blocked.** `run_on_boot` returns `()` and swallows every error (token issuance, client build, disk, HTTP). If PR #259's base changes the `system_metadata()`/request shape during a rebase, re-verify the five field names in `to_update_request` and the `UpdateParams` call site; a compile break there is the expected signal.
- **Each milestone is an independent commit** so the stack can be bisected; M1 and M2 are behavior-preserving (relocation + unused cache module) and safe to land even if M3 is reworked.
