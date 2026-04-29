# Add `miru-agent reprovision` command and tighten `provision()` to be a no-op when already activated

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `feat/reprovision`, base `main`) | read-write | All source, tests, CLI, http wrapper, and plan changes for this work. |
| `libs/backend-api/` (inside this repo, generated) | read-only | The `ReprovisionDeviceRequest` model already exists from the v04 regen committed in `7bd6e46`; consume only — do not edit. |

This plan lives in `plans/backlog/` of the agent repo because every code change is in this repo.

## Purpose / Big Picture

After this change the agent has a first-class `reprovision` subcommand parallel to the existing `provision` subcommand. Operationally:

- `miru-agent reprovision --backend-host=... --mqtt-broker-host=...` reads `MIRU_PROVISIONING_TOKEN` from the environment, generates a fresh RSA keypair in a temp dir, POSTs to `/devices/reprovision`, and bootstraps on-disk state with the device returned by the backend. The backend identifies the device by the provisioning token (the device record already has a name), so the `--device-name` flag is intentionally absent.
- Re-running `miru-agent provision ...` against an already-activated machine becomes a fast no-op that returns the cached `Device` without making an HTTP call or rotating keys. That makes `provision` idempotent for callers that retry it on top of healthy state. Any partial-state condition (no keypair, missing or unparseable `device.json`) falls through to the normal full provisioning path so the box can recover.

A developer verifies success by:

1. Running `./scripts/preflight.sh` and seeing `Preflight clean`.
2. Running `./scripts/test.sh` and seeing the new tests pass:
   - `cli::reprovision_args_parse::*`
   - `cli::args_parse::parses_reprovision_subcommand` (or equivalent)
   - `provision::entry::tests::determine_reprovision_settings::*`
   - `provision::reprovision_fn::success` (integration test under `agent/tests/provision/entry.rs`)
   - `provision::provision_fn::is_noop_when_already_activated` (new idempotency test)
   - `http::devices::reprovision::success` and `http::devices::reprovision::error_propagates`
3. Spot-check that `grep -n "reprovision" agent/src/main.rs` shows the new dispatch block parallel to the provision block, and that `grep -n "/devices/reprovision" agent/src/http/devices.rs` shows the new wrapper.

User-visible behavior change is additive only: existing `provision` behavior is preserved on first install, and the new `reprovision` subcommand is gated behind an explicit verb.

## Progress

- [x] (2026-04-29) M1: Extend `cli` with `ReprovisionArgs` and `Args::reprovision_args`; route `reprovision` verb through `Args::parse`.
- [x] (2026-04-29) M2: Add `http::devices::reprovision` + `ReprovisionParams` mirroring `provision`.
- [x] (2026-04-29) M3: Tighten `provision::entry::provision()` to no-op on activated state with a cached device file.
- [x] (2026-04-29) M4: Add `provision::entry::reprovision()` plus a `determine_reprovision_settings` helper that shares a body with `determine_settings`.
- [x] (2026-04-29) M5: Wire `main.rs` dispatch + `run_reprovision` + `handle_reprovision_result`.
- [x] (2026-04-29) M6: Tests — CLI, settings determination, http wrapper, `provision()` idempotency, `reprovision()` happy-path bootstrap.
- [ ] (YYYY-MM-DD) M7: Validation — `./scripts/preflight.sh` reports `Preflight clean`.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

- 2026-04-29: On-disk `device.json` is the local `crate::models::Device` (field `device_id` via `#[serde(rename)]`), not `backend_api::models::Device` (field `id`). Reading the cached device into `backend_client::Device` as the plan literally specified would always fail because the field name and required-field shape do not match. Implemented the short-circuit by reading `crate::models::Device` and constructing a minimal `backend_client::Device { id, name, session_id, ..Default::default() }` to return.
- 2026-04-29: Renamed `provision_fn::reprovision_overwrites_existing_storage` to `provision_fn::provision_is_idempotent_on_second_call` with inverted assertions (per plan M3): asserts that the second `provision` call is a no-op, mock is not invoked, keys are byte-identical.
- 2026-04-29: The sibling existing test `provision_fn::http_error_on_reprovision_preserves_existing_storage` also broke under the new short-circuit: it ran two sequential `provision()` calls expecting the second to fail with HTTPErr, but the second now short-circuits before reaching the failing mock. Updated in place to assert idempotent no-op (Ok result, mock call_count == 0) while still verifying every persisted blob is byte-identical. The reprovision-flow failure case is now covered by the new `reprovision_fn::http_error_preserves_existing_storage` test.

