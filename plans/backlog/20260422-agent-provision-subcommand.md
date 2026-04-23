# `miru-agent provision` Subcommand

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent` (this repo) | read-write | All changes land here: `agent/src/cli/`, `agent/src/main.rs`, `agent/src/http/`, `agent/src/installer/`, `agent/tests/`, `agent/README.md`, `agent/ARCHITECTURE.md`. |
| `backend` | read-only | Source of truth for the `POST /v1/devices`, `GET /v1/devices?name=…`, and `POST /v1/devices/{id}/activation_token` endpoints; we follow the same shapes that `scripts/install/provision.sh` already proves out. |

This plan lives in `agent/plans/` because all code changes are made inside the `agent` repo.

## Purpose / Big Picture

Today a customer provisions a device by running `scripts/install/provision.sh`, which curls the backend twice (create-or-fetch device, then issue an activation token), then shells into the agent with `--install` and a `MIRU_ACTIVATION_TOKEN` env var. After this change, the customer can do the same thing using nothing but the `miru-agent` binary:

    export MIRU_API_KEY=<api-key>
    sudo -E miru-agent provision --device-name=$HOSTNAME --allow-reactivation=false

Observable behavior:

- Reads `MIRU_API_KEY` from the environment.
- Calls the backend to create-or-fetch the device by name (POST → on 409, GET).
- Requests an activation token (`POST /v1/devices/{id}/activation_token`).
- Runs the existing local install flow (the same code path `--install` already uses) to write `/srv/miru/{device.json,settings.json,auth/...}`.
- Stops the `miru` systemd unit before install if running, restarts after.
- Prints a clear success message; on failure exits with a structured exit code (see Decision Log) so customer scripts can branch on the "device already activated, reactivation not allowed" case.

**Out of scope.**

- The APT publishing pipeline (how the new binary gets onto the customer's machine) — tracked separately.
- Removal or deprecation of `scripts/install/provision.sh` — both flows coexist until the next deprecation cycle.
- Migration of existing already-activated devices.
- Backend OpenAPI regeneration for the new `CreateDeviceRequest` type — a local request type is used inside `agent/src/http/devices.rs` instead.
- HTTP retry / backoff logic for the new provision calls — single-attempt, fail-loud; operators re-run on transient error.

## Progress

- [ ] Milestone 1: CLI plumbing — `ProvisionArgs` parsed, `main.rs` dispatches, exit-code constants exist, stub returns "not implemented".
- [ ] Milestone 2: HTTP layer — `X-API-Key` header support in `http/request.rs`, `create_or_fetch_device()` + `CreateDeviceRequest` in `http/devices.rs`, `MockClient` field added, unit tests pass.
- [ ] Milestone 3: Provision flow — `installer/provision.rs` implements end-to-end orchestration, wired into CLI, unit tests pass.
- [ ] Milestone 4: Systemd handling and root-privilege check.
- [ ] Milestone 5: Docs — `ARCHITECTURE.md` and `README.md` updated.
- [ ] Milestone 6: Manual smoke test against staging.

One commit per milestone.

## Surprises & Discoveries

Add entries as work proceeds.

## Decision Log

All entries below recorded 2026-04-22 by plan author.

- **CLI shape (M1).** A new top-level `provision` subcommand (sibling to `--install`), not a `--provision` flag. Matches customer-facing UX in the published example and parses cleanly as a positional verb in the existing custom parser.
- **Module location (M1/M3).** `agent/src/installer/provision.rs`, sibling to `install.rs`. Shares the device-onboarding domain; a top-level `agent/src/provision.rs` was rejected to keep onboarding code colocated.
- **Local request type (M2).** `CreateDeviceRequest { name }` defined locally in `agent/src/http/devices.rs` rather than regenerating the OpenAPI client. OpenAPI regeneration is out of scope; long-term it should move into `libs/backend-api` (TODO comment at the type definition).
- **Create-or-fetch placement (M2).** The POST + GET-on-409 fallback is collapsed into a single HTTP-layer function (`create_or_fetch_device`) so callers see one operation, matching `provision.sh`'s presentation and keeping the orchestrator thin.
- **`X-API-Key` plumbing (M2).** Add `api_key: Option<&str>` to `request::Params` (extends the existing token-bearing surface) rather than a new auth-mode enum. Mirrors the existing `with_token` pattern.
- **`issue_token` boundary (M2).** Add a new `issue_activation_token()` targeting `/devices/{id}/activation_token`; do NOT modify the existing `issue_token` (which targets `/devices/{id}/issue_token` and has live callers in `authn/token_mngr.rs` and elsewhere).
- **`determine_settings` refactor (M3).** Refactor `installer::install::determine_settings` so it accepts the `(backend_host, mqtt_broker_host)` inputs directly (e.g. `determine_settings_from(Option<&str>, Option<&str>)`), allowing `run_provision` to build settings once and reuse them across both HTTP clients. Removes duplication and keeps the install path unchanged.
- **Default backend host (M3).** Introduce `pub const DEFAULT_BACKEND_HOST: &str = "https://api.mirurobotics.com";` in `agent/src/installer/install.rs` and derive `Settings::default().backend.base_url` from it (`format!("{}/agent/v1", DEFAULT_BACKEND_HOST)`). `determine_settings_from` falls back to the constant when `backend_host` is `None`; `run_provision` reuses it as `unwrap_or(installer::install::DEFAULT_BACKEND_HOST)` when building the public-API client. One source of truth for the host.
- **Logging-init helper (M3).** Extract the existing 25-line stdout-off + temp-dir logging setup from `main.rs::run_installer` (lines 40–64) into `agent/src/installer/mod.rs::init_installer_logging() -> Result<(logs::Guard, filesys::Dir), InstallErr>`. Both `run_installer` and `run_provision` call it. Add `pub type Guard = tracing_appender::non_blocking::WorkerGuard;` to `agent/src/logs/mod.rs` so the helper signature reads naturally; `logs::init` returns the underlying `WorkerGuard` today. Avoids copy/paste drift.
- **Two backend base URLs (M3).** `provision` constructs **two** `http::Client` instances: a public-API client at `{backend_host}/v1` for the create-or-fetch and activation-token calls, and the existing agent-API client at `{backend_host}/agent/v1` for `installer::install()`. The public API and agent API live at distinct prefixes (verified in `provision.sh`).
- **Privilege check (M4).** Add `libc = "0.2"` to `agent/Cargo.toml` `[dependencies]`. `assert_root()` returns `NotRootErr` when `unsafe { libc::geteuid() } != 0`. `nix` is not currently a dependency (verified); `libc` is smaller and the call is trivial.
- **Systemd policy (M4).** Always attempt the operation (`systemctl stop miru` before install, `systemctl restart miru` after install); treat exit code 5 (`Unit miru.service not loaded`) as a no-op; any other non-zero exit returns `SystemdErr`. `systemctl is-active` returns non-zero for both "unit not loaded" and "unit loaded but inactive", which would incorrectly skip restart on a stopped-but-installed unit (e.g. after a manual stop). Single-attempt with exit-code matching is simpler and correctly handles the recovery scenario.
- **Backend "device already activated" error code (M3).** The string is `device_is_active` (verified in `backend/internal/configs/domain/devices/errors.go::IsActiveCode`, raised inside `backend/internal/configs/services/devices/activation_token.go::IssueActivationToken` via `dvcdmn.VerifyInactive`). Branch on `HTTPErr::RequestFailed` whose `error.code == "device_is_active"` and return `ProvisionErr::ReactivationNotAllowedErr`.
- **Exit codes.** Authoritative mapping (encoded in `agent/src/cli/exit_codes.rs`):

  | Exit | Constant | Scenario |
  |------|----------|----------|
  | 0 | `SUCCESS` | Provision succeeded. |
  | 1 | `GENERIC_FAILURE` | CLI parse failure, missing `--device-name`, non-root caller, configuration/programming errors (URL parse, `http::Client::new` failure, logging init failure). |
  | 2 | `MISSING_API_KEY` | `MIRU_API_KEY` env var unset. |
  | 3 | `BACKEND_ERROR` | An actual HTTP failure (network down, 5xx, timeout) on a provision call. Reserved exclusively for HTTP failures — configuration errors map to 1. |
  | 4 | `REACTIVATION_NOT_ALLOWED` | Backend returned `device_is_active` while `--allow-reactivation=false`. |
  | 5 | `INSTALL_FAILURE` | Inner `installer::install::install()` returned `InstallErr` (RSA keygen failure, `/srv/miru` write failure, `register_with_backend` activation failure, etc.) or systemd stop/restart failure. |

  Rationale for the 1-vs-3 split: exit 3 is reserved for actual HTTP failures so customer scripts can confidently retry on 3; configuration/programming errors deserve a different signal and map to exit 1.

## Outcomes & Retrospective

Fill in at the end of each milestone and again at completion.

## Context and Orientation

A reader who has never seen this repo before should be able to find their feet from this section alone.

**Repository layout (relevant parts).**

- `agent/src/main.rs` — binary entrypoint. Lines 22–37 dispatch on parsed CLI args: `display_version` first, then `install_args` (runs `run_installer()` and exits via `handle_install_result()`), then falls through to `run_agent()`. New provision dispatch lands here.
- `agent/src/cli/mod.rs` — custom CLI parser (no clap). Defines `Args { display_version, install_args }` and `InstallArgs { backend_host, mqtt_broker_host, device_name }`. Args split on `=` after stripping leading dashes. New `ProvisionArgs` and a `provision_args: Option<ProvisionArgs>` field on `Args` go here.
- `agent/src/installer/install.rs` — `install()` async function (lines 18–53) that drives RSA keygen, calls `register_with_backend()` (POSTs to `/devices/{id}/activate`), and bootstraps `/srv/miru/`. Reads activation JWT from `MIRU_ACTIVATION_TOKEN` via `read_token_from_env()` (lines 55–68). Signature:

      pub async fn install<HTTPClientT: http::ClientI>(
          http_client: &HTTPClientT,
          layout: &storage::Layout,
          settings: &settings::Settings,
          token: &str,
          device_name: Option<String>,
      ) -> Result<backend_client::Device, InstallErr>

  Also defines `determine_settings(args: &cli::InstallArgs) -> settings::Settings` which the provision flow reuses (input adapter required — see Plan of Work).

- `agent/src/installer/errors.rs` — `InstallErr` enum (variants: `MissingEnvVarErr`, `AuthnErr`, `CryptErr`, `FileSysErr`, `HTTPErr`, `StorageErr`) and the `MissingEnvVarErr { name, trace }` struct. `crate::impl_error!` registers the enum.
- `agent/src/installer/mod.rs` — module file; needs a new `pub mod provision;` line.
- `agent/src/installer/display.rs` — `format_info()` and `color()` helpers used by `main.rs::handle_install_result`. The provision result handler reuses these.
- `agent/src/http/devices.rs` — three existing functions (`activate`, `issue_token`, `update`) and three matching `*Params<'a>` structs. Uses `request::Params::post(...).with_token(...)` for bearer-auth POSTs. **No** create-or-fetch function exists; **no** API-key header support exists.
- `agent/src/http/request.rs` — `Params<'a>` struct (lines 20–28) carries `token: Option<&'a str>`; `build()` (lines 201–234) calls `add_token_to_headers()` (lines 236–248) which sets `Authorization: Bearer <token>`. Default headers (`Headers::to_map`, lines 162–172) set `Miru-Version`, `Miru-Agent-Version`, etc., but **no** `X-API-Key`.
- `agent/src/http/errors.rs` — `HTTPErr` variants: `RequestFailed`, `TimeoutErr`, `ReqwestErr`, `BuildReqwestErr`, `InvalidURLErr`, `InvalidHeaderValueErr`, `MarshalJSONErr`, `UnmarshalJSONErr`, `MockErr`. Backend errors deserialize into `backend_api::models::ErrorResponse { error: { code, message, params } }` — `RequestFailed` already carries an `Option<ErrorResponse>`, so the provision flow can branch on `error.code`.
- `agent/tests/installer/install.rs` — pattern for installer tests: `#[tokio::test]`, `MockClient` from `agent/tests/mocks/http_client.rs` (closure fields per endpoint), filesystem isolation via `filesys::Dir::create_temp_dir("install-test")`. New `agent/tests/provision/` mirrors this pattern.
- `agent/tests/mocks/http_client.rs` — `MockClient` with `activate_device_fn`, `issue_device_token_fn`, etc. closures. New `create_or_fetch_device_fn` and `issue_activation_token_with_api_key_fn` get added here.
- `agent/tests/mod.rs` — registers test modules; new `provision` module added.
- `libs/backend-api/src/models/` — generated OpenAPI types: `Device`, `ActivateDeviceRequest`, `IssueDeviceTokenRequest` (`{ claims, signature }` — for the agent-side `/issue_token`, NOT the public `/activation_token`), `TokenResponse`, `UpdateDeviceFromAgentRequest`, `ErrorResponse`. **No** request type for `POST /v1/devices` or `POST /v1/devices/{id}/activation_token`; the plan defines local `CreateDeviceRequest` and `IssueActivationTokenRequest { allow_reactivation: bool }` in `agent/src/http/devices.rs`.
- `scripts/install/provision.sh` — existing shell-based provisioner. **Read-only for this plan**.
- `agent/ARCHITECTURE.md` — lines 8–12 and 79–81 describe "Installer mode (--install)" and "Device setup". Updated in M5.
- `agent/README.md` — does not currently mention provision; gets a new section.
- `agent/AGENTS.md` — repo-specific conventions (import ordering, error idioms). No edits needed.

