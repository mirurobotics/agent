# `miru-agent provision` Subcommand

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent` (this repo) | read-write | All changes land here: `agent/src/cli/`, `agent/src/main.rs`, `agent/src/http/`, `agent/src/installer/`, `agent/tests/`, `agent/README.md`, `agent/ARCHITECTURE.md`. |
| `backend` | read-only | Source of truth for the `POST /v1/devices`, `GET /v1/devices?name=…`, and `POST /v1/devices/{id}/activation_token` endpoints; we follow the same request/response shapes the existing `scripts/install/provision.sh` already proves out. |

This plan lives in `agent/plans/` because all code changes are made inside the `agent` repo.

## Purpose / Big Picture

Today a customer who wants to provision a new device must run `scripts/install/provision.sh`, which curls the backend twice (create-or-fetch device, then issue an activation token), then shells into the agent with `--install` and a `MIRU_ACTIVATION_TOKEN` env var. The script also has to manage the systemd unit. That works, but it requires the customer to have `curl` + `jq` + the script.

After this change, the customer can do the same thing using nothing but the `miru-agent` binary:

    export MIRU_API_KEY=<api-key>
    sudo -E miru-agent provision --device-name=$HOSTNAME --allow-reactivation=false

Observable behavior:

- Reads `MIRU_API_KEY` from the environment.
- Calls the backend to create-or-fetch the device by name (POST → on 409 conflict, GET).
- Requests an activation token (`POST /v1/devices/{id}/activation_token`).
- Runs the existing local install flow (the same code path `--install` already uses) to write `/srv/miru/{device.json,settings.json,auth/...}`.
- Stops the `miru` systemd unit before install if it is running, restarts it after install.
- Prints a clear success message; on failure exits with a structured exit code (see Decision Log) so customer scripts can branch on the "device already activated, reactivation not allowed" case.

`scripts/install/provision.sh` is **not** removed by this plan — existing customer deployments still use it. Both flows coexist until the next deprecation cycle.

The APT publishing pipeline (how the new binary gets onto the customer's machine in the first place) is out of scope; it is tracked separately.

## Progress

- [ ] Milestone 1: CLI plumbing — `ProvisionArgs` parsed, `main.rs` dispatches, exit-code constants exist, stub returns "not implemented".
- [ ] Milestone 2: HTTP layer — `X-API-Key` header support in `http/request.rs`, `create_or_fetch_device()` + `CreateDeviceRequest` in `http/devices.rs`, `MockClient` field added, unit tests pass.
- [ ] Milestone 3: Provision flow — `installer/provision.rs` implements end-to-end orchestration, wired into CLI, unit tests pass.
- [ ] Milestone 4: Systemd handling and root-privilege check.
- [ ] Milestone 5: Docs — `ARCHITECTURE.md` and `README.md` updated.
- [ ] Milestone 6: Manual smoke test against staging.

Each milestone ends with a single commit. One commit per milestone.

## Surprises & Discoveries

Add entries as work proceeds.

## Decision Log

Add entries as work proceeds. The key open decisions are listed in **Plan of Work** and must be recorded here once made (where provision code lives, how `X-API-Key` is plumbed, where `CreateDeviceRequest` lives, systemd policy, root-check policy, exact exit codes).

## Outcomes & Retrospective

Fill in at the end of each milestone and again at completion.

## Context and Orientation

A reader who has never seen this repo before should be able to find their feet from this section alone.

**Repository layout (relevant parts).**

- `agent/src/main.rs` — binary entrypoint. Lines 22–37 dispatch on parsed CLI args: `display_version` first, then `install_args` (runs `run_installer()` and exits via `handle_install_result()`), then falls through to `run_agent()`. New provision dispatch lands here.
- `agent/src/cli/mod.rs` — custom CLI parser (no clap). Defines `Args { display_version, install_args }` and `InstallArgs { backend_host, mqtt_broker_host, device_name }`. Args are split on `=` after stripping leading dashes. New `ProvisionArgs` and a new `provision_args: Option<ProvisionArgs>` field on `Args` go here.
- `agent/src/installer/install.rs` — `install()` async function (lines 18–53) that drives RSA keygen, calls `register_with_backend()` (which POSTs to `/devices/{id}/activate`), and bootstraps `/srv/miru/`. Reads activation JWT from `MIRU_ACTIVATION_TOKEN` via `read_token_from_env()` (lines 55–68). Function signature:

      pub async fn install<HTTPClientT: http::ClientI>(
          http_client: &HTTPClientT,
          layout: &storage::Layout,
          settings: &settings::Settings,
          token: &str,
          device_name: Option<String>,
      ) -> Result<backend_client::Device, InstallErr>

  Also defines `determine_settings(args: &cli::InstallArgs) -> settings::Settings` which the provision flow will reuse (input adapter required — see Plan of Work).

- `agent/src/installer/errors.rs` — `InstallErr` enum (variants: `MissingEnvVarErr`, `AuthnErr`, `CryptErr`, `FileSysErr`, `HTTPErr`, `StorageErr`) and the `MissingEnvVarErr { name, trace }` struct used for env-var failures. `crate::impl_error!` registers the enum in the project's error machinery.
- `agent/src/installer/mod.rs` — module file; needs a new `pub mod provision;` line.
- `agent/src/installer/display.rs` — `format_info()` and `color()` helpers used by `main.rs::handle_install_result` for user-facing output. The provision result handler should reuse these so output style matches.
- `agent/src/http/devices.rs` — three existing functions (`activate`, `issue_token`, `update`) and three matching `*Params<'a>` structs. Uses `request::Params::post(...).with_token(...)` for bearer-auth POSTs. **No** create-or-fetch function exists; **no** API-key header support exists.
- `agent/src/http/request.rs` — `Params<'a>` struct (lines 20–28) carries `token: Option<&'a str>`; `build()` (lines 201–234) calls `add_token_to_headers()` (lines 236–248) which sets `Authorization: Bearer <token>`. Default headers (`Headers::to_map`, lines 162–172) set `Miru-Version`, `Miru-Agent-Version`, etc., but **no** `X-API-Key`. New API-key support extends `Params` and `build()`.
- `agent/src/http/errors.rs` — `HTTPErr` variants: `RequestFailed`, `TimeoutErr`, `ReqwestErr`, `BuildReqwestErr`, `InvalidURLErr`, `InvalidHeaderValueErr`, `MarshalJSONErr`, `UnmarshalJSONErr`, `MockErr`. Backend errors deserialize into `backend_api::models::ErrorResponse { error: { code, message, params } }` — `RequestFailed` already carries an `Option<ErrorResponse>`, so the provision flow can branch on `error.code` to detect the "device already activated" case.
- `agent/tests/installer/install.rs` — pattern for installer tests: `#[tokio::test]`, `MockClient` from `agent/tests/mocks/http_client.rs` (closure fields per endpoint), filesystem isolation via `filesys::Dir::create_temp_dir("install-test")`. New `agent/tests/provision/` module mirrors this pattern.
- `agent/tests/mocks/http_client.rs` — `MockClient` with `activate_device_fn`, `issue_device_token_fn`, etc. closures. New `create_device_fn` and `fetch_device_by_name_fn` (or a single `create_or_fetch_device_fn` if Decision in Plan of Work picks the merged function) get added here.
- `agent/tests/mod.rs` — registers test modules; new `provision` module gets added.
- `libs/backend-api/src/models/` — generated OpenAPI types: `Device`, `ActivateDeviceRequest`, `IssueDeviceTokenRequest` (already has `allow_reactivation`), `TokenResponse`, `UpdateDeviceFromAgentRequest`, `ErrorResponse`. **No** request type for `POST /v1/devices`.
- `scripts/install/provision.sh` — the existing shell-based provisioner. **Read-only for this plan**; the plan does not delete or modify it.
- `agent/ARCHITECTURE.md` — lines 8–12 and 79–81 describe "Installer mode (--install)" and "Device setup". Both get updated in Milestone 5 to mention `provision`.
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