## Decision Log

- 2026-04-29: Idempotency check reads the cached device via `layout.device().read_json::<crate::models::Device>()` (not `backend_client::Device` as the plan literally specified, and not `storage::resolve_device_id` / `storage::Device::spawn`). Rationale: the on-disk `device.json` schema is the local model, so the backend type cannot deserialize it. `storage::resolve_device_id` would silently fall back to the JWT in `auth/token.json` — which would let a corrupt `device.json` no-op when it should re-provision. `storage::Device::spawn` is a `ConcurrentCachedFile` actor and is heavier than needed for a one-shot read at boot. A minimal `read_json::<models::Device>` is the cheapest local signal that's also sensitive to corruption, which is the exact behavior we want. The local `Device` is then projected onto a default `backend_client::Device` populated with `id`, `name`, and `session_id` for the return value (the caller in `main.rs` only reads `device.name`).

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

### Repo conventions

- **Imports**: every source file uses three groups separated by blank lines and labelled with a comment: `// standard crates`, `// internal crates`, `// external crates`. The lint runner enforces this. See `agent/src/provision/entry.rs` for a canonical example.
- **Errors**: every error type derives `thiserror::Error` and implements `crate::errors::Error`; aggregating enums use the `crate::impl_error!` macro. No new error variants are added in this plan — `ProvisionErr` already covers `HTTPErr`, `CryptErr`, `FileSysErr`, `StorageErr`, `AuthnErr`, `LogsErr`, and `MissingEnvVarErr`. Backend errors surface verbatim through `ProvisionErr::HTTPErr`.
- **Test feature gate**: helpers and mocks are gated behind `#[cfg(feature = "test")]`. Unit tests live inside `#[cfg(test)] mod tests { ... }` in the same source file as the code under test; integration tests live under `agent/tests/<module>/...` and are reached through `RUST_LOG=off cargo test --features test`.
- **Workflow**:
  - `./scripts/test.sh` runs the agent test suite with the `test` feature on.
  - `./scripts/lint.sh` runs the custom import linter, `cargo fmt`, machete, audit, and clippy.
  - `./scripts/preflight.sh` runs lint + tests + tools-lint + tools-tests in parallel and prints `Preflight clean` on success.
- **Coverage gates**: `./scripts/covgate.sh` enforces per-module thresholds in `<module>/.covgate` files. Relevant gates for this work:
  - `agent/src/provision/.covgate` = `95.66`
  - `agent/src/cli/.covgate` = `100`
  - `agent/src/http/.covgate` = `93.61`
  Tests added in M6 should preserve or raise — never lower — these thresholds.

### Files involved