**Toolchain (working dir for all commands: `/home/ben/miru/workbench4/repos/agent/`).**

- Build: `cargo build -p miru-agent` (debug) or `cargo build --release -p miru-agent`.
- Tests: `./scripts/test.sh` (runs `RUST_LOG=off cargo test --features test -- --test-threads=1`).
- Lint: `./scripts/lint.sh` (custom import linter + `cargo fmt -p miru-agent -- --check` + `cargo clippy --package miru-agent --all-features -- -D warnings` + `cargo machete` + `cargo audit`).
- Format: `cargo fmt -p miru-agent`.
- Coverage: `./scripts/covgate.sh`.
- Refresh deps: `./scripts/update-deps.sh`.
- Preflight: `./scripts/preflight.sh` (runs lint + tests in parallel; must end with `Preflight clean`).

**Terms.**

- *Activation token*: short-lived JWT issued by the backend for a specific device id. Consumed by `POST /v1/devices/{id}/activate` together with the device's freshly-generated public key. Today read from `MIRU_ACTIVATION_TOKEN`.
- *API key*: long-lived secret scoped to a customer organization. Sent as `X-API-Key: <key>` on the device-create and token-issue calls. Today only `provision.sh` uses it.
- *Provision*: (a) creating-or-fetching a device row by name on the backend and (b) activating that row by uploading a public key to receive durable credentials.
- *Reactivation*: requesting a new activation token for a device that has previously been activated. The backend rejects unless `allow_reactivation: true`.
- *Layout* (`storage::Layout`): the `/srv/miru/...` filesystem layout. `Layout::default()` is production paths; tests build a layout under a temp dir.