- *Activation token*: a short-lived JWT issued by the backend for a specific device id. Encodes the device id; consumed by `POST /v1/devices/{id}/activate` together with the device's freshly-generated public key. Today read from `MIRU_ACTIVATION_TOKEN`.
- *API key*: a long-lived secret scoped to a customer organization. Sent as `X-API-Key: <key>` on the device-create and token-issue calls. Today only `provision.sh` uses it; the agent does not.
- *Provision*: the act of (a) creating-or-fetching a device row by name on the backend and (b) activating that row by uploading a public key to receive durable credentials. `provision.sh` is the existing shell implementation; this plan adds a Rust implementation inside `miru-agent`.
- *Reactivation*: requesting a new activation token for a device that has previously been activated. The backend rejects it unless the request body sets `allow_reactivation: true`.
- *Layout* (`storage::Layout`): the `/srv/miru/...` filesystem layout. `Layout::default()` points at the production paths; tests build a layout under a temp dir.

**Backend endpoints used (verified against `scripts/install/provision.sh`).**

- `POST {backend_host}/v1/devices`
  Headers: `X-API-Key: $MIRU_API_KEY`, `Content-Type: application/json`, `Miru-Version: 2026-03-09.tetons`.
  Body: `{"name": "<device-name>"}`.
  Responses: `200`/`201` → `Device`. `409 Conflict` → device exists; the caller must then `GET /v1/devices?name=<device-name>` (same `X-API-Key` header) to retrieve it.