- `agent/src/cli/mod.rs` — defines `Args` and `ProvisionArgs`. Today's `Args::parse` recognizes `--version` and `provision`. No `reprovision` verb yet.
- `agent/src/main.rs` — dispatches on `cli_args.display_version` and `cli_args.provision_args`. Defines `run_provision` and `handle_provision_result`. New `reprovision` dispatch lives next to the existing `provision` dispatch.
- `agent/src/provision/mod.rs` — re-exports `entry::*` and `errors::ProvisionErr`. The new `reprovision()` entry function is re-exported by the wildcard `pub use self::entry::*`.
- `agent/src/provision/entry.rs` — owns `provision`, `read_token_from_env`, `provision_with_backend`, and `determine_settings`. The plan modifies `provision`, factors out a private settings helper, and adds `reprovision` plus `determine_reprovision_settings`.
- `agent/src/provision/errors.rs` — `ProvisionErr` aggregator. No new variants required; reuse for both flows.
- `agent/src/provision/display.rs` — color/format helpers used by both `handle_provision_result` and the new `handle_reprovision_result`.
- `agent/src/http/devices.rs` — defines `ProvisionParams`, `IssueTokenParams`, `UpdateParams`, and the corresponding `provision`, `issue_token`, `update`, `get` functions. The plan adds `ReprovisionParams` and `reprovision`.
- `agent/src/storage/setup.rs` — `bootstrap()` already wipes prior state, moves keys into `auth/`, writes `device.json`, `settings.json`, blank `auth/token.json`, the marker, and recreates `events/`. The new `reprovision()` reuses it unchanged.
- `agent/src/storage/device.rs` — `assert_activated(layout)` checks for the presence of `auth/private_key.pem` and `auth/public_key.pem`. The idempotency short-circuit in `provision()` calls this first; if it succeeds, the function then reads the device file directly via `layout.device().read_json::<backend_client::Device>().await`.
- `agent/src/storage/layout.rs` — `Layout::device()` returns the `device.json` handle.
- `libs/backend-api/src/models/reprovision_device_request.rs` — generated. Shape: `ReprovisionDeviceRequest { public_key_pem: String, agent_version: String }`. No `name` field by design.
- `libs/backend-api/src/models/mod.rs` — already re-exports `ReprovisionDeviceRequest` after the v04 regen.
- `agent/tests/provision/entry.rs` — integration tests for `provision`. New tests for `reprovision` and the new idempotency case live alongside the existing `provision_fn` module in a sibling `pub mod reprovision_fn { ... }` and `pub mod provision_fn { ... is_noop_when_already_activated }`.
- `agent/tests/cli/mod.rs` — integration tests for `cli::Args::parse` and `cli::ProvisionArgs::parse`. New `mod reprovision_args_parse` and an args-parse case for the new verb live here.
- `agent/tests/mocks/http_client.rs` — `MockClient` and `match_route`. The mock currently maps `(POST, "/devices/provision")` to `Call::ProvisionDevice`. We add a new `Call::ReprovisionDevice` variant + a `reprovision_device_fn` field + a `match_route` arm for `(POST, "/devices/reprovision")`. This is the only mock-layer change; existing tests are unaffected because the new route is not on a `starts_with` path that overlaps anything.
- `agent/tests/http/devices.rs` — integration tests for `http::devices::*`. New `pub mod reprovision { success, error_propagates }` mirrors `pub mod provision`.

### Glossary

- **Provisioning token**: an opaque bearer token set in the `MIRU_PROVISIONING_TOKEN` environment variable and posted to `/devices/provision` or `/devices/reprovision`. The backend identifies the device record by this token; for `reprovision`, no `name` is sent because the existing record already has one.
- **Bootstrap**: `storage::setup::bootstrap` — moves the freshly generated keypair from the temp dir into `auth/`, then `reset()`s all stateful files (device, settings, blank token, wiped resources, recreated events) and writes the agent-version marker.
- **Activated state**: the on-disk condition checked by `storage::assert_activated` — both `auth/private_key.pem` and `auth/public_key.pem` exist on disk. It does **not** verify those keys are still registered with the backend. The new idempotency short-circuit also requires a parseable `device.json`; together those are the cheapest local signal that the install is complete.

## M1 — Add `ReprovisionArgs` and the `reprovision` verb to `cli`

**Goal**: parse `miru-agent reprovision --backend-host=... --mqtt-broker-host=...` from the OS argv, populating a new `Option<ReprovisionArgs>` on `Args`. Silently ignore any `--device-name=...` (no field, no case branch, no warning — matches how `ProvisionArgs::parse` silently ignores unknown keys today).

**Files touched**:
- `agent/src/cli/mod.rs`

**Code shape** (signatures only):

```rust
// agent/src/cli/mod.rs
#[derive(Debug, Default)]
pub struct Args {
    pub display_version: bool,
    pub provision_args: Option<ProvisionArgs>,
    pub reprovision_args: Option<ReprovisionArgs>,
}

impl Args {
    pub fn parse(inputs: &[String]) -> Self;
}

#[derive(Debug, Default)]
pub struct ProvisionArgs { /* unchanged */ }

#[derive(Debug, Default)]
pub struct ReprovisionArgs {
    pub backend_host: Option<String>,
    pub mqtt_broker_host: Option<String>,
}

impl ReprovisionArgs {
    pub fn parse(inputs: &[String]) -> Self;
}
```

`Args::parse` gains a new `match` arm:

```rust
"reprovision" => args.reprovision_args = Some(ReprovisionArgs::parse(inputs)),
```

`ReprovisionArgs::parse` mirrors `ProvisionArgs::parse` but matches only the two host keys; the `device-name` arm is omitted entirely so the field doesn't exist on the struct. Unknown keys remain `match _ => {}` no-ops.

**Test additions** (in `agent/tests/cli/mod.rs`):

