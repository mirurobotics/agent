# Make miru-agent package upgrades idempotent via boot-time rebootstrap

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `repos/agent/` | read-write | All source, tests, and OpenAPI changes for this work live here. Touches `agent/src/{storage,app,sync,authn,http,main.rs}`, `api/specs/backend/v03.yaml`, and the generated crate at `libs/backend-api/`. |
| `repos/backend/` | read-only | Reference only, to confirm the new `GET /devices/{device_id}` endpoint shape that this agent change consumes. The central OpenAPI repo will be updated by a separate change; this plan only edits the spec mirrored at `repos/agent/api/specs/backend/v03.yaml`. |

This plan lives in `repos/agent/plans/backlog/` because every code edit lands in the agent repository.

## Purpose / Big Picture

After this change a Debian package upgrade of `miru-agent` becomes self-healing. When the new binary boots it compares its own compile-time `version::VERSION` against a persisted on-disk marker. If they disagree, or the marker is missing, the agent wipes all stateful files except the device's RSA keypair, re-fetches its `Device` record from the backend (using a private-key-signed JWT), and rewrites `device.json`, `settings.json`, the auth token, and the marker. After that the agent boots normally with fresh, schema-current state.

User-visible outcome: bumping the agent version in `Cargo.toml`, building, installing, and starting the service results in a healthy agent on the next boot, with the on-disk `agent_version.json` reflecting the new version, even when the previous on-disk schema for `device.json` or `settings.json` would otherwise have been incompatible. Operators can simulate this by deleting `/var/lib/miru/agent_version.json` and restarting the service: the agent re-bootstraps and comes up clean.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Read fresh state of `agent/src/storage/layout.rs` and `agent/src/storage/setup.rs` on `main` (these were updated very recently — do not trust this plan's pointers blindly; re-open the files first).
- [ ] Add `Layout::agent_version()` accessor returning `/var/lib/miru/agent_version.json`.
- [ ] Add `agent/src/storage/agent_version.rs` defining `pub struct AgentVersion { pub version: String }` plus `read(&filesys::File) -> Result<Option<AgentVersion>, StorageErr>` and `write(&filesys::File, &AgentVersion) -> Result<(), StorageErr>`. Wire it through `agent/src/storage/mod.rs`.
- [ ] Refactor `agent/src/storage/setup.rs` so that `setup::reset(layout, device, settings)` becomes the single source of truth for resetting persistent state, and `setup::bootstrap(layout, device, settings, private_key_file, public_key_file)` keeps its installer signature by moving keys into `auth/` and then delegating to `reset`.
- [ ] Extend `agent/src/authn/` so a JWT can be minted from just a private key + device id without instantiating the full `TokenManager`. Provide `authn::issue_token_with_private_key(http_client, private_key_file, device_id) -> Result<Token, AuthnErr>` (or the smallest equivalent surface) reused by both the existing token manager and the new `app::upgrade` flow.
- [ ] Add `GET /device` (operationId `getDevice`, security `ClerkAuth`, no path params, returns `#/components/schemas/Device`) to `api/specs/backend/v03.yaml` and re-run `api/regen.sh` so `libs/backend-api/src/models/` reflects the regenerated types. Add `http::devices::get(client, token) -> Result<Device, HTTPErr>` in `agent/src/http/devices.rs`.
- [ ] Create `agent/src/app/upgrade.rs` exposing `pub async fn ensure(layout: &Layout, http_client: &impl ClientI, version: &str) -> Result<(), UpgradeErr>` that no-ops on marker match and otherwise loops on `GET /device` with `cooldown::Backoff` until the backend responds, then calls `setup::reset`, PATCHes the backend with the new agent version, and writes the marker. Wire it into `agent/src/app/mod.rs`.
- [ ] Integrate `app::upgrade::ensure` into `main.rs::run_agent` immediately after `let layout = storage::Layout::default();` and before `storage::assert_activated`. On error, log and exit.
- [ ] Delete `agent/src/sync/agent_version.rs`, the corresponding `pub mod agent_version;` line in `agent/src/sync/mod.rs`, the `agent_version::push(...)` block in `sync::syncer::SingleThreadSyncer::sync_impl`, and the `agent_version: String` field on `SyncerArgs` and `SingleThreadSyncer` if no remaining consumer references it.
- [ ] Remove the `agent_version` parameter threaded through `agent/src/app/run.rs` and `agent/src/app/state.rs` (only the syncer-bound flow). Keep the parameter in `run.rs` ONLY if it has another genuine use (e.g. telemetry/log line). Delete `agent/tests/sync/agent_version.rs` and the `pub mod agent_version;` in `agent/tests/sync/mod.rs`.
- [ ] Add tests under `agent/tests/storage/setup.rs` for `setup::reset` (preserves both keys, wipes everything else, writes marker).
- [ ] Add tests under `agent/tests/app/upgrade.rs` for `ensure` (no-op when marker matches; rebootstrap when marker missing; rebootstrap when marker version differs; backoff retries `GET /device` until success; final state contains marker, fresh device, blank token, both keys preserved). Wire `pub mod upgrade;` into `agent/tests/app/mod.rs`.
- [ ] Run preflight (`scripts/preflight.sh`) and confirm output ends with `Preflight clean`. No lint, format, test, covgate, or regenerated-API drift errors.
- [ ] Open PR following `repos/agent/AGENTS.md` conventions; PR description references this plan path.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Preserve set is `auth/private_key.pem`, `auth/public_key.pem`, and `agent_version.json`. Everything else (`device.json`, `settings.json`, `auth/token.json`, `resources/`, `events/`, `/srv/miru/config_instances/`) is wiped on rebootstrap.
  Rationale: The keypair is the device's identity at the backend — losing it would orphan the device. Everything else is recoverable from the backend or re-derivable from compile-time defaults. Keeping the marker ensures the next boot is a no-op.
  Date/Author: 2026-04-26 / locked with user before authoring.

- Decision: Marker file path is `/var/lib/miru/agent_version.json` with shape `{ "version": "x.y.z" }`. Accessor lives at `Layout::agent_version()`. Storage type follows the `storage::Device`/`storage::Settings` read/write pattern.
  Rationale: Keeps the marker beside other agent state, version-as-string is forward-compatible, and the accessor + read/write pair matches existing conventions so reviewers do not need to learn a new pattern.
  Date/Author: 2026-04-26 / locked with user.

- Decision: Compile-time defaults from `Settings::default()` (`Backend::default().base_url = "https://api.mirurobotics.com/agent/v1"`, `MQTTBroker::default().host = "mqtt.mirurobotics.com"`) drive both the rebootstrap-time HTTP client and the rewritten `settings.json`.
  Rationale: After a wipe there is no settings file to read; the only honest source of truth is the binary itself. This matches how the installer currently works (`installer::install::determine_settings`).
  Date/Author: 2026-04-26 / locked with user.

- Decision: Any version disagreement (upgrade, downgrade, missing marker) triggers a full rebootstrap. Missing marker is treated as upgrade.
  Rationale: Simplest correct policy; a downgrade can equally invalidate on-disk schemas. Missing marker is the upgrade-from-pre-marker case.
  Date/Author: 2026-04-26 / locked with user.

- Decision: When the backend is unreachable during rebootstrap, `app::upgrade::ensure` retries forever using `cooldown::Backoff` rather than failing the boot.
  Rationale: A rebootstrap that fails fast leaves the agent in a half-wiped state on the next boot; better to block until the backend is reachable. User chose option (a).
  Date/Author: 2026-04-26 / locked with user.

- Decision: The OpenAPI change for `GET /devices/{device_id}` (`operationId: getDevice`) is made only in `repos/agent/api/specs/backend/v03.yaml`. The central OpenAPI repository will be updated separately later.
  Rationale: Avoid coupling this PR's merge to a cross-repo coordination. Re-running `api/regen.sh` against the local spec produces the new client types in `libs/backend-api/` immediately.
  Date/Author: 2026-04-26 / locked with user.

- Decision: JWT signing for the rebootstrap path reuses the existing private-key-based signing logic, not a new auth surface. If the existing `authn::Token`/`TokenManager` requires a `device_id` parameter that isn't available before `device.json` is rewritten, the chosen workaround is to extend `authn` so that the signing helper accepts an explicit `device_id` argument, not to add `device_id` to the marker file.
  Rationale: The marker is a pure version stamp; widening it would couple unrelated concerns. The device id is recoverable from the backend response, and the JWT itself encodes it.
  Date/Author: 2026-04-26 / locked with user (bias toward extending `authn`). Note: see Context section — `agent/src/authn/token_mngr.rs::SingleThreadTokenManager::prepare_issue_token_request` already encapsulates "sign claims with private key file"; the simplest extension is a free function in `agent/src/authn/` that takes `(private_key_file, device_id, http_client)` and returns a `Token`, and have the manager call it.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The miru-agent is a Rust binary (crate name `miru_agent`, package crate at `repos/agent/agent/`). Its persistent state lives under `/var/lib/miru/` on the device and is currently bootstrapped by the installer subcommand (`agent/src/installer/install.rs`, called from `agent/src/main.rs::run_installer`). Once installed, the long-lived service is started by `run_agent()` in `agent/src/main.rs`, which reads `device.json` and `settings.json`, asserts the device is activated, then enters `app::run::run`.

Today, the agent has no concept of a self-driven schema migration on package upgrade. If a new binary changes the on-disk schema for `Device` or `Settings`, the agent will fail to deserialize them and crash. There is also no marker recording which version last successfully booted, so there is nothing to detect "I am newer than what's on disk." A file-based `agent_version` does flow from `Cargo.toml` to the running process via `version::VERSION` (`agent/src/version/mod.rs`), and is then pushed to the backend each sync via `sync::agent_version::push` (called from `sync::syncer::SingleThreadSyncer::sync_impl`). That push path is the only place the running version is reconciled with persistent state today, and it only updates the embedded `agent_version` field on the `Device` model — it does nothing about schema changes.

Key existing files and types this plan touches:

- `agent/src/main.rs` — entrypoint. `run_agent` is the boot path. Currently `let layout = storage::Layout::default();` is followed by `storage::assert_activated`, then settings load, logging init, and `app::run::run`. The new upgrade gate inserts between the layout construction and `assert_activated`.
- `agent/src/storage/layout.rs` — defines `Layout`, `AuthLayout`, and accessors that produce `filesys::File`/`filesys::Dir` handles for paths under `/var/lib/miru/`. Existing accessors include `device()`, `settings()`, `resources()`, `events_dir()`, and `auth()`. We add `agent_version()` returning `self.root().file("agent_version.json")`.
- `agent/src/storage/setup.rs` — current shape (one function): `pub async fn bootstrap(layout, device, settings, private_key_file, public_key_file) -> Result<(), StorageErr>` writes `device.json`, `settings.json`, blank `token.json`, moves keypair into `auth/`, deletes `resources/`, and recreates `events/`. The refactor extracts the non-key-moving body into `pub async fn reset(layout, device, settings) -> Result<(), StorageErr>` and reduces `bootstrap` to "move keys + reset". `reset` additionally wipes `/srv/miru/config_instances/` (currently no accessor — see below) and writes the new marker.
- `agent/src/storage/device.rs` — `pub type Device = ConcurrentCachedFile<models::Device, device::Updates>;`. Not a plain serde struct read/write helper. The new `storage::AgentVersion` follows the simpler `Settings` pattern (a plain `serde::{Serialize, Deserialize}` struct with helpers) rather than the cached-actor pattern, because the marker is read once at boot and written once at upgrade.
- `agent/src/storage/settings.rs` — reference for the marker's read/write style. `Settings::default()` is the source of truth for `Backend.base_url` and `MQTTBroker.host` post-rebootstrap.
- `agent/src/storage/mod.rs` — re-exports. Add `pub mod agent_version;` and `pub use self::agent_version::AgentVersion;`.
- `agent/src/sync/agent_version.rs` — to be deleted.
- `agent/src/sync/syncer.rs` — `SingleThreadSyncer` holds `agent_version: String`; `SyncerArgs` carries it; `sync_impl` calls `agent_version::push`. Remove all three. Remove the `pub mod agent_version` line from `agent/src/sync/mod.rs`.
- `agent/src/app/run.rs` — currently `pub async fn run(agent_version: String, options, shutdown_signal)` threads `agent_version` through `init` → `init_app_state` → `AppState::init` → `SyncerArgs`. With the syncer-bound consumer gone, remove the field/arg threading at every layer, unless a non-syncer use survives (e.g. a telemetry log line). If the parameter is fully unused, drop it from `run`'s signature and update `main.rs::run_agent` accordingly.
- `agent/src/app/state.rs` — drop the `agent_version: String` parameter from `AppState::init`'s arg list and the `SyncerArgs { ..., agent_version, ... }` field on the call site. `init_device_id` does not reference `agent_version` so no changes there.
- `agent/src/app/mod.rs` — add `pub mod upgrade;`.
- `agent/src/app/upgrade.rs` (NEW) — owns the `ensure(...)` function and an `UpgradeErr` enum that wraps `StorageErr`, `HTTPErr`, and `AuthnErr`.
- `agent/src/authn/token_mngr.rs` — `SingleThreadTokenManager::prepare_issue_token_request` and `issue_token` are the prior art for "sign with private key, post to `/devices/{id}/issue_token`, return `Token`." Extract a private-key-only entry point to a helper free function (e.g. `agent/src/authn/issue.rs::issue_token(http_client, private_key_file, device_id) -> Result<Token, AuthnErr>`) and have both `SingleThreadTokenManager::issue_token` and `app::upgrade::ensure` call it.
- `agent/src/http/devices.rs` — add `pub async fn get(client: &impl ClientI, token: &str) -> Result<Device, HTTPErr>` that hits `GET {base_url}/device` with bearer auth (no path params, since the device id is encoded in the JWT).
- `api/specs/backend/v03.yaml` — adds a new path `/device` (no `device_id` in path) under tags `Devices`, `operationId: getDevice`, `security: [ClerkAuth: []]`, response `200` body `#/components/schemas/Device`. The existing `/devices/{device_id}` PATCH and friends remain unchanged.
- `libs/backend-api/src/models/` — regenerated by `api/regen.sh`. Adds nothing new at the model layer (response is the existing `Device` schema), but the request file may be touched depending on generator behavior.
- `agent/src/cooldown/mod.rs` — provides `Backoff { base_secs, growth_factor, max_secs }` and `calc(&backoff, exp) -> i64`. The `app::upgrade::ensure` retry loop uses the same `Backoff` shape that `SingleThreadSyncer` uses (`base_secs: 1, growth_factor: 2, max_secs: 12 * 60 * 60`).

Definitions of non-obvious terms:

- "Rebootstrap" — the post-install path performed by the running binary (not the installer subcommand) that resets all on-disk state except the keypair and the marker.
- "Marker" — the `agent_version.json` file at `/var/lib/miru/agent_version.json`. Its presence + matching version is the signal "no rebootstrap needed."
- "JWT signing from just the private key" — minting an `IssueDeviceTokenRequest` whose `claims.device_id` and signature are produced from the on-disk PEM without first requiring `device.json` to exist. The backend already accepts this (it's how `TokenManager` refreshes), but the existing helper requires constructing the manager with a device id. We add a private-key-only helper.

OTA agent: ignored. Treated as dead code per locked decision.

## Plan of Work

### 1. `Layout::agent_version()` and `storage::AgentVersion`

Edit `agent/src/storage/layout.rs`. Add an accessor:

	pub fn agent_version(&self) -> filesys::File {
	    self.root().file("agent_version.json")
	}

Create `agent/src/storage/agent_version.rs`:

	use serde::{Deserialize, Serialize};
	use crate::filesys::{self, PathExt, WriteOptions};
	use crate::storage::errors::StorageErr;

	#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
	pub struct AgentVersion {
	    pub version: String,
	}

	pub async fn read(file: &filesys::File) -> Result<Option<AgentVersion>, StorageErr> {
	    if !file.exists() { return Ok(None); }
	    Ok(Some(file.read_json::<AgentVersion>().await?))
	}

	pub async fn write(file: &filesys::File, marker: &AgentVersion) -> Result<(), StorageErr> {
	    file.write_json(marker, WriteOptions::OVERWRITE_ATOMIC).await?;
	    Ok(())
	}

Wire it into `agent/src/storage/mod.rs`:

	pub mod agent_version;
	pub use self::agent_version::AgentVersion;

Note: the existing `storage::Device` is a `ConcurrentCachedFile`-based type, not a serde struct; the new `AgentVersion` is intentionally simpler (read-once, write-once) and matches the `Settings` style of being a plain `Serialize + Deserialize` struct with thin helpers. Document this in the module docstring.

### 2. Refactor `setup.rs` into `bootstrap` + `reset`

Edit `agent/src/storage/setup.rs`:

	pub async fn reset(
	    layout: &Layout,
	    device: &models::Device,
	    settings: &Settings,
	) -> Result<(), StorageErr> {
	    // ensure auth dir exists (token.json lives there)
	    let auth_dir = layout.auth();
	    auth_dir.root.create_if_absent().await?;

	    // overwrite device.json
	    layout.device().write_json(device, WriteOptions::OVERWRITE_ATOMIC).await?;

	    // overwrite settings.json
	    layout.settings().write_json(settings, WriteOptions::OVERWRITE_ATOMIC).await?;

	    // blank token.json
	    auth_dir.token().write_json(&authn::Token::default(), WriteOptions::OVERWRITE_ATOMIC).await?;

	    // wipe resources/ (also wipes config_instances/)
	    layout.resources().delete().await?;

	    // wipe events/ and recreate
	    let events_dir = layout.events_dir();
	    events_dir.delete().await?;
	    events_dir.create_if_absent().await?;

	    // wipe /srv/miru/config_instances/ if a layout accessor exists for it; else
	    // explicit `Dir::new("/srv/miru/config_instances").delete().await?;` with the
	    // same `delete` helper. CONFIRM during implementation that no such accessor
	    // exists today (current `layout.rs` only exposes resources()/config_instances()
	    // under /var/lib/miru). If it doesn't, add a `Layout::customer_configs() ->
	    // Dir { Dir::new("/srv/miru/config_instances") }` accessor for symmetry rather
	    // than hardcoding the path inside `reset`.

	    // write the new marker
	    storage::agent_version::write(
	        &layout.agent_version(),
	        &storage::AgentVersion { version: VERSION_PLACEHOLDER },
	    ).await?;

	    Ok(())
	}

The marker version cannot be hardcoded inside `reset`; pass it as an argument. The final signature is:

	pub async fn reset(
	    layout: &Layout,
	    device: &models::Device,
	    settings: &Settings,
	    agent_version: &str,
	) -> Result<(), StorageErr>

Then rewrite `bootstrap` to delegate:

	pub async fn bootstrap(
	    layout: &Layout,
	    device: &models::Device,
	    settings: &Settings,
	    private_key_file: &filesys::File,
	    public_key_file: &filesys::File,
	) -> Result<(), StorageErr> {
	    let auth_dir = layout.auth();
	    auth_dir.root.create_if_absent().await?;
	    private_key_file.move_to(&auth_dir.private_key(), Overwrite::Allow).await?;
	    public_key_file.move_to(&auth_dir.public_key(), Overwrite::Allow).await?;
	    reset(layout, device, settings, &device.agent_version).await
	}

The installer call site at `agent/src/installer/install.rs:36` needs no change because the public signature of `bootstrap` is preserved; only its body changes. (Confirm this during implementation; the current installer passes the device version embedded in the `models::Device` value.)

### 3. Extend `authn` for private-key-only token issuance

Today `SingleThreadTokenManager::prepare_issue_token_request` and `issue_token` (`agent/src/authn/token_mngr.rs:87-150`) need a `device_id` and an `http_client`, both of which the upgrade flow has. The cleanest extension is to extract a free function:

	// agent/src/authn/issue.rs (NEW)
	use std::sync::Arc;
	use crate::authn::{errors::AuthnErr, token::Token};
	use crate::filesys::file::File;
	use crate::http;

	pub async fn issue_token<HTTPClientT: http::ClientI>(
	    http_client: &HTTPClientT,
	    private_key_file: &File,
	    device_id: &str,
	) -> Result<Token, AuthnErr> { /* same body as SingleThreadTokenManager::issue_token,
	                                  using the local args instead of self.* */ }

Wire it into `agent/src/authn/mod.rs` (`pub mod issue; pub use self::issue::issue_token;`) and refactor `SingleThreadTokenManager::issue_token` to delegate:

	async fn issue_token(&self) -> Result<Token, AuthnErr> {
	    issue::issue_token(self.http_client.as_ref(), &self.private_key_file, &self.device_id).await
	}

Edge case: `app::upgrade::ensure` does not yet have a `device_id`. It must `GET /device` first (which is JWT-authenticated), but minting the JWT requires the `device_id`. The cleanest order is therefore: extract `device_id` from the on-disk private key path? No — the device id lives in the public key fingerprint registered with the backend, not the private key itself. The simplest source of `device_id` pre-rebootstrap is `device.json` if it parses, else the existing `auth/token.json` if a still-valid JWT is present, else we need a different endpoint. Re-read `agent/src/app/state.rs::init_device_id` — it already does this exact `device.json → token.json::extract_device_id` fallback. The upgrade flow must do the same fallback before minting a fresh token. Document this in the implementation: `app::upgrade::ensure` calls a copy or shared helper of `init_device_id` to obtain the `device_id`, then issues a fresh token, then `GET /device`. If both `device.json` and `token.json` are missing/corrupt the agent cannot recover automatically — fail boot with a clear error message ("agent cannot rebootstrap: device id unknown; reinstall via `miru-agent install`"). This is acceptable because that scenario means the device was never installed.

Lift `init_device_id` out of `agent/src/app/state.rs::AppState::init` into a new `agent/src/storage/device.rs::resolve_device_id(layout) -> Result<String, ...>` (or similar) so both paths share it, then update `AppState::init` to call the shared helper.

### 4. OpenAPI spec edit + regen

Edit `api/specs/backend/v03.yaml`. Insert a new top-level path entry directly above `/devices/{device_id}`:

	  /device:
	    parameters:
	    - $ref: '#/components/parameters/MiruVersion'
	    get:
	      tags:
	      - Devices
	      summary: Get
	      operationId: getDevice
	      description: Retrieve the device record for the authenticated agent.
	      responses:
	        '200':
	          description: Successfully retrieved the device.
	          content:
	            application/json:
	              schema:
	                $ref: '#/components/schemas/Device'

Note: the global `security: [ClerkAuth: []]` declaration at line 80 of the spec already applies to this operation, so no per-operation `security` block is needed.

Then run `api/regen.sh` from `repos/agent/`. Confirm the diff is limited to `libs/backend-api/src/models/` (no behavioral changes since we're using existing schemas; the request body model files may rename or no-op). Commit the regenerated files in the same commit that adds the new `http::devices::get`.

### 5. New HTTP helper

Edit `agent/src/http/devices.rs`. Append:

	pub async fn get(client: &impl ClientI, token: &str) -> Result<Device, HTTPErr> {
	    let url = format!("{}/device", client.base_url());
	    let request = request::Params::get(&url).with_token(token);
	    super::client::fetch(client, request).await
	}

Confirm `request::Params::get` exists and has a `with_token` builder; if the existing module exposes only `post`/`patch` constructors, add a `get` constructor (look for a single-line addition in `agent/src/http/request.rs`).

### 6. New `app::upgrade` module

Create `agent/src/app/upgrade.rs`:

	use std::time::Duration;
	use crate::authn;
	use crate::cooldown;
	use crate::filesys::PathExt;
	use crate::http::{self, ClientI};
	use crate::models;
	use crate::storage::{self, Layout, Settings};

	#[derive(Debug, thiserror::Error)]
	pub enum UpgradeErr {
	    #[error(transparent)] StorageErr(#[from] storage::StorageErr),
	    #[error(transparent)] HTTPErr(#[from] http::errors::HTTPErr),
	    #[error(transparent)] AuthnErr(#[from] authn::AuthnErr),
	    #[error("device id unknown — agent has never been installed; run `miru-agent install`")]
	    UninstalledDeviceErr,
	}

	pub async fn ensure<HTTPClientT: ClientI>(
	    layout: &Layout,
	    http_client: &HTTPClientT,
	    version: &str,
	) -> Result<(), UpgradeErr> {
	    let marker_file = layout.agent_version();
	    if let Some(marker) = storage::agent_version::read(&marker_file).await? {
	        if marker.version == version { return Ok(()); }
	    }

	    // resolve device id from on-disk state (device.json -> token.json fallback)
	    let device_id = storage::device::resolve_device_id(layout).await
	        .map_err(|_| UpgradeErr::UninstalledDeviceErr)?;

	    let private_key_file = layout.auth().private_key();
	    private_key_file.assert_exists()?;

	    let backoff = cooldown::Backoff { base_secs: 1, growth_factor: 2, max_secs: 12 * 60 * 60 };
	    let mut err_streak: u32 = 0;
	    let device: backend_api::models::Device = loop {
	        let token = authn::issue_token(http_client, &private_key_file, &device_id).await?;
	        match http::devices::get(http_client, &token.token).await {
	            Ok(d) => break d,
	            Err(e) => {
	                tracing::warn!("upgrade: GET /device failed; will retry: {e}");
	                let wait = cooldown::calc(&backoff, err_streak);
	                tokio::time::sleep(Duration::from_secs(wait as u64)).await;
	                err_streak = err_streak.saturating_add(1);
	            }
	        }
	    };

	    let mut device_model: models::Device = (&device).into();
	    device_model.agent_version = version.to_string();
	    let settings = Settings::default();
	    storage::setup::reset(layout, &device_model, &settings, version).await?;

	    // PATCH backend with new agent_version (best-effort; reset already wrote the
	    // marker, so even if PATCH fails the on-disk state is consistent — the next
	    // sync will retry via the existing syncer? No — that path is being deleted.
	    // The PATCH must therefore succeed before we declare upgrade success: include
	    // it inside the same retry loop OR a second loop using the same backoff).
	    let token = authn::issue_token(http_client, &private_key_file, &device_id).await?;
	    let mut err_streak: u32 = 0;
	    loop {
	        let result = http::devices::update(http_client, http::devices::UpdateParams {
	            id: &device_id,
	            payload: &backend_api::models::UpdateDeviceFromAgentRequest {
	                agent_version: Some(version.to_string()),
	            },
	            token: &token.token,
	        }).await;
	        match result {
	            Ok(_) => break,
	            Err(e) => {
	                tracing::warn!("upgrade: PATCH /devices/{{id}} failed; will retry: {e}");
	                let wait = cooldown::calc(&backoff, err_streak);
	                tokio::time::sleep(Duration::from_secs(wait as u64)).await;
	                err_streak = err_streak.saturating_add(1);
	            }
	        }
	    }

	    Ok(())
	}

Wire it in `agent/src/app/mod.rs`: add `pub mod upgrade;`.

Note: the order "reset before PATCH" means a backend PATCH failure leaves on-disk state at the new version while the backend still shows the old version. The backend will catch up at the next regular sync only if some other path keeps pushing the version — but this plan deletes that path. Therefore the PATCH must be retried forever inside `ensure` (as written above), so that `ensure` returns `Ok(())` only when the backend has been told the new version. This matches the user's locked option (a) "block forever with backoff."

### 7. Boot integration

Edit `agent/src/main.rs::run_agent`. After:

	let layout = storage::Layout::default();

insert:

	let backend_default = storage::Backend::default();
	let bootstrap_http_client = match http::Client::new(&backend_default.base_url) {
	    Ok(c) => c,
	    Err(e) => { error!("upgrade: failed to construct http client: {e}"); return; }
	};
	if let Err(e) = miru_agent::app::upgrade::ensure(
	    &layout,
	    &bootstrap_http_client,
	    version::VERSION,
	).await {
	    error!("upgrade gate failed: {e}");
	    return;
	}

before the existing `if let Err(e) = storage::assert_activated(&device_file).await { ... return; }`. The existing logging guard initialization (`logs::init`) runs *after* this block today; that is acceptable because `tracing::error!` macros use the global subscriber — initialize logging *before* the upgrade gate so retry messages are visible. Move the `logs::init` call (or a minimal early-stage logging init) above the upgrade gate. Choose the option with the smallest blast radius during implementation; document the choice in Surprises & Discoveries.

### 8. Delete the old version-push path

- Delete `agent/src/sync/agent_version.rs`.
- Edit `agent/src/sync/mod.rs`: remove `pub mod agent_version;`.
- Edit `agent/src/sync/syncer.rs`:
  - Remove `agent_version,` from the `use crate::sync::{ ... };` import.
  - Remove `pub agent_version: String,` from `SyncerArgs`.
  - Remove `agent_version: String,` from `SingleThreadSyncer`.
  - Remove `agent_version: args.agent_version,` from `SingleThreadSyncer::new`.
  - Remove the `agent_version::push(...)` block from `sync_impl` (lines 238–247 in the current file).
- Edit `agent/src/app/state.rs`: drop the `agent_version` parameter from `AppState::init`'s signature and from the `SyncerArgs { ..., agent_version, ... }` field.
- Edit `agent/src/app/run.rs`: drop the `agent_version` parameter from `run`, `init`, and `init_app_state`. Update `agent/src/main.rs::run_agent` to call `run(options, await_shutdown_signal())` instead of `run(version::VERSION.to_string(), options, ...)`.
- Delete `agent/tests/sync/agent_version.rs`.
- Edit `agent/tests/sync/mod.rs`: remove `pub mod agent_version;`.

If grep reveals a non-syncer consumer of the parameter (telemetry, server state), preserve only that path and route the version through it directly rather than the SyncerArgs.

### 9. Tests

Add to `agent/tests/storage/setup.rs` a new submodule `mod reset { ... }` mirroring the existing `mod bootstrap`:

- `reset_preserves_keys_and_writes_marker`: pre-create the keypair and a `device.json` with stale schema; call `setup::reset`; assert both keys still exist with their original content, the marker file exists with the supplied version, `device.json` matches the new device, `settings.json` matches the supplied settings, `token.json` is the default `Token`, `resources/` does not exist, and `events/` is empty.
- `reset_wipes_resources_subtree`: pre-create files under `resources/config_instances/contents/` and assert they're gone after `reset`.
- `reset_when_no_prior_state`: layout is empty; `reset` succeeds and produces a complete tree.
- `reset_overwrites_existing_marker`: pre-write a marker with a different version; assert it's replaced.

Add `agent/tests/app/upgrade.rs` (and `pub mod upgrade;` in `agent/tests/app/mod.rs`):

- `ensure_is_noop_when_marker_matches`: pre-write the marker with `version`, populate keypair + device.json. Call `ensure(layout, mock_http, version)`. Assert no HTTP calls were made (zero `get_device_calls` and zero `update_device_calls` on the mock).
- `ensure_rebootstraps_when_marker_missing`: pre-populate keypair + device.json, no marker. Mock returns a fresh `Device`. After `ensure`, the marker exists with `version`, `device.json` reflects the mock response, both keys preserved, `update_device_calls` == 1.
- `ensure_rebootstraps_when_marker_version_differs`: pre-write marker with `"v0.0.1"`, run `ensure(_, _, "v0.0.2")`. Same assertions as above.
- `ensure_retries_until_get_device_succeeds`: mock returns `HTTPErr::MockErr { is_network_conn_err: true }` on first N attempts then succeeds. Use `tokio::time::pause` + `tokio::time::advance` so the retries run instantly. Assert eventual success and that the marker reflects the new version.
- `ensure_returns_uninstalled_err_when_no_device_id_resolvable`: empty layout (no `device.json`, no `token.json`). `ensure` returns `UpgradeErr::UninstalledDeviceErr`.

Stretch: `agent/tests/app/run.rs` already exists; add an end-to-end-ish test that writes a fake old-version marker, points `Layout` at a tempdir, stubs the HTTP client to return a known device, then calls the smallest possible boot harness (likely a helper that wraps `app::upgrade::ensure` + `app::run::run` with a short shutdown) and asserts the marker is updated.

The `MockClient` at `agent/tests/mocks/http_client.rs` already has `update_device_fn`; extend it with a `get_device_fn` that mirrors the same pattern. The existing `agent_version` test references `update_device_fn`; since that test is being deleted, no migration needed there.

### 10. Validation

Run `scripts/preflight.sh` from `repos/agent/`. The plan's acceptance criterion is that the script's final line is `Preflight clean`. If any of `LINT_EXIT`, `TEST_EXIT`, `TOOLS_LINT_EXIT`, `TOOLS_TEST_EXIT` is non-zero, address the failure before declaring the work complete.

## Concrete Steps

All commands run from `repos/agent/` unless otherwise noted.

1. Read fresh state of the two recently-updated files:

	$ less agent/src/storage/layout.rs agent/src/storage/setup.rs

2. Implement layout accessor + storage type:

	# edit agent/src/storage/layout.rs (add agent_version() accessor)
	# create agent/src/storage/agent_version.rs
	# edit agent/src/storage/mod.rs (add pub mod + pub use)

3. Refactor `setup.rs`. Run unit tests after each substep:

	$ cargo test -p miru_agent --test mod -- storage::setup

   Expected: `bootstrap` tests still pass; new `reset` tests fail (they don't exist yet).

4. Add `reset` tests in `agent/tests/storage/setup.rs`. Re-run:

	$ cargo test -p miru_agent --test mod -- storage::setup

   Expected: all `bootstrap` and `reset` tests pass.

5. Add `authn::issue::issue_token` free function and refactor `SingleThreadTokenManager::issue_token` to delegate. Run:

	$ cargo test -p miru_agent --test mod -- authn

   Expected: existing token-manager tests still pass.

6. Edit `api/specs/backend/v03.yaml` to add `/device` endpoint. Regen the client crate:

	$ ./api/regen.sh

   Expected: changes confined to `libs/backend-api/src/models/` (and possibly `libs/backend-api/src/lib.rs`). If the generator emits an api/operation file, include it.

7. Add `http::devices::get` in `agent/src/http/devices.rs`. Verify it compiles:

	$ cargo build -p miru_agent

8. Lift `init_device_id` out of `app::state` into `storage::device::resolve_device_id` and update both call sites.

9. Create `agent/src/app/upgrade.rs` with `ensure` and `UpgradeErr`. Add `pub mod upgrade;` to `agent/src/app/mod.rs`.

10. Wire `app::upgrade::ensure` into `main.rs::run_agent`. Move logging init above the upgrade gate.

11. Delete `agent/src/sync/agent_version.rs` and remove every reference. Update tests.

	$ grep -rn "agent_version" agent/src/ agent/tests/

   Expected: only model-layer references (`models::device::Updates::agent_version`, `models::Device::agent_version`, the `Miru-Agent-Version` HTTP header, and the new code in `app::upgrade`) remain. No `sync::agent_version`, no `SyncerArgs::agent_version`, no `run::run(agent_version, ...)`, no `agent_version.rs` test file.

12. Add tests at `agent/tests/app/upgrade.rs` and add `pub mod upgrade;` to `agent/tests/app/mod.rs`. Extend `MockClient` with `get_device_fn`. Run:

	$ cargo test -p miru_agent --test mod -- app::upgrade

   Expected: 5 tests pass.

13. Run preflight:

	$ scripts/preflight.sh

   Expected last line: `Preflight clean`. If anything fails, fix and re-run; do not move on until clean.

14. Commit the work in logical chunks (storage refactor + marker; authn helper; OpenAPI + http::devices::get; upgrade module + boot integration; delete old version-push; tests). From `repos/agent/`:

	$ git status
	$ git add -- <paths>
	$ git commit

15. Push and open the PR per `repos/agent/AGENTS.md`. Reference this plan path in the PR body.

## Validation and Acceptance

The change is accepted when ALL of the following hold:

1. From `repos/agent/`, `scripts/preflight.sh` exits 0 and prints `Preflight clean`. This is the gate the user explicitly asked for: lint, format, and the full agent test suite (including the new tests) must all be green BEFORE the changes are published. No partial-clean state is acceptable.

2. The new test `agent/tests/storage/setup.rs::reset::reset_preserves_keys_and_writes_marker` passes.

3. The new test `agent/tests/app/upgrade.rs::ensure_rebootstraps_when_marker_missing` passes and asserts the post-ensure file tree matches: `auth/private_key.pem` and `auth/public_key.pem` preserved by content, `agent_version.json` present with the supplied version, fresh `device.json`, default `settings.json`, blank `auth/token.json`, no `resources/` subtree from before the call.

4. `grep -rn "sync::agent_version\|agent_version::push" agent/src/ agent/tests/` returns no matches.

5. `grep -n "agent_version: String" agent/src/sync/syncer.rs agent/src/app/state.rs` returns no matches (or matches only on a comment explaining the field's removal — strongly prefer no matches at all).

6. `api/specs/backend/v03.yaml` contains a top-level `/device` GET with `operationId: getDevice` and the existing `Device` schema as its 200 response.

7. `agent/src/http/devices.rs` exposes `pub async fn get(client: &impl ClientI, token: &str) -> Result<Device, HTTPErr>`.

8. Manual smoke (optional, document in Surprises & Discoveries if performed): on a dev box where the agent has previously installed itself, delete `/var/lib/miru/agent_version.json` and restart the service; observe the boot logs include `upgrade: GET /device` (or analogous), then the marker is recreated and the service stabilizes. Re-restart the service: observe a no-op (no `upgrade:` log lines).

## Idempotence and Recovery

- Every step in `Concrete Steps` is re-runnable without damage. The OpenAPI regen (`api/regen.sh`) is fully idempotent — re-running it produces no diff once the spec is in its final shape.
- `setup::reset` is itself idempotent: calling it twice with the same inputs leaves the disk in the same state. `app::upgrade::ensure` is idempotent post-success: the marker file is written last (after the backend PATCH succeeds), so a crash partway through results in a missing/old marker on the next boot, which re-triggers `ensure` and converges.
- If the `api/regen.sh` step produces unexpected diffs in `libs/backend-api/src/models/`, retry with a clean working tree: `git checkout -- libs/backend-api/` and re-run.
- If `app::upgrade::ensure` deadlocks the boot (backend down forever), the recovery is to manually create `/var/lib/miru/agent_version.json` with the running version. This is a documented operational lever — note it in the PR description.
- If `setup::reset` partially completes (e.g. `device.json` written but `events/` recreate fails), the marker is not yet written, so the next boot re-enters `ensure` and re-runs `reset` from scratch. The keypair is never touched by `reset`, so identity survives every recovery path.
- The `agent_version::push` deletion is reversible only via `git revert` if a regression appears. There is no other consumer of the deleted code path on disk; rolling back the binary alone does not require restoring the source.

---

Revision notes: initial draft authored 2026-04-26 under the locked decisions enumerated in the Decision Log. No subsequent revisions yet.
