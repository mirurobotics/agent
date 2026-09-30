# TCP token auth and discovery file for the local device API

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench2/repos/agent) | read-write | Rust workspace for the Miru device agent (crate `miru-agent` in `agent/`). All edits, validation, and commits happen here. |
| `python-device-sdk`, `openapi` | not touched | Client transport and the upstream device spec security scheme are follow-ups (see Out of scope). |

Branch `feat/tcp-token-auth` (already checked out), PR base `main`. This is **PR 12 — token auth + discovery file** of the roadmap `plans/active/20260910-windows-support.md` (Phase 2, local device API). The plan lives here because every change is in the agent repo.

## Purpose / Big Picture

PR 11 (PR #258) added an optional loopback TCP transport for the local device API (`127.0.0.1:<port>`, on by default only on Windows). Today any local user or process can call every route over it. After this change every TCP request must carry `Authorization: Bearer <token>`. The agent generates a fresh random token each start and writes it, with the bound port, to a "discovery file" that only the agent's owner and group can read. Clients read the file, then call the API.

Observable result: with the agent running and TCP enabled, `curl -i http://127.0.0.1:<port>/v0.2/health` returns `401 Unauthorized` with `WWW-Authenticate: Bearer`; the same request with `-H "Authorization: Bearer $(jq -r .token <discovery file>)"` returns `200`; `curl -N` on `/v0.2/events` with the header streams a `: heartbeat` line. On Linux the discovery file is `/var/lib/miru/device-api/device-api.json` (mode `0640`); on Windows `C:\ProgramData\Miru\device-api\device-api.json`. The file disappears when the agent stops. The Unix socket transport is unchanged and unauthenticated.

Out of scope (follow-ups): `python-device-sdk` (TCP transport, discovery-file reader, re-read-on-401); MSI `Miru Clients` local group plus an inheritable read ACE on `ProgramData\Miru\device-api` (`build/windows/miru-agent.wxs`, roadmap PR 9 note); the upstream `mirurobotics/openapi` device spec security scheme (`api/specs/device/v02.yaml` is vendored — do not edit it here). The agent never sets Windows ACLs through FFI.

## Progress

- [x] M0 Activate plan (move to `plans/active/`; `docs(plans):` commit)
- [x] M1 `server/auth.rs` (Token, `check_bearer`), `GenerateTokenErr`, Authorization redaction in `routes.rs` and its log-capture test, unit tests
- [x] M2 `server/discovery.rs`, `Layout::device_api_dir`/`device_api_discovery`, `tcp::serve` wiring and its `init_tcp_server` call site, integration tests
- [x] M3 `app/run.rs` stale-file removal and run-level tests
- [x] M4 Docs: `ARCHITECTURE.md`, doc comments, roadmap PR 12 marking + decision log
- [x] M5 Preflight CLEAN (CI green on `1c1636de` in one round; re-confirmed green on `4d9d4ef8` after the move to `plans/completed/`; PR #273 marked ready) (draft PR open, CI green incl. `windows-check`); then plan moved to `plans/completed/` with this box ticked (green SHA) and Outcomes filled, and CI re-confirmed green on that commit

## Surprises & Discoveries

- 2026-09-30: Review found the original M3 placement (at the top of `init_optional_services`, after `init_app_state`) left a stale discovery file when init failed early (for example invalid storage). Moved the removal to the first line of `init`.

## Decision Log

- 2026-09-30: Token = 32 bytes from `aws_lc_rs::rand::fill`, base64url no-pad (43 chars, header-safe), regenerated every start, held only in `Arc<Token>` (never in `AppOptions` or `server::State`, both Debug-logged).
- 2026-09-30: Compare with `aws_lc_rs::constant_time::verify_slices_are_equal` (length is public: always 43). `tower_http` `ValidateRequestHeaderLayer::bearer` is not constant-time; not used.
- 2026-09-30: Auth covers every TCP route including `/v0.2/health` and `/v0.2/events`; the Unix socket stays unauthenticated (file mode 0660, group `miru`).
- 2026-09-30: Layer order loopback (403) → bearer (401) → router, so unauthenticated requests never touch the idle activity tracker.
- 2026-09-30: Discovery file in a dedicated `device-api/` dir under the data root, so the MSI can grant readers that dir alone without exposing `settings.json`/`device.json`.
- 2026-09-30: File mode `0o640` (owner rw, group read): matches the `miru`-group boundary of the socket and the public-key precedent; `0o600` would lock out `miru`-group SDK clients.
- 2026-09-30: Remove the file on graceful shutdown and remove any stale file at startup, so clients can tell "agent not serving TCP" (no file) from "token rotated" (401 → re-read).
- 2026-09-30: `tcp::serve` failure (token or discovery write) is logged and the agent continues without TCP, like a bind failure.
- 2026-09-30: Stale discovery-file removal runs first in `init`, before any fallible init step, so a restart loop that fails early never leaves a file behind.

## Outcomes & Retrospective

Delivered in PR #273. TCP requests to the local device API now need a per-start Bearer token, which clients read from `<data root>/device-api/device-api.json` (mode 0640 on unix). The Unix socket is unchanged. Preflight was CLEAN on the first CI round with no CI fixes needed: lint, test (coverage gates), tools, windows-check and windows-package-scope all passed. The required `ai-review` job skips draft PRs, so it first ran when the PR was marked ready.

One change from the original plan: stale discovery-file removal moved from `init_optional_services` to the first line of `init`, so an early init failure can't leave an old port and token on disk through a restart loop (see Surprises and Decision Log).

Remaining work, all in other repos or PRs: python-device-sdk (TCP transport, discovery file, re-read on 401); the MSI `Miru Clients` group and read ACE on `ProgramData\Miru\device-api` (until then only admins and SYSTEM can read the file on Windows); and a security scheme in the upstream device openapi spec.

## Context and Orientation

All paths are relative to the repo root `/home/ben/miru/workbench2/repos/agent`; the crate lives in `agent/`.

The **local device API** is an axum HTTP router (`agent/src/server/routes.rs`, `pub fn router(state: Arc<State>) -> Router`) with routes under `/v0.2` (`/health`, `/version`, `/device`, `/device/sync`, deployments, releases, git commits, and `GET /v0.2/events`, a Server-Sent Events (SSE) stream in `agent/src/server/sse.rs` that sends an immediate `: heartbeat` comment frame). `router()` applies shared middleware in the private fn `middleware()`: an activity-tracker `from_fn` (touches the idle timer) and a `tower_http` `TraceLayer` whose span includes all request headers (`DefaultMakeSpan::new().include_headers(true)`), so an `Authorization` header would be logged unless its `HeaderValue` is marked sensitive (`set_sensitive(true)`, which makes Debug print `Sensitive`; precedent `agent/src/gcs/mod.rs:77`).

Two **transports** serve that router. `agent/src/server/unix.rs` (unix only) serves the Unix socket `/run/miru/miru.sock`, access-controlled by file mode 0660 group `miru`. `agent/src/server/tcp.rs` has `pub async fn bind(port) -> Result<TcpListener, ServerErr>` (127.0.0.1 only; port 0 = OS-assigned) and a non-async `pub fn serve(listener, state, shutdown_signal) -> JoinHandle<Result<(), ServerErr>>` that wraps the router in `check_loopback` (403 unless Host is `127.0.0.1:<port>`/`localhost:<port>` and any Origin matches). In axum the last `.layer(...)` added is the outermost. `tcp.rs` has inline `#[cfg(test)] mod tests` that build a small `Router` and drive it with `tower::ServiceExt::oneshot`.

`agent/src/server/mod.rs` declares the modules and `DEFAULT_ENABLE_TCP_SERVER = cfg!(windows)` (unchanged), with a doc comment (L16-19) saying TCP "has no auth yet". `agent/src/server/errors.rs` defines leaf error structs (e.g. `BindTcpListenerErr { addr, source, trace: Box<Trace> }` with `#[derive(Debug, thiserror::Error)]` and `impl crate::errors::Error`), the `ServerErr` enum, and a `crate::impl_error!(ServerErr { ... })` list; a new leaf needs all three. `agent/src/server/state.rs` `State` derives `Debug`.

`agent/src/app/run.rs` starts everything: `init_optional_services` calls `init_local_api_servers` when either transport is enabled; that fn returns early on Windows with a warning if TCP is off, then `init_tcp_server` binds (bind failure → `error!("Failed to start tcp server, continuing without it: {e}")`, return `Ok(())`) and calls `tcp::serve`, registering the handle with `ShutdownManager` (`with_tcp_server_handle`). `ShutdownManager::shutdown` joins server handles in `shutdown_impl`; if teardown exceeds `max_shutdown_delay` it calls `std::process::exit(1)`, skipping cleanup. Options live in `agent/src/app/options.rs` (`AppOptions { storage: StorageOptions { layout, .. }, enable_tcp_server, server: server::Options { socket_file, tcp_port }, lifecycle, .. }`), logged with `{:?}` in `agent/src/main.rs`.

`agent/src/disk/layout.rs` `Layout` maps the data root (`/var/lib/miru` on Linux, `C:\ProgramData\Miru` on Windows) to paths via methods like `settings()`, `device()`. Tests in `agent/tests/disk/layout.rs` (`mod storage_layout`, helper `under_root(&[..])`, one test per method).

`agent/src/filesys/` provides `files::write_json(file, &obj, WriteOptions { overwrite: Overwrite::Allow|Deny, atomic: Atomic::Yes|No, mode: Option<u32> })` (creates the parent dir; atomic = temp file in the same dir + rename; `mode` applied at temp-file creation and ignored on Windows, where the file inherits the directory ACL), `files::read_json`, and `files::delete` (NotFound is `Ok`). Precedent: `agent/src/crypt/rsa.rs` writes the public key with mode `0o640`.

Crates already in `agent/Cargo.toml`: `aws-lc-rs` (`rand::fill`, `constant_time::verify_slices_are_equal`), `secrecy` (`SecretString` redacts in Debug; `ExposeSecret`), `axum` 0.8, `tower`, `serde`/`serde_json`; dev: `reqwest` (`Response::chunk`), `serial_test`. `crate::crypt::base64::encode_bytes_url_safe_no_pad` encodes 32 bytes to 43 chars. No new dependencies.

Tests: integration tests compile into one target named `mod` (`agent/tests/mod.rs`, mirroring `src/`); `agent/tests/server/mod.rs` declares submodules. `agent/tests/server/tcp.rs` has a `Fixture` (real `State`, `shutdown_tx`), `no_proxy_client()`, `mod bind`, and `mod serve` (`serves_routes_over_loopback`, `rejects_foreign_host`). `agent/tests/app/run.rs` has `#[serial]` run tests (`HANG_GUARD` 60s, `NEVER`, `SHUTDOWN_WATCHDOG`, `prepare_valid_server_storage`), including `max_runtime_reached` (TCP on, port 0) and `tcp_port_in_use_does_not_abort_startup`. Conventions: import groups `// standard crates` / `// internal crates` / `// external crates`; production fn bodies ≤ 50 lines (funclen lint); no runs of ≥4 field-by-field `assert_eq!`; mode-bit asserts only under `#[cfg(unix)]` (precedent `agent/tests/crypt/rsa.rs`); CI `windows-check` runs the full test suite on Windows, so everything else must be portable. Coverage gates: `agent/src/server/.covgate` 87, `agent/src/app/.covgate` 90.38, `agent/src/disk/.covgate` 96.79.

## Plan of Work

**M1 — token, bearer middleware, redaction.** Add `GenerateTokenErr { trace: Box<Trace> }` to `agent/src/server/errors.rs` (`#[error("failed to generate local device API token")]`, `impl crate::errors::Error`), a `ServerErr::GenerateTokenErr` variant, and an `impl_error!` entry. Create `agent/src/server/auth.rs` and declare `pub mod auth;` in `mod.rs`. `Token` wraps `secrecy::SecretString` and derives only `Debug` (secrecy prints `[REDACTED]`). `Token::generate()` fills `[0u8; 32]` with `aws_lc_rs::rand::fill`, maps failure to `GenerateTokenErr`, encodes with `encode_bytes_url_safe_no_pad`. `Token::expose(&self) -> &str` returns the secret (used by `authorized`, `discovery::write`, and tests). `pub async fn check_bearer(token: Arc<Token>, req: Request, next: Next) -> Response` (pub, so the lib target has no dead-code warning before M2 wires it) reads `AUTHORIZATION`, requires `to_str()` to succeed, `split_once(' ')`, scheme `eq_ignore_ascii_case("Bearer")`, then `verify_slices_are_equal(credentials.as_bytes(), token.expose().as_bytes())`; any failure → `warn!("Rejected local device API request without a valid bearer token")` (never the credential) and a bare `(StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")]).into_response()`; success → `next.run(req).await`. Keep parsing in a small helper `fn authorized(req: &Request, token: &Token) -> bool`. Inline `#[cfg(test)] mod tests` follow the `tcp.rs` Router + `oneshot` pattern.

In `agent/src/server/routes.rs` `middleware()`, add as the first (outermost) `ServiceBuilder` layer `from_fn(redact_authorization)`, where `async fn redact_authorization(mut req: Request, next: Next) -> Response` marks every `AUTHORIZATION` value sensitive (`if let Entry::Occupied(mut e) = req.headers_mut().entry(AUTHORIZATION) { for v in e.iter_mut() { v.set_sensitive(true); } }`, with `axum::http::header::Entry`), so a repeated header is redacted too. This redacts the header in `TraceLayer` spans on both transports. It is tested through the real router (Validation), so a layer-order regression fails.

**M2 — discovery file and serve wiring.** In `agent/src/disk/layout.rs` add `device_api_dir()` = `root().subdir("device-api")` and `device_api_discovery()` = `device_api_dir().file("device-api.json")`. Create `agent/src/server/discovery.rs` (`pub mod discovery;`): `pub const SCHEMA_VERSION: u32 = 1;`, private `#[derive(Serialize)] struct Discovery<'a> { schema_version: u32, port: u16, token: &'a str }` (no Debug), `write(file, port, token)` via `files::write_json` with `WriteOptions { overwrite: Overwrite::Allow, atomic: Atomic::Yes, mode: Some(0o640) }`, and `remove(file)` via `files::delete`; both map `FileSysErr` into `ServerErr` with `?`.

Rewrite `tcp::serve` as async (signature under Interfaces). Order: `listener.local_addr()` → port (error → `RunAxumServerErr`, as today); `Token::generate()?`; `discovery::write(&discovery_file, port, &token).await?` (on error the listener drops and nothing is served); build the app in a helper `fn app(state, port, token: Arc<Token>) -> Router` as `routes::router(state).layer(from_fn(move |req, next| check_bearer(token.clone(), req, next))).layer(from_fn(move |req, next| check_loopback(port, req, next)))`; spawn a helper `async fn run(listener, app, shutdown_signal, discovery_file) -> Result<(), ServerErr>` that awaits `axum::serve(...).with_graceful_shutdown(...)`, then calls `discovery::remove` (on error `warn!("Failed to remove discovery file: {e}")`), then returns the serve result. Every fn stays under 50 lines. In the same milestone update the only production caller, `init_tcp_server` in `agent/src/app/run.rs`: pass `options.storage.layout.device_api_discovery()` to `tcp::serve(...).await`; on `Err(e)` log `error!("Failed to start tcp server, continuing without it: {e}")` and return `Ok(())` (bind-failure handling unchanged), so the crate builds at the end of M2. Update `agent/tests/server/tcp.rs` call sites and add tests (Validation).

**M3 — stale-file removal.** In `agent/src/app/run.rs` add `async fn remove_stale_discovery_file(layout: &Layout)` that calls `server::discovery::remove(&layout.device_api_discovery())` and `warn!`s on error; call it as the first line of `init`, before `init_app_state`. This one call covers TCP off on Linux, both transports off, the Windows early return, an early init failure (for example invalid storage), and leftovers from a crash or the shutdown-timeout `process::exit`. `DEFAULT_ENABLE_TCP_SERVER` is unchanged.

**M4 — docs.** `ARCHITECTURE.md`: extend the `server` paragraph (L46) with the TCP bearer auth and discovery-file behavior described in Purpose and the Decision Log (401 + `WWW-Authenticate: Bearer` after the loopback 403, per-OS path, `{schema_version, port, token}`, mode 0640, lifecycle, clients re-read on 401, Unix socket unauthenticated); add `device-api/device-api.json` to the on-disk file lists (L76, L113) and correct `storage::Layout` → `disk::Layout` (L76, L102, L113). Replace the "no auth yet" wording in `agent/src/server/mod.rs` (L16-19) and extend the `enable_tcp_server` doc in `agent/src/disk/settings.rs` (L18-19) to mention bearer auth and the discovery file. In `plans/active/20260910-windows-support.md` mark the PR 12 heading `(done — \`server/auth.rs\`, \`server/discovery.rs\`)`, rewrite its paragraph to what shipped (keep the python-device-sdk and MSI follow-up notes), and copy this plan's Decision Log items into its `## Decision log` using that file's `- <date>: <decision>. Rationale: ...` format.

**M5 — validation and completion.** Run preflight (Concrete Steps M5) until it reports `CLEAN`. Then, in one plan-only commit, tick M5 with the green SHA, fill Surprises and Outcomes & Retrospective (including any CI fixes), and move the plan to `plans/completed/`; push it and confirm CI is green on that new head before the PR leaves draft.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent`.

M0:

    mv plans/backlog/20260930-tcp-token-auth-discovery-file.md plans/active/
    git add plans/active/20260930-tcp-token-auth-discovery-file.md
    git commit -m "docs(plans): activate tcp token auth plan"

M1 (after edits):

    RUST_LOG=off cargo test -p miru-agent --lib server::auth
    RUST_LOG=off cargo test -p miru-agent --test mod server::tcp::redaction
    ./scripts/lint.sh
    git add -A && git commit -m "feat(server): add bearer token auth middleware"

Expected: `server::auth` reports `test result: ok. 12 passed` (the tests listed under Validation); `server::tcp::redaction` reports `test result: ok. 1 passed`. Run `./scripts/lint.sh` before every milestone commit and re-stage what it reformats; every milestone commit must be lint-clean.

M2:

    RUST_LOG=off cargo test -p miru-agent --test mod server::tcp
    RUST_LOG=off cargo test -p miru-agent --test mod server::discovery
    RUST_LOG=off cargo test -p miru-agent --test mod disk::layout
    ./scripts/lint.sh
    git add -A && git commit -m "feat(server): require bearer token and write discovery file on tcp"

Expected: each reports `test result: ok.`; `server::tcp` includes the new `serve::*` tests listed under Validation (e.g. `serve::rejects_missing_token`, `serve::discovery_write_failure_errors`); `server::discovery` reports `6 passed` (5 on Windows); `disk::layout` includes `device_api_dir` and `device_api_discovery`.

M3:

    RUST_LOG=off cargo test -p miru-agent --test mod app::run
    ./scripts/lint.sh
    git add -A && git commit -m "feat(app): manage tcp discovery file across agent lifecycle"

Expected: `test result: ok.`, including `tcp_requires_bearer_and_cleans_up_discovery_file`, `discovery_write_failure_does_not_abort_startup`, `stale_discovery_file_removed_when_tcp_disabled`, and the extended `tcp_port_in_use_does_not_abort_startup`.

M4:

    ./scripts/lint.sh
    git add -A && git commit -m "docs: document tcp bearer auth and discovery file"

Expected: lint exits 0 with no files rewritten.

M5:

Run preflight (the `preflight` skill). It pushes the branch, opens a draft PR against `main` if none exists (`ci.yml` runs only on pull requests and pushes to `main`), watches CI (`lint`, `test` with coverage gates, `tools`, `windows-check`), fixes failures from CI logs in follow-up commits, and must report `CLEAN`. By hand:

    git push -u origin feat/tcp-token-auth
    gh pr create --draft --base main --fill
    gh pr checks --watch

Fixes found by preflight or CI go in follow-up commits (e.g. `fix(server): ...`), not amends. Once CI is green, edit the plan (tick M5 with the green SHA, fill Surprises and Outcomes), then:

    git mv plans/active/20260930-tcp-token-auth-discovery-file.md plans/completed/
    git add plans/completed/20260930-tcp-token-auth-discovery-file.md
    git commit -m "docs(plans): complete tcp token auth plan"
    git push
    gh pr checks --watch                  # must be green on this head
    gh pr ready                           # only once CI is green on the final head

## Validation and Acceptance

**preflight must report CLEAN (CI green on the pushed branch head, including windows-check) before the PR leaves draft or the task is reported complete.**

Unit tests inline in `agent/src/server/auth.rs` (`cargo test -p miru-agent --lib server::auth`):

- `generate_yields_43_char_base64url`: `expose().len() == 43` and every char is `[A-Za-z0-9_-]`.
- `generate_is_unique`: two `Token::generate()` values differ.
- `debug_redacts_secret`: `format!("{token:?}")` does not contain `token.expose()`.
- `check_bearer` via Router + `oneshot`: `missing_header_is_401` (also asserts `WWW-Authenticate: Bearer`), `basic_scheme_is_401`, `wrong_token_is_401`, `different_length_token_is_401`, `empty_credentials_is_401` (`"Bearer "`), `non_ascii_header_is_401` (`HeaderValue::from_bytes(b"Bearer \xff")`), `correct_token_is_200`, `lowercase_scheme_is_200`.

Log redaction, in a new `pub mod redaction` at the end of `agent/tests/server/tcp.rs` (a sibling of `mod bind`/`mod serve`; written in M1; it uses only `Fixture` and `routes::router`): `authorization_header_is_redacted_in_trace_spans` makes `CapturingWriter` in `agent/tests/logs/mod.rs` `pub(crate)` (with a `pub(crate)` field) and imports it, installs `tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).with_writer(writer).finish()` via `tracing::subscriber::set_default` (the default current-thread `#[tokio::test]` keeps it in scope across `.await`), sends `GET /v0.2/health` with two `Authorization` headers (`Bearer not-a-real-secret` and `Bearer also-not-a-secret`, via two `.header()` calls) through `routes::router(fixture.state.clone())` with `tower::ServiceExt::oneshot`, and asserts the captured output contains `Sensitive` and contains neither secret. It fails if `redact_authorization` is removed or moved inside `TraceLayer`.

Integration tests in `agent/tests/server/tcp.rs` (`--test mod server::tcp`). Add `discovery_file` (`dir.dir().subdir("device-api").file("device-api.json")`) to `Fixture`, a helper that reads it with `files::read_json::<serde_json::Value>` and returns the token, and a helper that starts serve on `tcp::bind(0)`. Update `serves_routes_over_loopback` and `rejects_foreign_host` to the new signature (`.await.unwrap()`, send the Bearer header where a 200 is expected). New tests in `mod serve`:

- `rejects_missing_token` → 401; `rejects_wrong_token` → 401; `accepts_valid_bearer` → 200 on `/v0.2/health`.
- `foreign_host_with_valid_token_is_403` (loopback runs before auth).
- `sse_streams_heartbeat_with_token`: `GET /v0.2/events` with Bearer → 200, `content-type` starts with `text/event-stream`, first `response.chunk()` (under `tokio::time::timeout(5s)`) contains `heartbeat`; then `shutdown_tx.send(())` and the join handle completes `Ok` within a timeout.
- `sse_without_token_is_401`.
- `discovery_file_contents`: `schema_version == 1`, `port ==` the listener's bound port, token is 43 chars and is the one `accepts_valid_bearer` uses.
- `#[cfg(unix)] discovery_file_mode_is_0640`: `metadata.permissions().mode() & 0o777 == 0o640`.
- `discovery_file_overwrites_atomically`: pre-write junk to the file, serve, file parses with the new port, and `std::fs::read_dir` of the parent lists only `device-api.json`.
- `restart_rotates_token`: two serve starts (shutdown between) yield different tokens.
- `discovery_file_removed_after_shutdown`.
- `discovery_write_failure_errors`: discovery file whose parent path is an existing regular file → `serve` returns `Err` and a connection to the listener's old address is refused.

New `agent/tests/server/discovery.rs` (declare `pub mod discovery;` in `agent/tests/server/mod.rs`): `write_then_remove` (file exists, parses, then gone) and `remove_missing_is_ok`.

`agent/tests/disk/layout.rs`: `device_api_dir` (`under_root(&["device-api"])`) and `device_api_discovery` (`under_root(&["device-api", "device-api.json"])`).

Run-level tests in `agent/tests/app/run.rs` (`#[serial]`, `--test mod app::run`):

- `tcp_requires_bearer_and_cleans_up_discovery_file`: `enable_tcp_server: true`, `server: Options { socket_file: /tmp/miru.sock, tcp_port: 0 }` (as the other run tests; the default `/run/miru/miru.sock` is not writable in CI), `is_persistent: true`, `idle_timeout: NEVER`, `max_shutdown_delay: SHUTDOWN_WATCHDOG`; spawn `run(options, async move { let _ = rx.await; })` with a `tokio::sync::oneshot` shutdown (as `shutdown_signal_received`); poll (≤10s, 20ms steps) for `layout.device_api_discovery()`; read port and token; health without header → 401, with Bearer → 200; send shutdown; `run` returns `Ok` within `HANG_GUARD`; file no longer exists.
- Extend `tcp_port_in_use_does_not_abort_startup`: before `run`, pre-write a stale file at `layout.device_api_discovery()`; after `run` returns `Ok`, the file no longer exists (a failed bind never leaves a token file behind).
- Extend `invalid_app_state_initialization`: pre-write a stale file at `layout.device_api_discovery()`; after `run` returns `Err`, the file no longer exists.
- `discovery_write_failure_does_not_abort_startup`: write a regular file at `layout.root().file("device-api")` so the discovery dir cannot be created, then run with `enable_tcp_server: true`, `tcp_port: 0` and `max_runtime` 100ms (as `max_runtime_reached`); `run` returns `Ok` (the `tcp::serve` error is logged, not propagated).
- `stale_discovery_file_removed_when_tcp_disabled`: pre-write a file at `device_api_discovery()`, run with `enable_tcp_server: false` and `max_runtime` 100ms (as `max_runtime_reached`), assert the file is gone after `run` returns.

All tests are portable except the mode assertion. The full suite, lint and coverage gates (server ≥ 87, app ≥ 90.38, disk ≥ 96.79) run in CI during M5: `lint` runs `LINT_FIX=0 ./scripts/lint.sh`, `test` runs `./scripts/covgate.sh`, and `windows-check` runs `cargo test --package miru-agent` on Windows.

End to end, `tcp_requires_bearer_and_cleans_up_discovery_file` runs the full agent `run` against a temp data root. On an activated Linux device with this build installed, reproduce the Purpose transcript by setting `"enable_tcp_server": true` in `/var/lib/miru/settings.json`, running `sudo systemctl restart miru`, and reading the token with `sudo jq -r .token /var/lib/miru/device-api/device-api.json`; `sudo stat -c %a` on that file prints `640`, and after `sudo systemctl stop miru` the file is gone.

## Idempotence and Recovery

All edits are source and test changes; re-running tests and lint is safe. `scripts/lint.sh` rewrites formatting in place — re-stage after it. If a milestone commit is wrong, fix forward with a new commit. At runtime the design is self-healing: the discovery write is atomic (never a half-written file), a stale file from a crash or shutdown-timeout exit is removed at the next start, and every discovery or token failure degrades to "agent runs without TCP" rather than aborting. On Windows, delete or rename can fail with a sharing violation if a client holds the file open; removal logs a warning, and a failed write at startup leaves TCP off until the next start (clients should open, read, and close the file promptly).

Risk: until the MSI follow-up adds the `Miru Clients` read ACE, only Administrators, SYSTEM, and the service account can read the file on Windows, so non-admin clients cannot authenticate (secure by default).

## Interfaces and Dependencies

No new crates. New or changed signatures:

    // agent/src/server/auth.rs
    #[derive(Debug)]
    pub struct Token(secrecy::SecretString);
    impl Token {
        pub fn generate() -> Result<Token, ServerErr>;
        pub fn expose(&self) -> &str;
    }
    pub async fn check_bearer(token: Arc<Token>, req: Request, next: Next) -> Response;

    // agent/src/server/discovery.rs
    pub const SCHEMA_VERSION: u32 = 1;
    pub async fn write(file: &filesys::File, port: u16, token: &Token) -> Result<(), ServerErr>;
    pub async fn remove(file: &filesys::File) -> Result<(), ServerErr>;

    // agent/src/server/tcp.rs
    pub async fn serve(
        listener: TcpListener,
        state: Arc<State>,
        discovery_file: filesys::File,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> Result<JoinHandle<Result<(), ServerErr>>, ServerErr>;

    // agent/src/disk/layout.rs
    pub fn device_api_dir(&self) -> filesys::Dir;          // <root>/device-api
    pub fn device_api_discovery(&self) -> filesys::File;   // <root>/device-api/device-api.json

    // agent/src/server/errors.rs
    pub struct GenerateTokenErr { pub trace: Box<Trace> }  // + ServerErr::GenerateTokenErr, impl_error! entry

Discovery file format (pretty JSON):

    {
      "schema_version": 1,
      "port": 6478,
      "token": "<43-char base64url>"
    }