- `args_parse::parses_reprovision_subcommand_with_reprovision_args` — input `["miru-agent", "reprovision", "--backend-host=...", "--mqtt-broker-host=..."]` produces `args.reprovision_args.is_some()`, `args.provision_args.is_none()`, and the host fields populated.
- `args_parse::ignores_reprovision_options_without_reprovision_flag` — input `["miru-agent", "--backend-host=..."]` yields `reprovision_args.is_none()`.
- `args_parse::recognizes_provision_and_reprovision_independently` — sanity that the two verbs are independent (`provision` alone -> only `provision_args`; `reprovision` alone -> only `reprovision_args`).
- `reprovision_args_parse::parses_known_key_value_options` — `--backend-host=...`, `--mqtt-broker-host=...` populate, and a present `--device-name=foo` is silently ignored (compile-time evidence: `ReprovisionArgs` has no `device_name` field, so the test simply asserts the two known fields and uses no device-name accessor).
- `reprovision_args_parse::ignores_unknown_or_non_key_value_tokens` — mirror of the existing provision case.
- `reprovision_args_parse::last_duplicate_value_wins` — mirror of the existing provision case.
- `reprovision_args_parse::empty_values_are_treated_as_none` — mirror.

**Validation step**: `cargo build -p miru-agent --features test`, then `cargo test -p miru-agent --features test cli::` — expect all new and existing CLI tests pass.

## M2 — Add `http::devices::reprovision`

**Goal**: a thin wrapper around `client::fetch` that POSTs `ReprovisionDeviceRequest` to `/devices/reprovision`. The shape mirrors `http::devices::provision` exactly.

**Files touched**:
- `agent/src/http/devices.rs`

**Code shape**:

```rust
// agent/src/http/devices.rs
use backend_api::models::{
    Device, ProvisionDeviceRequest, ReprovisionDeviceRequest, TokenResponse,
    UpdateDeviceFromAgentRequest,
};

pub struct ReprovisionParams<'a> {
    pub payload: &'a ReprovisionDeviceRequest,
    pub token: &'a str,
}

pub async fn reprovision(
    client: &impl ClientI,
    params: ReprovisionParams<'_>,
) -> Result<Device, HTTPErr> {
    let url = format!("{}/devices/reprovision", client.base_url());
    let request = request::Params::post(&url, request::marshal_json(params.payload)?)
        .with_token(params.token);
    super::client::fetch(client, request).await
}
```

**Test additions** (in `agent/tests/http/devices.rs`):

Add a new `pub mod reprovision { use super::*; ... }` parallel to `pub mod provision`:

- `reprovision::success` — constructs a `ReprovisionDeviceRequest { public_key_pem: "test-pem", agent_version: "v0.0.0" }`, posts via `devices::reprovision(&mock, ReprovisionParams { ... })`, asserts `result == Device::default()`, `mock.call_count(Call::ReprovisionDevice) == 1`, and the captured request matches `{ method: POST, path: "/devices/reprovision", url: "http://mock/devices/reprovision", body: Some(<expected_json>), token: Some("test-token") }`.
- `reprovision::error_propagates` — sets `mock.reprovision_device_fn = Box::new(|| Err(mock_err()))`, runs `devices::reprovision`, asserts `Err(HTTPErr::MockErr(_))`.

**Mock plumbing** (in `agent/tests/mocks/http_client.rs`):

- Add `Call::ReprovisionDevice` to the `Call` enum.
- Add `pub reprovision_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>` to `MockClient`, defaulting to `Box::new(|| Ok(Device::default()))`.
- Add a `match_route` arm: `(m, p) if *m == Method::POST && p == "/devices/reprovision" => Call::ReprovisionDevice,` placed adjacent to the existing `/devices/provision` arm so a code reviewer sees them together.
- Add a `handle_route` arm: `Call::ReprovisionDevice => json(&(self.reprovision_device_fn)()?),`.

The mock changes are additive and do not affect existing tests.

**Validation step**: `cargo test -p miru-agent --features test http::devices::reprovision::` — expect both new tests pass; rerun the full `http::devices::` suite to confirm no regression.

## M3 — Tighten `provision()` to no-op when the same machine is already activated

**Goal**: at the top of `provision()`, before generating any keypair or making any HTTP call, short-circuit if the machine is already fully activated AND the device file is parseable. Any partial state (missing/unparseable device file, missing keys) falls through to the existing flow.

**Files touched**:
- `agent/src/provision/entry.rs`

**Code shape**:

```rust
pub async fn provision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
    device_name: Option<String>,
) -> Result<backend_client::Device, ProvisionErr> {
    // Idempotency short-circuit: if the machine is fully activated AND the
    // cached device file is readable, return that device unchanged. Any
    // partial state (missing keys, unparseable device.json) falls through to
    // the full provisioning flow so the box can recover.
    if storage::assert_activated(layout).await.is_ok() {
        if let Ok(device) = layout.device().read_json::<backend_client::Device>().await {
            return Ok(device);
        }
    }

    // ...existing body unchanged: temp dir, gen_key_pair, provision_with_backend,
    // bootstrap, temp_dir cleanup...
}
```

The short-circuit uses `assert_activated` (which already validates both keys exist) and a direct `read_json` on `Layout::device()`. It deliberately does **not** call `storage::resolve_device_id` because that helper falls back to the JWT in `auth/token.json`, which would let a corrupt-`device.json` install no-op when it should re-provision.

After implementation, capture in **Decision Log**: why the cached-device read goes through `layout.device().read_json::<backend_client::Device>()` rather than another path (the value we actually want to return is the full backend `Device`, which `device.json` already stores; `storage::Device` is a `ConcurrentCachedFile` actor and is heavier than needed for a one-shot read at boot).

**Test additions** (in `agent/tests/provision/entry.rs`, inside `pub mod provision_fn`):

- `provision_fn::is_noop_when_already_activated` —
  1. Set up a tempdir layout, run a successful `provision::provision` once with `MockClient` returning a known `Device`.
  2. Construct a second `MockClient` whose `provision_device_fn` returns `Err(HTTPErr::MockErr { is_network_conn_err: true })` (a poison-pill that will fail any unexpected call).
  3. Call `provision::provision(&mock_poison, &layout, &settings, &token, Some("ignored".into()))`.
  4. Assert `Ok(device)` is returned, the device id matches the originally-provisioned id, `mock_poison.call_count(Call::ProvisionDevice) == 0`, and the on-disk private key bytes are byte-identical to the post-first-provision bytes (i.e. no key rotation occurred).
- `provision_fn::falls_through_when_keys_missing` — pre-create only `device.json` (no keys); call `provision`; assert it follows the full flow (mock provision call count == 1, keys appear on disk).
- `provision_fn::falls_through_when_device_file_corrupt` — pre-create both keys and a `device.json` containing garbage bytes; call `provision`; assert full flow (provision call count == 1, the corrupt file is overwritten).

These three new cases sit inside the existing `provision_fn` module alongside `success`, `http_error_aborts_provision`, `reprovision_overwrites_existing_storage`, and `http_error_on_reprovision_preserves_existing_storage`. Note: the existing `reprovision_overwrites_existing_storage` test name dates from before this feature; do **not** rename it — it documents what happened when `provision` was called twice in sequence under the old behavior. After this change that test will fail because the second `provision` call is now a no-op and never re-rotates keys. **Update the test**: rename it to `provision_is_idempotent_on_second_call` and assert the opposite — the on-disk keys are byte-identical after the second call and the second mock's `ProvisionDevice` count is 0. Capture this rename in **Surprises & Discoveries** during implementation.

**Validation step**: `cargo test -p miru-agent --features test provision::` — expect all four `provision_fn` cases (the original three plus `is_noop_when_already_activated`, and the renamed test) pass.

## M4 — Add `provision::reprovision()` plus shared settings helper

**Goal**: a public `reprovision()` entry function that mirrors `provision()` minus the device-name argument and the idempotency short-circuit (reprovision is always meant to run the full workflow). A parallel `determine_reprovision_settings()` helper shares a private body with the existing `determine_settings()`.

**Files touched**:
- `agent/src/provision/entry.rs`

**Code shape**:

```rust
pub async fn reprovision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
) -> Result<backend_client::Device, ProvisionErr> {
    let temp_dir = layout.temp_dir();

    let result = async {
        let private_key_file = temp_dir.file("private.key");
        let public_key_file = temp_dir.file("public.key");
        rsa::gen_key_pair(4096, &private_key_file, &public_key_file, Overwrite::Allow).await?;

        let device = reprovision_with_backend(http_client, &public_key_file, token).await?;
        storage::setup::bootstrap(
            layout,
            &(&device).into(),
            settings,
            &private_key_file,
            &public_key_file,
            version::VERSION,
        )
        .await?;
        Ok(device)
    }
    .await;

    if let Err(e) = temp_dir.delete().await {
        debug_assert!(false, "failed to clean up temp dir: {e}");
        warn!("failed to clean up temp dir: {e}");
    }
    result
}

async fn reprovision_with_backend<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    public_key_file: &filesys::File,
    token: &str,
) -> Result<backend_client::Device, ProvisionErr> {
    let public_key_pem = public_key_file.read_string().await?;
    let payload = backend_client::ReprovisionDeviceRequest {
        public_key_pem,
        agent_version: version::VERSION.to_string(),
    };
    let params = http::devices::ReprovisionParams { payload: &payload, token };
    Ok(http::devices::reprovision(http_client, params).await?)
}

// Shared settings body. Private; both public wrappers delegate.
fn build_settings(
    backend_host: Option<&str>,
    mqtt_broker_host: Option<&str>,
) -> settings::Settings {
    let mut settings = settings::Settings::default();
    if let Some(host) = backend_host {
        settings.backend.base_url = format!("{}/agent/v1", host);
    }
    if let Some(host) = mqtt_broker_host {
        settings.mqtt_broker.host = host.to_string();
    }
    settings
}

pub fn determine_settings(args: &cli::ProvisionArgs) -> settings::Settings {
    build_settings(args.backend_host.as_deref(), args.mqtt_broker_host.as_deref())
}

pub fn determine_reprovision_settings(args: &cli::ReprovisionArgs) -> settings::Settings {
    build_settings(args.backend_host.as_deref(), args.mqtt_broker_host.as_deref())
}
```

`reprovision()` uses the same temp-dir pattern as `provision()` so a partial failure (HTTP error after key gen) leaves the existing on-disk install untouched and the temp dir cleaned up. The bootstrap step is the same `storage::setup::bootstrap` call that `provision()` uses, which is locked in scope per the task description.

`backend_api::models::ReprovisionDeviceRequest` is added to the `use backend_api::models as backend_client;` consumers — it is reachable via `backend_client::ReprovisionDeviceRequest` without changing the import.

**Test additions** (in `agent/src/provision/entry.rs` `#[cfg(test)] mod tests`):

Inside the existing `mod determine_settings` add a sibling `mod determine_reprovision_settings`:

- `backend_host_appends_agent_v1_suffix` — given `ReprovisionArgs { backend_host: Some("https://custom.example.com"), .. }`, returns settings with `backend.base_url == "https://custom.example.com/agent/v1"`.
- `mqtt_broker_host_override` — given `ReprovisionArgs { mqtt_broker_host: Some("mqtt.custom.example.com"), .. }`, returns settings with `mqtt_broker.host == "mqtt.custom.example.com"`.
- `no_overrides_preserves_defaults` — `ReprovisionArgs::default()` returns a settings struct identical to `Settings::default()` for the two relevant fields.

These three cases mirror the existing `mod determine_settings` exactly.

**Test additions for the integration flow** (in `agent/tests/provision/entry.rs`, new module `pub mod reprovision_fn`):

- `reprovision_fn::success` —
  1. Build a tempdir layout, default `Settings`, a fresh JWT-shape provisioning token (use the existing `new_jwt(DEVICE_ID)` helper).
  2. Construct `MockClient { reprovision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "after-reprovision"))), ..MockClient::default() }`.
  3. Call `provision::reprovision(&mock, &layout, &settings, &token).await.unwrap()`.
  4. Assert `device.id == DEVICE_ID`, `device.name == "after-reprovision"`.
  5. Assert `device.json` exists and parses, contains `device_id == DEVICE_ID` and `name == "after-reprovision"`.
  6. Assert `settings.json` exists.
  7. Assert `auth/private_key.pem`, `auth/public_key.pem`, and `auth/token.json` all exist.
  8. Assert `temp_dir` has been cleaned up.
  9. Assert `mock.call_count(Call::ReprovisionDevice) == 1`, `mock.call_count(Call::ProvisionDevice) == 0`.
  10. Inspect `mock.requests()` and verify the captured request's `body` deserializes (via `serde_json::from_str::<serde_json::Value>`) to a JSON object that contains `public_key_pem` and `agent_version` keys but **no** `name` key — that is the load-bearing wire-format invariant for this feature.