**Backend endpoints used (verified against `scripts/install/provision.sh`).**

Two distinct base URLs are in play. The provision module constructs **two** separate `http::Client` instances:
- A public-API client whose `base_url()` returns `format!("{}/v1", backend_host)` — used for the create-or-fetch and activation-token calls.
- An agent-API client whose `base_url()` returns `format!("{}/agent/v1", backend_host)` — used by the inner `installer::install()` call.

- `POST {backend_host}/v1/devices` (public API)
  Headers: `X-API-Key: $MIRU_API_KEY`, `Content-Type: application/json`, `Miru-Version: 2026-03-09.tetons`.
  Body: `{"name": "<device-name>"}`.
  Responses: `200`/`201` → `Device`. `409 Conflict` → device exists; caller `GET /v1/devices?name=<device-name>` (same `X-API-Key`) to retrieve it.

- `POST {backend_host}/v1/devices/{device_id}/activation_token` (public API)
  Headers: `X-API-Key: $MIRU_API_KEY`.
  Body: `{"allow_reactivation": <bool>}`.
  Response: `{"token": "<jwt>", "expires_at": "..."}`. Backend returns `error.code == "device_is_active"` (HTTP 400) when previously activated and `allow_reactivation=false`.

- `POST {backend_host}/agent/v1/devices/{device_id}/activate` (agent API)
  Existing agent-issued call performed by `installer::install()`. Bearer-auth with the activation JWT. Untouched by this plan.