- `POST {backend_host}/v1/devices/{device_id}/activation_token`
  Headers: `X-API-Key: $MIRU_API_KEY`.
  Body: `{"allow_reactivation": <bool>}`.
  Response: `{"token": "<jwt>", "expires_at": "..."}`. Backend rejects with a typed error when the device was previously activated and `allow_reactivation=false`. The plan must branch on the error code to surface exit code 4 (see Plan of Work).

- `POST {backend_host}/v1/devices/{device_id}/activate`
  This is the existing, agent-issued call performed by `installer::install()`. Bearer-auth with the activation JWT. Untouched by this plan.

## Plan of Work

The work breaks into six milestones. Each milestone ends with one commit.

### Milestone 1 — CLI plumbing

Goal: `miru-agent provision --device-name=foo --allow-reactivation=false` parses cleanly, dispatches to a stub handler that prints "not implemented", and exits 1. Compiles, lints, and existing tests still pass.

Edits:

1. `agent/src/cli/mod.rs` — add `provision_args: Option<ProvisionArgs>` to `Args`. Recognize the `provision` subcommand-style token by adding a match arm to the top-level loop so `inputs.iter().skip(1)` matching `"provision"` (after `trim_start_matches('-')`) sets `provision_args = Some(ProvisionArgs::parse(inputs))`. Mirror the `--install` shape: `provision` is a positional verb, the rest of the flags are `--key=value`. Define:

       #[derive(Debug, Default)]
       pub struct ProvisionArgs {
           pub backend_host: Option<String>,
           pub mqtt_broker_host: Option<String>,
           pub device_name: Option<String>,
           pub allow_reactivation: bool,  // default false
       }

   `allow_reactivation` parses from `--allow-reactivation=true|false` (default `false`). Add unit tests covering: required `--device-name` parsed; `--allow-reactivation=true` and `=false`; unknown flags ignored; missing `--device-name` produces `device_name = None` (validation happens later).

2. `agent/src/cli/exit_codes.rs` (NEW) — define exit-code constants. Decision Log entry must record the values picked, but use these:

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

4. **Decision to record:** the parser today uses a single shared loop and treats `--install` as a flag. `provision` is a verb, but to minimize scope we treat it as a flag too (no positional-vs-flag distinction). Document this in Decision Log.

Validation for M1: `cargo build -p miru-agent`, `cargo fmt -p miru-agent`, `cargo clippy --package miru-agent --all-features -- -D warnings`, `./scripts/test.sh` all green. Manual: `./target/debug/miru-agent provision --device-name=foo` prints "not implemented" and exits 1.