- `reprovision_fn::http_error_preserves_existing_storage` — pre-provision the box, capture every persisted blob byte-for-byte, run a `reprovision` against a mock whose `reprovision_device_fn` returns `Err(HTTPErr::MockErr { is_network_conn_err: true })`, then assert every captured blob is byte-identical and `temp_dir` is gone. Mirrors the existing `http_error_on_reprovision_preserves_existing_storage` test for `provision`.
- `reprovision_fn::rotates_keypair` — pre-provision the box, capture the private-key bytes, run a successful reprovision, assert the new private-key bytes differ from the captured bytes. The reprovision flow always rotates keys because it always runs the full bootstrap.

**Validation step**: `cargo test -p miru-agent --features test provision::` — expect all `provision_fn` and `reprovision_fn` cases pass; `cargo test -p miru-agent --features test --lib provision::entry::tests::determine_reprovision_settings` — expect the three new unit tests pass.

## M5 — Wire `main.rs` dispatch

**Goal**: dispatch on `cli_args.reprovision_args` parallel to `provision_args`. Add `run_reprovision(args)` and `handle_reprovision_result(result)` parallel to the existing `run_provision`/`handle_provision_result` pair. Success message: `"Successfully reprovisioned this device as <name>!"` with the name colored green via `display::format_info` and `display::color(_, Colors::Green)`.

**Files touched**:
- `agent/src/main.rs`

**Code shape**:

After the existing block:

```rust
    if let Some(provision_args) = cli_args.provision_args {
        let result = run_provision(provision_args).await;
        handle_provision_result(result);
        return;
    }
```

add:

```rust
    if let Some(reprovision_args) = cli_args.reprovision_args {
        let result = run_reprovision(reprovision_args).await;
        handle_reprovision_result(result);
        return;
    }
```

The two new functions mirror the provision pair:

```rust
async fn run_reprovision(args: cli::ReprovisionArgs) -> Result<backend_client::Device, ProvisionErr> {
    let tmp_dir = Dir::create_temp_dir("miru-agent-reprovision-logs").await?;
    let options = logs::Options {
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let _guard = logs::init(options)?;

    let settings = provision::determine_reprovision_settings(&args);
    let http_client = http::Client::new(&settings.backend.base_url)?;
    let layout = storage::Layout::default();
    let token = provision::read_token_from_env()?;

    let result = provision::reprovision(&http_client, &layout, &settings, &token).await;

    drop(_guard);
    if let Err(e) = tmp_dir.delete().await {
        eprintln!("failed to clean up reprovision log dir: {e}");
    }

    result
}

fn handle_reprovision_result(result: Result<backend_client::Device, ProvisionErr>) {
    match result {
        Ok(device) => {
            let msg = format!(
                "Successfully reprovisioned this device as {}!",
                display::color(&device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Err(e) => {
            error!("Reprovisioning failed: {:?}", e);
            println!("An error occurred during reprovisioning. Contact us at ben@mirurobotics.com for immediate support.\n\nError: {e}\n");
            std::process::exit(1);
        }
    }
}
```