The existing `http::devices::issue_token()` posts to a different agent-API path (`/devices/{id}/issue_token`) and is referenced by `agent/src/authn/token_mngr.rs` and other call sites. It must NOT be modified; the new `issue_activation_token()` is a separate function.

## Plan of Work

Six milestones; each ends with one commit.

### Milestone 1 — CLI plumbing

Goal: `miru-agent provision --device-name=foo --allow-reactivation=false` parses, dispatches to a stub printing "not implemented", exits 1. Compiles, lints, existing tests pass.

Edits:

1. `agent/src/cli/mod.rs` — add `provision_args: Option<ProvisionArgs>` to `Args`. Recognize `provision` in the top-level loop (after `trim_start_matches('-')`) so it sets `provision_args = Some(ProvisionArgs::parse(inputs))`. Mirror the `--install` shape: `provision` is a positional verb, the rest are `--key=value`. Define:

       #[derive(Debug, Default)]
       pub struct ProvisionArgs {
           pub backend_host: Option<String>,
           pub mqtt_broker_host: Option<String>,
           pub device_name: Option<String>,
           pub allow_reactivation: bool,  // default false
       }

   `allow_reactivation` parses from `--allow-reactivation=true|false` (default `false`). Unit tests: required `--device-name` parsed; `=true` and `=false`; unknown flags ignored; missing `--device-name` produces `device_name = None` (validation later).

2. `agent/src/cli/exit_codes.rs` (NEW) — define exit constants:

       pub const SUCCESS: i32 = 0;
       pub const GENERIC_FAILURE: i32 = 1;
       pub const MISSING_API_KEY: i32 = 2;
       pub const BACKEND_ERROR: i32 = 3;
       pub const REACTIVATION_NOT_ALLOWED: i32 = 4;
       pub const INSTALL_FAILURE: i32 = 5;

   Add `pub mod exit_codes;` to `agent/src/cli/mod.rs`.

3. `agent/src/main.rs` — add a third dispatch branch between the install branch and `run_agent()`:

       if let Some(provision_args) = cli_args.provision_args {
           let exit_code = run_provision(provision_args).await;
           std::process::exit(exit_code);
       }

   Stub `run_provision()` returns `cli::exit_codes::GENERIC_FAILURE` and prints "provision: not implemented".

4. The parser treats `provision` as a token in the existing single-pass loop (same as `--install` today, after `trim_start_matches('-')`).

Manual smoke: `./target/debug/miru-agent provision --device-name=foo` prints "provision: not implemented" and exits 1.

Commit: `feat(cli): add provision subcommand skeleton`.

### Milestone 2 — HTTP layer

Goal: a tested `http::devices::create_or_fetch_device()` callable against `MockClient`.

Edits:

1. `agent/src/http/request.rs` — extend `Params<'a>` with `pub api_key: Option<&'a str>`. Initialize to `None` in every constructor (`get`, `post`, `patch`). Add a fluent setter:

       pub fn with_api_key(mut self, api_key: &'a str) -> Self {
           self.api_key = Some(api_key);
           self
       }

   In `build()` (after `add_token_to_headers`), add `add_api_key_to_headers()` doing `headers.insert("X-API-Key", HeaderValue::from_str(api_key)?)` with the same `InvalidHeaderValueErr` mapping. Unit-test `Params::with_api_key` and that `build()` puts the header on the request.