Commit: `feat(cli): add provision subcommand skeleton`.

### Milestone 2 — HTTP layer

Goal: a tested `http::devices::create_or_fetch_device()` function exists and can be called against `MockClient`.

Edits:

1. `agent/src/http/request.rs` — extend `Params<'a>` with `pub api_key: Option<&'a str>`. Initialize to `None` in every constructor (`get`, `post`, `patch`). Add a fluent setter:

       pub fn with_api_key(mut self, api_key: &'a str) -> Self {
           self.api_key = Some(api_key);
           self
       }

   In `build()` (after `add_token_to_headers`), add `add_api_key_to_headers()` that does `headers.insert("X-API-Key", HeaderValue::from_str(api_key)?)` with the same `InvalidHeaderValueErr` mapping as `add_token_to_headers`. Unit-test `Params::with_api_key` and that `build()` puts the header on the request.

2. `agent/src/http/devices.rs` — add a local request type and the new function. **Decision to record:** define `CreateDeviceRequest` locally in `agent/src/http/devices.rs` rather than regenerate the OpenAPI client. Long-term it should move into `libs/backend-api`, but the OpenAPI generator pipeline is out of scope for this plan. Add a `// TODO(provision): move to libs/backend-api once OpenAPI spec is regenerated.` comment at the type definition.

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
           let url = format!("{}/devices", client.base_url());
           let body = request::marshal_json(&CreateDeviceRequest { name: params.name })?;
           let post = request::Params::post(&url, body).with_api_key(params.api_key);
           match super::client::fetch::<_, Device>(client, post).await {
               Ok(device) => Ok(device),
               Err(HTTPErr::RequestFailed(_, status, _)) if status == reqwest::StatusCode::CONFLICT => {
                   let get = request::Params::get(&url)
                       .with_api_key(params.api_key)
                       .with_query(QueryParams::from([("name", params.name)]));
                   super::client::fetch(client, get).await
               }
               Err(e) => Err(e),
           }
       }

   **Decision to record:** the create-and-fetch fallback is performed inside the HTTP-layer function (one logical operation from the caller's perspective) rather than orchestrated in the provision module. This matches how `provision.sh` presents it to users.

   Note: the precise `RequestFailed` arm pattern depends on the variant signature in `agent/src/http/errors.rs`. Verify against that file when implementing — match against the same shape `super::client::fetch` already uses elsewhere in `devices.rs` (none today), so cross-reference `agent/src/http/client.rs` for how `RequestFailed` is constructed and how `StatusCode` is exposed.

   Add a sibling `issue_activation_token()` (or extend `issue_token()`) that uses `X-API-Key` instead of bearer auth — `provision.sh` calls `POST /v1/devices/{id}/activation_token` with `X-API-Key`, not bearer. The existing `issue_token` is unauthenticated in headers (lines 39–46 of `devices.rs`), and the body shape (`IssueDeviceTokenRequest`) already has `allow_reactivation`. **Decision to record:** add a new function `issue_activation_token_with_api_key` rather than mutating `issue_token`, to avoid breaking any existing callers. (Verify call sites with `Grep "issue_token"` before deciding; if no other callers exist, change `issue_token` directly and document that.)

3. `agent/tests/mocks/http_client.rs` — add `create_or_fetch_device_fn: Option<...>` closure field. Add `issue_activation_token_with_api_key_fn` (or update existing). Provide reasonable defaults that panic with "no mock set" so tests fail loudly.

4. Tests in `agent/tests/http/devices.rs` (or new file under that module): test `create_or_fetch_device` happy path (200 → device returned), 409 path (POST returns 409, GET returns device), and unrelated 5xx (propagated as `HTTPErr`). Use the existing test pattern in `agent/tests/http/`.

Validation for M2: `./scripts/test.sh` and `./scripts/lint.sh` clean.

Commit: `feat(http): add X-API-Key header and create_or_fetch_device`.

### Milestone 3 — Provision flow

Goal: `miru-agent provision --device-name=foo` runs end-to-end against a mocked backend in tests.

Edits:

1. `agent/src/installer/provision.rs` (NEW) — the orchestrator. **Decision to record:** lives at `agent/src/installer/provision.rs`, sibling to `install.rs`, because it shares the device-onboarding domain. Alternative (`agent/src/provision.rs` top-level) was considered and rejected to keep onboarding code colocated.

   Public entrypoint signature:

       pub async fn provision<HTTPClientT: http::ClientI>(
           http_client: &HTTPClientT,
           layout: &storage::Layout,
           settings: &settings::Settings,
           api_key: &str,
           device_name: &str,
           allow_reactivation: bool,
       ) -> Result<backend_client::Device, ProvisionErr>

   Steps inside:
   - Call `http::devices::create_or_fetch_device({ name: device_name, api_key })` → `Device`.
   - Call `http::devices::issue_activation_token_with_api_key({ id: device.id, api_key, allow_reactivation })` → `TokenResponse`.
   - Branch on the typed backend error for "device already activated && reactivation not allowed" — return a distinct `ProvisionErr::ReactivationNotAllowed` variant so the CLI can map to exit code 4. The exact backend error code string must be confirmed against the backend (search `backend/internal/...` or live-test with `provision.sh`); record the verified string in Decision Log.
   - Hand off to `installer::install::install(http_client, layout, settings, &token, Some(device_name.to_string()))`. Map the resulting `InstallErr` into `ProvisionErr::InstallErr`.

   Define `ProvisionErr` with `thiserror` and `crate::impl_error!`, mirroring `InstallErr`. Variants: `MissingApiKeyErr(MissingEnvVarErr)`, `BackendErr(http::HTTPErr)`, `ReactivationNotAllowedErr { device_id: String, trace: Box<Trace> }`, `InstallErr(installer::errors::InstallErr)`, `NotRootErr { trace: Box<Trace> }` (used in M4), `SystemdErr { msg: String, trace: Box<Trace> }` (used in M4).

   Add `pub fn read_api_key_from_env() -> Result<String, ProvisionErr>` mirroring `install::read_token_from_env()` but reading `MIRU_API_KEY` and returning `MissingApiKeyErr`.

   Add a settings adapter: provision needs `backend.base_url` and `mqtt_broker.host`, same as install. Reuse `install::determine_settings` by constructing an `InstallArgs { backend_host, mqtt_broker_host, device_name }` from `ProvisionArgs` — or, cleaner, refactor `determine_settings` into a free function that takes `(backend_host: Option<&str>, mqtt_broker_host: Option<&str>)` and call it from both. **Decision to record:** the plan picks the refactor (small, safe, removes duplication).

2. `agent/src/installer/mod.rs` — add `pub mod provision;`.

3. `agent/src/installer/errors.rs` — no changes needed (the new `ProvisionErr` lives in `provision.rs`).

4. `agent/src/main.rs` — replace the M1 stub with a real `run_provision()`:

       async fn run_provision(args: cli::ProvisionArgs) -> i32 {
           // initialize logging exactly as run_installer does (temp dir, stdout off)
           // ...
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
           let http_client = match http::Client::new(&settings.backend.base_url) {
               Ok(c) => c,
               Err(_) => return cli::exit_codes::BACKEND_ERROR,
           };
           let layout = storage::Layout::default();
           let result = installer::provision::provision(
               &http_client, &layout, &settings, &api_key,
               device_name, args.allow_reactivation,
           ).await;
           handle_provision_result(result)
       }

   `handle_provision_result()` reuses `display::format_info` / `display::color` for success output (matching `handle_install_result`) and maps `ProvisionErr` variants to exit codes:
   - `Ok(_)` → `SUCCESS`
   - `MissingApiKeyErr` → `MISSING_API_KEY`
   - `BackendErr` → `BACKEND_ERROR`
   - `ReactivationNotAllowedErr` → `REACTIVATION_NOT_ALLOWED`
   - `InstallErr` → `INSTALL_FAILURE`
   - `NotRootErr` → `GENERIC_FAILURE` (M4 may refine)
   - `SystemdErr` → `INSTALL_FAILURE` (M4)

5. Tests under `agent/tests/provision/` (new directory). Mirror `agent/tests/installer/install.rs`. Required cases:
   - happy path: `MockClient` returns a device on POST, returns a token on issue, install bootstraps storage; `provision` returns `Ok(device)` and `/srv/miru` (test layout) is populated.
   - 409 fetch path: POST returns 409, GET returns device; assert the GET was called with `?name=<device-name>`.
   - missing `MIRU_API_KEY`: assert `MissingApiKeyErr`.
   - backend 5xx on POST: `BackendErr` returned.
   - device already activated + `allow_reactivation=false`: token-issue mock returns the typed error; assert `ReactivationNotAllowedErr` with the correct `device_id`.
   - install failure (e.g., RSA keygen target dir read-only): `InstallErr` returned.

   Register the new module in `agent/tests/mod.rs`.

Validation for M3: `./scripts/test.sh` and `./scripts/lint.sh` clean.

Commit: `feat(provision): orchestrate device create-or-fetch + activation`.

### Milestone 4 — Systemd handling and root-privilege check

Goal: provision behaves correctly when run as root on a machine that already has the `miru` systemd unit, and refuses to silently corrupt `/srv/miru` when run unprivileged.

Edits:

1. `agent/src/installer/provision.rs`:
   - Add `fn assert_root() -> Result<(), ProvisionErr>` using `nix::unistd::geteuid()` (already a dep — confirm with `Grep` against `agent/Cargo.toml`; if not present, use `unsafe { libc::geteuid() } == 0` which is the more portable raw call). Returns `NotRootErr` if not root. Call at the very top of `provision()`.
   - Add `fn stop_miru_unit_if_running()` and `fn restart_miru_unit_if_present()` using `std::process::Command::new("systemctl")`. Both must succeed silently when the unit is absent (handles the fresh-install case where the unit hasn't been written yet — `provision` may run either before or after a fresh apt install).

     Approach: `systemctl is-active miru` exit code 0 → unit is loaded; otherwise treat as absent and return `Ok`. On `is-active` returning 0, run `systemctl stop miru` (before install) and `systemctl restart miru` (after install). If those calls themselves fail, return `SystemdErr { msg, trace }`.

     **Decision to record:** the `is-active`-first probe is the chosen policy; alternative (always run `stop` and ignore "unit not found") was rejected as too easy to mask real failures.

   - Wire these into `provision()`: `assert_root()` first → `read_api_key_from_env()` → `stop_miru_unit_if_running()` → existing flow → on success, `restart_miru_unit_if_present()`.

2. Tests:
   - `assert_root` is hard to unit-test inside CI (CI usually runs as a non-root user). Make `assert_root` a free function and add a `#[cfg(test)]` shim allowing tests to inject a fake euid; assert `provision()` short-circuits with `NotRootErr`.
   - For systemd, abstract the `Command::new("systemctl")` calls behind a small trait (e.g., `SystemctlI { fn is_active(&self, unit: &str) -> bool; fn stop(&self, unit: &str) -> Result<(), SystemdErr>; fn restart(&self, unit: &str) -> Result<(), SystemdErr>; }`) with a real impl `RealSystemctl` and a mock impl for tests. Provision takes a `&impl SystemctlI`. Add tests for: unit absent → no calls beyond `is_active`; unit present → stop called, then restart called; stop fails → provision returns `SystemdErr` and skips install.

Validation for M4: `./scripts/test.sh` and `./scripts/lint.sh` clean.

Commit: `feat(provision): privilege check and systemd lifecycle`.

### Milestone 5 — Documentation

Edits:

1. `agent/ARCHITECTURE.md` — update lines 8–12 ("Installer mode (--install)") and lines 79–81 ("Device setup") to mention the `provision` subcommand. State that `provision` is the customer-facing entrypoint; `--install` remains the lower-level primitive used internally and by `provision.sh`.

2. `agent/README.md` — add a new "Provisioning a device" section after the existing usage section. Include the target UX:

       export MIRU_API_KEY=<api-key>
       sudo -E miru-agent provision --device-name=$HOSTNAME --allow-reactivation=false

   List the exit codes (referenced from `agent/src/cli/exit_codes.rs`).

3. **Do not** modify `scripts/install/provision.sh` — the script is left untouched per Scope.

Validation for M5: links and headings render; no code changes, so `./scripts/lint.sh` is the only check needed.

Commit: `docs: describe miru-agent provision subcommand`.

### Milestone 6 — Manual smoke test

Goal: prove end-to-end behavior against a live (staging) backend before declaring done.

Manual procedure (recorded in `Outcomes & Retrospective` after running):

1. Build a release binary: `cargo build --release -p miru-agent` from repo root.
2. Provision a fresh VM (or container) with no `/srv/miru` and no `miru` systemd unit. SCP the binary to `/usr/sbin/miru-agent`.
3. Run:

       export MIRU_API_KEY=<staging-api-key>
       sudo -E /usr/sbin/miru-agent provision \
           --device-name=$(hostname) \
           --backend-host=https://api-staging.miruml.com \
           --mqtt-broker-host=mqtt-staging.miruml.com \
           --allow-reactivation=false
       echo "exit=$?"

   Expected: prints "Successfully activated this device as <name>!", exit 0. `/srv/miru/device.json`, `/srv/miru/settings.json`, `/srv/miru/auth/private.key`, `/srv/miru/auth/public.key`, `/srv/miru/auth/token` all exist and are owned by `miru:miru`.

4. Re-run the same command. Expected: exit 4 (`REACTIVATION_NOT_ALLOWED`) since the device was just activated and `allow_reactivation=false`.

5. Re-run with `--allow-reactivation=true`. Expected: exit 0; new keypair on disk; previous `/srv/miru/auth/private.key` is replaced.

6. On a machine that already has `miru.service` active, run provision and observe (via `journalctl -u miru -f`) that the unit is stopped before install and restarted after.

7. Confirm with `unset MIRU_API_KEY && sudo /usr/sbin/miru-agent provision --device-name=foo` → exit 2.

8. As a non-root user: `MIRU_API_KEY=x miru-agent provision --device-name=foo` → exit 1 with "must be run as root" message.

Commit: none required — Outcomes & Retrospective edits cover this.

## Concrete Steps

All commands assume working directory `/home/ben/miru/workbench4/repos/agent/` unless stated.

**M1 — CLI plumbing.**

    git checkout -b feat/agent-provision-subcommand
    # Edit agent/src/cli/mod.rs, agent/src/cli/exit_codes.rs (NEW), agent/src/main.rs.
    cargo fmt -p miru-agent
    cargo build -p miru-agent
    cargo clippy --package miru-agent --all-features -- -D warnings
    ./scripts/test.sh
    # Manual smoke:
    ./target/debug/miru-agent provision --device-name=foo
    # Expected: prints "provision: not implemented", exit 1.
    git add agent/src/cli/mod.rs agent/src/cli/exit_codes.rs agent/src/main.rs
    git commit -m "feat(cli): add provision subcommand skeleton"

**M2 — HTTP layer.**

    # Edit agent/src/http/request.rs, agent/src/http/devices.rs, agent/tests/mocks/http_client.rs,
    # add agent/tests/http/devices.rs (or extend existing).
    cargo fmt -p miru-agent
    cargo build -p miru-agent
    ./scripts/test.sh
    # Expected: new tests for create_or_fetch_device and X-API-Key header pass.
    ./scripts/lint.sh
    git add agent/src/http/ agent/tests/
    git commit -m "feat(http): add X-API-Key header and create_or_fetch_device"

**M3 — Provision flow.**

    # Add agent/src/installer/provision.rs, edit agent/src/installer/mod.rs and agent/src/main.rs.
    # Add agent/tests/provision/ directory and register in agent/tests/mod.rs.
    cargo fmt -p miru-agent
    cargo build -p miru-agent
    ./scripts/test.sh
    ./scripts/lint.sh
    git add agent/src/installer/ agent/src/main.rs agent/tests/
    git commit -m "feat(provision): orchestrate device create-or-fetch + activation"

**M4 — Systemd + root check.**

    # Edit agent/src/installer/provision.rs; add SystemctlI trait + RealSystemctl impl.
    cargo fmt -p miru-agent
    cargo build -p miru-agent
    ./scripts/test.sh
    ./scripts/lint.sh
    git add agent/src/installer/provision.rs agent/tests/provision/
    git commit -m "feat(provision): privilege check and systemd lifecycle"

**M5 — Docs.**

    # Edit agent/ARCHITECTURE.md, agent/README.md.
    ./scripts/lint.sh
    git add agent/ARCHITECTURE.md agent/README.md
    git commit -m "docs: describe miru-agent provision subcommand"

**M6 — Manual smoke test.**

    cargo build --release -p miru-agent
    # Follow the manual procedure in Plan of Work / Milestone 6.
    # Record results in Outcomes & Retrospective.

**Final preflight (before opening PR).**

    ./scripts/preflight.sh
    # Expected: ends with "Preflight clean".

## Validation and Acceptance

Acceptance is behavioral. Each criterion below is checked against the binary or test suite.

1. **The new subcommand parses and dispatches.**

       ./target/debug/miru-agent provision --device-name=foo --allow-reactivation=false

   After M3, this exits 2 if `MIRU_API_KEY` is unset; after M4 it exits 1 (`NotRoot`) if not root.

2. **Unit tests pass.** From repo root:

       ./scripts/test.sh

   Expected: final line `test result: ok. <N> passed; 0 failed; ...`. The new tests in `agent/tests/provision/` and `agent/tests/http/devices.rs` (create_or_fetch + X-API-Key) all pass. No previously-passing test regresses.

3. **Lint passes.** From repo root:

       ./scripts/lint.sh
       echo "exit=$?"

   Expected: ends with `Lint complete` and `exit=0`. The custom import linter, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo machete`, and `cargo audit` all pass.

4. **Preflight clean.** From repo root:

       ./scripts/preflight.sh

   Expected: trailing line `Preflight clean`. **`$preflight` must report `clean` before changes are published.**

5. **Manual smoke test (Milestone 6).** All eight steps in the M6 procedure produce the documented exit codes and observable on-disk / systemd state.

6. **Exit-code contract honoured by the binary.** Verified by either unit tests on `handle_provision_result()` or the manual procedure:
   - `MIRU_API_KEY` unset → exit 2.
   - Network down or backend 5xx on either device-create or token-issue → exit 3.
   - Device already activated, `--allow-reactivation=false` → exit 4.
   - `/srv/miru` not writable, RSA keygen fails, etc. → exit 5.
   - Otherwise non-zero → exit 1.

## Idempotence and Recovery

- All `cargo`, `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/preflight.sh`, `./scripts/update-deps.sh` invocations are idempotent.
- The `provision` subcommand itself is **safe to re-run** as long as `--allow-reactivation=true`. Re-running with `=false` against an already-activated device exits 4 without modifying `/srv/miru`. Re-running with `=true` overwrites `/srv/miru/auth/*` (this matches `provision.sh` semantics today).
- Systemd handling: stopping a unit that is already stopped is a no-op via the `is-active` probe. Restarting a unit that is not loaded is suppressed by the same probe. If the unit file is corrupted or missing after partial install, the operator can `systemctl daemon-reload` and re-run `provision`.
- Filesystem: the `install()` flow writes via a temp dir + atomic move (see `agent/src/installer/install.rs` and `storage::setup::bootstrap`). A crash mid-write leaves the previous `/srv/miru/auth/*` intact. Recovery: re-run `provision` with the same args.
- HTTP retries: not added by this plan. `http::Client` already times out at 10s; transient backend errors surface as exit 3 and the operator re-runs.
- Rollback for a regression discovered post-merge: `git revert <merge-sha>` removes the subcommand. `provision.sh` continues to work because it was never touched. Existing devices are unaffected (no schema or storage changes).
- Per-milestone commits make `git bisect` cheap if a regression slips through.