**Test additions**: `main.rs` has no direct test coverage today (it's the binary entry). The dispatch logic itself is exercised end-to-end through the CLI tests in M1 plus the `provision::reprovision` integration tests in M4. No new tests are needed here; the implementor verifies behavior by running `miru-agent reprovision --help`-equivalent flag combinations on a dev box only if a smoke test is desired (note in **Surprises & Discoveries** if performed).

**Validation step**: `cargo build -p miru-agent --features test` — expect a clean build. `cargo build -p miru-agent` (without the `test` feature) — expect a clean build, since `main.rs` is part of the production binary.

## M6 — Tests already covered in M1–M5, plus integration sanity

This milestone is a checkpoint. By the end of M5 the following tests exist:

- M1: 6 new CLI tests under `agent/tests/cli/mod.rs` (`args_parse::*` and `reprovision_args_parse::*`).
- M2: 2 new http tests under `agent/tests/http/devices.rs` (`reprovision::success`, `reprovision::error_propagates`) and one new `Call` variant + `match_route` arm + `handle_route` arm + mock field in `agent/tests/mocks/http_client.rs`.
- M3: 1 new + 2 new `provision_fn` tests under `agent/tests/provision/entry.rs` (`is_noop_when_already_activated`, `falls_through_when_keys_missing`, `falls_through_when_device_file_corrupt`); 1 renamed test (`reprovision_overwrites_existing_storage` -> `provision_is_idempotent_on_second_call` with inverted assertions).
- M4: 3 new unit tests under `agent/src/provision/entry.rs` `mod determine_reprovision_settings`; 3 new integration tests under `agent/tests/provision/entry.rs` `pub mod reprovision_fn` (`success`, `http_error_preserves_existing_storage`, `rotates_keypair`).

Run `./scripts/test.sh` and confirm all pass. If the covgate file under `agent/src/provision/.covgate` (`95.66`) drops, add coverage for any uncovered branch in `reprovision`, `reprovision_with_backend`, `build_settings`, `determine_reprovision_settings`, or the `provision()` short-circuit. **Do not lower the threshold.** Same rule for `agent/src/cli/.covgate` (`100`) and `agent/src/http/.covgate` (`93.61`).

If the `100` threshold under `cli` is at risk because the new `reprovision` arm in `Args::parse` has an untested edge, add a CLI test to cover it (the listed M1 cases should already cover all four arms of the verb match: `version`, `provision`, `reprovision`, `_ => {}`).

**Validation step**: `./scripts/test.sh` reports all tests passing; `./scripts/covgate.sh` reports all modules above threshold.

## M7 — Validation

Run `./scripts/preflight.sh` from the repo root.

> **Validation gate**: `./scripts/preflight.sh` MUST report `Preflight clean` before changes are pushed or a PR is opened.

If preflight fails:

- **Lint failures** (import order, fmt, clippy): fix per the conventions documented in `agent/AGENTS.md`. Most likely culprits — import order in the modified `agent/src/cli/mod.rs`, `agent/src/http/devices.rs`, `agent/src/provision/entry.rs`, and `agent/src/main.rs`; `cargo fmt -p miru-agent` will resolve formatting; clippy `-D warnings` may flag an unused import if `ReprovisionDeviceRequest` was added without being referenced (it is, by `reprovision_with_backend`).
- **Tests**: read the failure. The most likely failure is the renamed `reprovision_overwrites_existing_storage` test if the implementor forgot to invert its assertions after the M3 idempotency change.
- **Coverage**: add tests; never lower a threshold (see M6).
- **Tools lint / tools tests**: typically unaffected by this change. If they fail, investigate whether a generated file under `libs/backend-api/` was inadvertently touched — the v04 regen is already committed and the agent code only consumes the new model.

Re-run `./scripts/preflight.sh` until the final line is `Preflight clean`.

## Validation and Acceptance

The change is accepted when ALL of the following hold:

1. `./scripts/preflight.sh` exits 0 and prints `Preflight clean`.
2. `cargo test -p miru-agent --features test cli::` runs the new and existing CLI tests and all pass.
3. `cargo test -p miru-agent --features test http::devices::` includes the new `reprovision::*` cases and all pass.
4. `cargo test -p miru-agent --features test provision::` includes the renamed `provision_fn::provision_is_idempotent_on_second_call`, the three new `provision_fn::*` short-circuit cases, and the three new `reprovision_fn::*` cases; all pass.
5. `grep -n 'pub fn parse' agent/src/cli/mod.rs` shows two `parse` impls (`Args::parse`, `ProvisionArgs::parse`, `ReprovisionArgs::parse`).
6. `grep -n 'pub async fn reprovision' agent/src/http/devices.rs agent/src/provision/entry.rs` shows the wrapper and the entry function.
7. `grep -n 'reprovision_args' agent/src/main.rs` shows the dispatch block parallel to `provision_args`.
8. The wire-format invariant test (`reprovision_fn::success` step 10) passes — the captured POST body contains no `name` field.
9. No `.covgate` file is modified.

## Idempotence and Recovery

- All edits are pure source/test changes; rerunning steps re-edits the same file content.
- The new mock plumbing in `agent/tests/mocks/http_client.rs` is additive; existing tests are unaffected because the new route does not overlap any existing `match_route` arm.
- The `provision()` short-circuit is conservative: it only triggers when `assert_activated` succeeds AND `device.json` is parseable. Any partial state falls through to the full flow, so a half-installed box always recovers via the existing path.
- The `reprovision()` flow uses the same temp-dir pattern as `provision()`. Failures after key generation but before bootstrap leave the existing install untouched. The temp dir is cleaned up unconditionally.
- The renamed `reprovision_overwrites_existing_storage` -> `provision_is_idempotent_on_second_call` test is a behavior change, not a regression; the old name described the pre-change behavior of `provision`, the new name describes the post-change behavior. Capture the rename in **Surprises & Discoveries**.