2. `agent/src/http/devices.rs` — define `CreateDeviceRequest` locally with a `// TODO(provision): move to libs/backend-api once OpenAPI spec is regenerated.` comment, plus the new function:

       #[derive(Debug, Serialize)]
       pub struct CreateDeviceRequest<'a> {
           pub name: &'a str,
       }

       pub struct CreateOrFetchDeviceParams<'a> {
           pub name: &'a str,
           pub api_key: &'a str,
       }

       pub async fn create_or_fetch_device(
           client: &impl ClientI,
           params: CreateOrFetchDeviceParams<'_>,
       ) -> Result<Device, HTTPErr> {
           // `client.base_url()` is `{backend_host}/v1` (constructed in run_provision),
           // so the absolute URL is `{backend_host}/v1/devices`.
           let url = format!("{}/devices", client.base_url());
           let body = request::marshal_json(&CreateDeviceRequest { name: params.name })?;
           let post = request::Params::post(&url, body).with_api_key(params.api_key);
           match super::client::fetch::<_, Device>(client, post).await {
               Ok(device) => Ok(device),
               Err(HTTPErr::RequestFailed(rf)) if rf.status == reqwest::StatusCode::CONFLICT => {
                   let get = request::Params::get(&url)
                       .with_api_key(params.api_key)
                       .with_query(QueryParams::from([("name", params.name)]));
                   super::client::fetch(client, get).await
               }
               Err(e) => Err(e),
           }
       }

   Add a sibling `issue_activation_token()` that POSTs to `/devices/{id}/activation_token` (distinct from `issue_token`'s `/devices/{id}/issue_token`) using `X-API-Key` instead of bearer auth. Define a local `IssueActivationTokenRequest { allow_reactivation: bool }` (parallel to `CreateDeviceRequest`, with the same TODO) and an `IssueActivationTokenParams<'a> { id: &'a str, api_key: &'a str, allow_reactivation: bool }`. Do NOT modify `issue_token`.

3. `agent/tests/mocks/http_client.rs` — add `create_or_fetch_device_fn` and `issue_activation_token_with_api_key_fn` closure fields. Default each to `Box::new(|_params| Ok(Default::default()))`, matching the existing convention (lines 72–80). Tests override per case.

4. Tests in `agent/tests/http/devices.rs` (extend or new file in that module): `create_or_fetch_device` happy path (200 → device returned), 409 path (POST 409, GET returns device), unrelated 5xx (propagated as `HTTPErr`). Use the existing test pattern.

Commit: `feat(http): add X-API-Key header and create_or_fetch_device`.

### Milestone 3 — Provision flow

Goal: `miru-agent provision --device-name=foo` runs end-to-end against a mocked backend in tests.

Edits:

1. `agent/src/installer/provision.rs` (NEW) — orchestrator, sibling to `install.rs`. Public entrypoint:

       pub async fn provision<PublicHTTPClientT, AgentHTTPClientT>(
           public_api_client: &PublicHTTPClientT,
           agent_client: &AgentHTTPClientT,
           layout: &storage::Layout,
           settings: &settings::Settings,
           api_key: &str,
           device_name: &str,
           allow_reactivation: bool,
       ) -> Result<backend_client::Device, ProvisionErr>
       where
           PublicHTTPClientT: http::ClientI,
           AgentHTTPClientT: http::ClientI,

   Steps:
   - `http::devices::create_or_fetch_device(public_api_client, { name: device_name, api_key })` → `Device` (uses `{backend_host}/v1`).
   - `http::devices::issue_activation_token(public_api_client, { id: &device.id, api_key, allow_reactivation })` → `TokenResponse`.
   - On `HTTPErr::RequestFailed` whose `error.code == "device_is_active"`, return `ProvisionErr::ReactivationNotAllowedErr`. All other `HTTPErr` map to `ProvisionErr::BackendErr`.
   - `installer::install::install(agent_client, layout, settings, &token, Some(device_name.to_string()))`. Map `InstallErr` into `ProvisionErr::InstallErr`.

   Define `ProvisionErr` with `thiserror` and `crate::impl_error!`, mirroring `InstallErr`. Variants: `MissingApiKeyErr(MissingEnvVarErr)`, `BackendErr(http::HTTPErr)`, `ReactivationNotAllowedErr { device_id: String, trace: Box<Trace> }`, `InstallErr(installer::errors::InstallErr)`, `NotRootErr { trace: Box<Trace> }` (M4), `SystemdErr { msg: String, trace: Box<Trace> }` (M4).

   Add `pub fn read_api_key_from_env() -> Result<String, ProvisionErr>` mirroring `install::read_token_from_env()` but reading `MIRU_API_KEY` and returning `MissingApiKeyErr`.

   Introduce `pub const DEFAULT_BACKEND_HOST: &str = "https://api.mirurobotics.com";` in `agent/src/installer/install.rs`. Refactor `Settings::default()` (in `agent/src/storage/settings.rs`) so its `backend.base_url` is built as `format!("{}/agent/v1", installer::install::DEFAULT_BACKEND_HOST)` (or move the constant to `storage::settings` if the cycle is awkward — implementer picks placement, but the constant is single source of truth). Refactor `installer::install::determine_settings` so it accepts inputs directly: `pub fn determine_settings_from(backend_host: Option<&str>, mqtt_broker_host: Option<&str>) -> settings::Settings`, using `DEFAULT_BACKEND_HOST` when `backend_host` is `None`. Have `determine_settings(args: &cli::InstallArgs)` delegate to it. `run_provision` calls `determine_settings_from` directly and reuses `DEFAULT_BACKEND_HOST` for the public-API client base URL.

2. `agent/src/installer/mod.rs`:
   - Extract `pub async fn init_installer_logging() -> Result<(logs::Guard, filesys::dir::Dir), InstallErr>` covering the existing 25-line setup in `agent/src/main.rs::run_installer` (lines 40–64): create `Dir::create_temp_dir("miru-agent-installer-logs")`, build `logs::Options { stdout: false, log_dir: tmp_dir.path().to_path_buf(), ..Default::default() }`, call `logs::init(options)`, return `(guard, tmp_dir)` to the caller for cleanup. Add `pub type Guard = tracing_appender::non_blocking::WorkerGuard;` to `agent/src/logs/mod.rs` (today `logs::init` returns `tracing_appender::non_blocking::WorkerGuard` directly and no alias exists).
   - Update `main.rs::run_installer` to call this helper.
   - Add `pub mod provision;`.

3. `agent/src/installer/errors.rs` — no changes (the new `ProvisionErr` lives in `provision.rs`).

4. `agent/src/main.rs` — replace the M1 stub with a real `run_provision()`:

       async fn run_provision(args: cli::ProvisionArgs) -> i32 {
           let (_guard, tmp_dir) = match installer::init_installer_logging().await {
               Ok(pair) => pair,
               Err(_) => return cli::exit_codes::GENERIC_FAILURE,
           };

           let api_key = match installer::provision::read_api_key_from_env() {
               Ok(k) => k,
               Err(_) => {
                   eprintln!("MIRU_API_KEY environment variable is not set");
                   return cli::exit_codes::MISSING_API_KEY;
               }
           };
           let device_name = match args.device_name.as_deref() {
               Some(n) => n,
               None => {
                   eprintln!("--device-name is required");
                   return cli::exit_codes::GENERIC_FAILURE;
               }
           };
           let settings = installer::install::determine_settings_from(
               args.backend_host.as_deref(),
               args.mqtt_broker_host.as_deref(),
           );

           let backend_host = args
               .backend_host
               .as_deref()
               .unwrap_or(installer::install::DEFAULT_BACKEND_HOST);
           let agent_http_client = match http::Client::new(&settings.backend.base_url) {
               Ok(c) => c,
               Err(_) => return cli::exit_codes::GENERIC_FAILURE,
           };
           let public_api_http_client =
               match http::Client::new(&format!("{}/v1", backend_host)) {
                   Ok(c) => c,
                   Err(_) => return cli::exit_codes::GENERIC_FAILURE,
               };

           let layout = storage::Layout::default();
           let result = installer::provision::provision(
               &public_api_http_client,
               &agent_http_client,
               &layout,
               &settings,
               &api_key,
               device_name,
               args.allow_reactivation,
           ).await;

           drop(_guard);
           if let Err(e) = tmp_dir.delete().await {
               eprintln!("failed to clean up provision log dir: {e}");
           }
           handle_provision_result(result)
       }

   `handle_provision_result()` reuses `display::format_info` / `display::color` for success output (matching `handle_install_result`) and maps `ProvisionErr` variants per the Decision Log exit-code table:
   - `Ok(_)` → `SUCCESS`
   - `MissingApiKeyErr` → `MISSING_API_KEY`
   - `BackendErr` → `BACKEND_ERROR`
   - `ReactivationNotAllowedErr` → `REACTIVATION_NOT_ALLOWED`
   - `InstallErr` → `INSTALL_FAILURE`
   - `NotRootErr` → `GENERIC_FAILURE` (M4 may refine)
   - `SystemdErr` → `INSTALL_FAILURE` (M4)

5. Tests under `agent/tests/provision/` (NEW). Mirror `agent/tests/installer/install.rs`. Required cases:
   - happy path: `MockClient` returns a device on POST, returns a token on issue, install bootstraps storage; `provision` returns `Ok(device)` and `/srv/miru` (test layout) is populated.
   - 409 fetch path: POST returns 409, GET returns device; assert the GET was called with `?name=<device-name>`.
   - `read_api_key_from_env()` returns `MissingApiKeyErr` when `MIRU_API_KEY` is unset.
   - backend 5xx on POST: `BackendErr` returned.
   - device already activated + `allow_reactivation=false`: token-issue mock returns the typed error; assert `ReactivationNotAllowedErr` with the correct `device_id`.
   - install failure (e.g., RSA keygen target dir read-only): `InstallErr` returned.

   Register the new module in `agent/tests/mod.rs`.

Commit: `feat(provision): orchestrate device create-or-fetch + activation`.

### Milestone 4 — Systemd handling and root-privilege check

Goal: provision behaves correctly when run as root on a machine that already has the `miru` systemd unit, and refuses to silently corrupt `/srv/miru` when run unprivileged.

Edits:

1. `agent/Cargo.toml` — add `libc = "0.2"` under `[dependencies]` (alphabetically between `futures` and `backend-api`).

2. `agent/src/installer/provision.rs`:
   - Define `fn assert_root() -> Result<(), ProvisionErr>` returning `Err(ProvisionErr::NotRootErr { trace: crate::trace!() })` when `unsafe { libc::geteuid() } != 0`. Lives in `installer/provision.rs` but is invoked from `run_provision()` in `agent/src/main.rs` as the **very first** statement — before `init_installer_logging()`, `read_api_key_from_env()`, or arg parsing — so privilege failure short-circuits before any side effects.
   - Add `fn stop_miru_unit_if_running()` and `fn restart_miru_unit_if_present()` using `std::process::Command::new("systemctl")`. Both must succeed silently when the unit is absent (handles the fresh-install case where the unit hasn't been written yet).

     Approach: always attempt the operation in a single shot and treat exit code 5 (`Unit miru.service not loaded`) as a no-op. For `stop`, run `systemctl stop miru`; for `restart`, run `systemctl restart miru`. Inspect exit code: `0` → success; `5` → no-op; any other non-zero → return `SystemdErr { msg, trace }` carrying captured stderr.

   - Wire into `provision()`: `stop_miru_unit_if_running()` runs at the very start of `provision()`, then the M3 flow (create-or-fetch → issue token → `installer::install::install`), then on success `restart_miru_unit_if_present()`. `assert_root()` is invoked from `run_provision()` in `main.rs` as the first statement, so by the time `provision()` is called the privilege check has passed.

   - Update the M3 `run_provision()` snippet in `agent/src/main.rs` to add `if let Err(_) = installer::provision::assert_root() { eprintln!("miru-agent provision must be run as root (sudo -E)"); return cli::exit_codes::GENERIC_FAILURE; }` as the very first statement, before `init_installer_logging()`.

3. Tests:
   - `assert_root` is hard to unit-test in CI (CI runs as non-root). Make `assert_root` a free function and add a `#[cfg(test)]` shim allowing tests to inject a fake euid; assert `assert_root()` returns `Err(NotRootErr)` when the fake euid is non-zero and `Ok(())` when zero. (The driver-level `run_provision()` short-circuit is exercised by the M6 manual smoke test.)
   - For systemd, abstract `Command::new("systemctl")` calls behind `SystemctlI { fn stop(&self, unit: &str) -> Result<(), SystemdErr>; fn restart(&self, unit: &str) -> Result<(), SystemdErr>; }` with a real impl `RealSystemctl` (maps exit 5 to `Ok(())` and any other non-zero to `SystemdErr`) and a mock for tests. Provision takes a `&impl SystemctlI`. Tests:
     - unit not loaded (mock returns `Ok(())` from both `stop` and `restart` — provision proceeds and finishes);
     - unit loaded (mock records both calls in order);
     - stop returns `SystemdErr` → provision returns `SystemdErr` and skips install.

Commit: `feat(provision): privilege check and systemd lifecycle`.

### Milestone 5 — Documentation

Edits:

1. `agent/ARCHITECTURE.md` — update lines 8–12 ("Installer mode (--install)") and lines 79–81 ("Device setup") to mention the `provision` subcommand. State that `provision` is the customer-facing entrypoint; `--install` remains the lower-level primitive used internally and by `provision.sh`.

2. `agent/README.md` — add a "Provisioning a device" section after the existing usage section. Include the target UX:

       export MIRU_API_KEY=<api-key>
       sudo -E miru-agent provision --device-name=$HOSTNAME --allow-reactivation=false

   List the exit codes (referenced from `agent/src/cli/exit_codes.rs`).

3. **Do not** modify `scripts/install/provision.sh`.

Commit: `docs: describe miru-agent provision subcommand`.

### Milestone 6 — Manual smoke test

Manual procedure (recorded in `Outcomes & Retrospective` after running):

1. Build: `cargo build --release -p miru-agent` from repo root.
2. Provision a fresh VM/container with no `/srv/miru` and no `miru` systemd unit. SCP the binary to `/usr/sbin/miru-agent`.
3. Run:

       export MIRU_API_KEY=<staging-api-key>
       sudo -E /usr/sbin/miru-agent provision \
           --device-name=$(hostname) \
           --backend-host=https://api-staging.miruml.com \
           --mqtt-broker-host=mqtt-staging.miruml.com \
           --allow-reactivation=false
       echo "exit=$?"

   Expected: prints "Successfully activated this device as <name>!", exit 0. `/srv/miru/device.json`, `/srv/miru/settings.json`, `/srv/miru/auth/private.key`, `/srv/miru/auth/public.key`, `/srv/miru/auth/token` exist and are owned by `miru:miru`.

4. Re-run same command. Expected: exit 4 (`REACTIVATION_NOT_ALLOWED`).
5. Re-run with `--allow-reactivation=true`. Expected: exit 0; new keypair on disk; previous `/srv/miru/auth/private.key` replaced.
6. On a machine with `miru.service` already active, run provision and observe (via `journalctl -u miru -f`) that the unit is stopped before install and restarted after.
7. `unset MIRU_API_KEY && sudo /usr/sbin/miru-agent provision --device-name=foo` → exit 2.
8. As non-root: `MIRU_API_KEY=x miru-agent provision --device-name=foo` → exit 1 with "must be run as root" message.

Commit: none required — Outcomes & Retrospective edits cover this.

## Concrete Steps

All commands assume working directory `/home/ben/miru/workbench4/repos/agent/` unless stated.

### Per-milestone validation recipe

Run, in order, after the edits for each milestone:

    cargo fmt -p miru-agent
    cargo build -p miru-agent
    ./scripts/test.sh
    ./scripts/lint.sh

All four must exit 0 before committing the milestone.

### Branch setup (once)

    git checkout -b feat/agent-provision-subcommand

### M1 — CLI plumbing
Files touched: `agent/src/cli/mod.rs`, `agent/src/cli/exit_codes.rs` (NEW), `agent/src/main.rs`.
Run the per-milestone validation recipe.
Manual smoke: `./target/debug/miru-agent provision --device-name=foo` prints "provision: not implemented" and exits 1.
Commit: `feat(cli): add provision subcommand skeleton`.

### M2 — HTTP layer
Files touched: `agent/src/http/request.rs`, `agent/src/http/devices.rs`, `agent/tests/mocks/http_client.rs`, `agent/tests/http/devices.rs` (extend or new).
Run the per-milestone validation recipe.
Commit: `feat(http): add X-API-Key header and create_or_fetch_device`.

### M3 — Provision flow
Files touched: `agent/src/installer/mod.rs`, `agent/src/installer/install.rs` (refactor `determine_settings`, add `determine_settings_from`), `agent/src/installer/provision.rs` (NEW), `agent/src/main.rs`, `agent/tests/provision/` (NEW), `agent/tests/mod.rs`.
Run the per-milestone validation recipe.
Commit: `feat(provision): orchestrate device create-or-fetch + activation`.

### M4 — Systemd + root check
Files touched: `agent/Cargo.toml` (add `libc = "0.2"`), `agent/src/installer/provision.rs`, `agent/tests/provision/`.
Run the per-milestone validation recipe.
Commit: `feat(provision): privilege check and systemd lifecycle`.

### M5 — Docs
Files touched: `agent/ARCHITECTURE.md`, `agent/README.md`.
Run only `./scripts/lint.sh` (no code changes).
Commit: `docs: describe miru-agent provision subcommand`.

### M6 — Manual smoke test

    cargo build --release -p miru-agent
    # Follow the manual procedure in Plan of Work / Milestone 6.
    # Record results in Outcomes & Retrospective.

### Final preflight (before opening PR)

    ./scripts/preflight.sh
    # Expected: ends with "Preflight clean".

## Validation and Acceptance

Acceptance is behavioral. Each criterion is checked against the binary or test suite.

`$preflight` must report `clean` before changes are published.

1. **The new subcommand parses and dispatches.**

       ./target/debug/miru-agent provision --device-name=foo --allow-reactivation=false

   - After M3, when run as root with `MIRU_API_KEY` unset → exits 2 with `MissingApiKey` error.
   - After M4, when run as non-root → exits 1 with `NotRoot` error before any other check.

2. **Unit tests pass.** `./scripts/test.sh` ends with `test result: ok. <N> passed; 0 failed; ...`. New tests in `agent/tests/provision/` and `agent/tests/http/devices.rs` pass; no regression.

3. **Lint passes.** `./scripts/lint.sh` ends with `Lint complete` and exit 0.

4. **Preflight clean.** `./scripts/preflight.sh` ends with `Preflight clean`. See the standalone preflight gate above.

5. **Manual smoke test (Milestone 6).** All eight steps produce the documented exit codes and observable on-disk / systemd state.

6. **Exit-code contract honoured by the binary.** Verified by either unit tests on `handle_provision_result()` or the M6 manual procedure. The authoritative mapping lives in the Decision Log entry "Exit codes"; each row must be exercised by either a unit test or one of the M6 steps.

## Idempotence and Recovery

- All `cargo`, `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/preflight.sh`, `./scripts/update-deps.sh` invocations are idempotent.
- The `provision` subcommand itself is **safe to re-run** as long as `--allow-reactivation=true`. Re-running with `=false` against an already-activated device exits 4 without modifying `/srv/miru`. Re-running with `=true` overwrites `/srv/miru/auth/*` (matches `provision.sh` semantics today).
- Systemd: `stop_miru_unit_if_running()` and `restart_miru_unit_if_present()` invoke `systemctl` once and treat exit 5 as a no-op, so re-running `provision` is safe whether the unit is loaded, stopped, or absent. Any other non-zero exit fails fast with `SystemdErr`. If the unit file is corrupted/missing after partial install, operator can `systemctl daemon-reload` and re-run.
- Filesystem: `install()` writes via temp dir + atomic move (see `agent/src/installer/install.rs` and `storage::setup::bootstrap`). A crash mid-write leaves previous `/srv/miru/auth/*` intact. Recovery: re-run `provision` with the same args.
- HTTP retries: not added by this plan. `http::Client` already times out at 10s; transient backend errors surface as exit 3 and the operator re-runs.
- Rollback for a regression discovered post-merge: `git revert <merge-sha>` removes the subcommand. `provision.sh` continues to work because it was never touched. Existing devices unaffected.
- Per-milestone commits make `git bisect` cheap.
