# Architecture

This document describes the high-level architecture of the Miru Agent. If you want to familiarize yourself with the codebase, you are in the right place.

## Bird's Eye View

The agent is a Rust binary that runs on customer devices (robots). It solves one core problem: keeping device configurations in sync with what the user defined in the Miru platform. It pulls configuration deployments from the Miru backend, applies them to a target directory on disk, and reports status back. It also exposes a local Unix socket server so that on-device applications can query the device state.

The binary has two mutually exclusive modes, selected at startup:

- **Installer mode** (`--install`): activates a new device by reading an activation token from the environment, registering with the backend, and writing device identity and auth files to disk.
- **Agent runtime mode** (default): reads settings from disk, initializes shared state (AppState), starts background workers (MQTT subscriber, poller, token refresh), serves a local HTTP server, and waits for a shutdown signal.

These modes do not share runtime state.

## Codemap

All source lives under `agent/src/`. The binary entry point is `main.rs`.

### Core infrastructure

`cli` — command-line argument parsing. Determines installer vs runtime mode.

`errors` — custom Error trait with `code()`, `http_status()`, `params()`, `is_network_conn_err()` methods. All error types derive `thiserror::Error`. Aggregating enums use the `impl_error!` macro defined here.

`filesys` — file, directory, and path abstractions. Types `Dir`, `File`, and the `PathExt` trait.

`logs` — tracing-subscriber setup with file rotation. Configured via `logs::Options`.

`models` — shared data types (Device, Deployment, Release, etc.).

`version` — build-time version string. Embedded by `build.rs` from git commit hash and build date.

### Networking

`http` — reqwest-based HTTP client with configurable retry and backoff. Type `http::Client`. All backend API calls go through this; it handles auth headers automatically.

`mqtt` — rumqttc-based MQTT subscriber. Listens for real-time events from the backend (e.g., new deployment available) so the agent can react immediately instead of waiting for the next poll.

`server` — axum HTTP server on a Unix socket (`/tmp/miru.sock`). Exposes device state, health, and action endpoints for the CLI and frontend. Route handlers live in `server/handlers.rs`.

### Security

`authn` — JWT token lifecycle. Type `TokenManager` handles background refresh and persistence via `TokenFile`. Spawns as a background task; communicates via channels.

`crypt` — RSA key handling and JWT creation/parsing. Types `jwt::Claims`, RSA key loading functions.

### Business logic

`sync` — orchestrates full state synchronization with the backend. Type `Syncer` is the main coordination point; it fetches device state, identifies needed deployments, and drives the deploy pipeline.

`deploy` — deployment state machine. The FSM in `deploy/fsm` manages the lifecycle: download artifacts to a staging directory, apply to the target config directory, report status. `deploy/apply` handles the actual file operations.

`services/` — domain service layer. Submodules: `device` (device status sync), `deployment` (deployment management), `git_commit` (commit tracking), `release` (release management).

`cache` — file-system-backed cache with TTL. Used for caching backend responses.

`cooldown` — exponential backoff. Type `Backoff` with configurable base, growth factor, and max.

### Observability

`telemetry` — OpenTelemetry integration.

`activity` — tracks last-active timestamps. Type `Tracker`. Used for idle detection in non-persistent mode.

### Persistence

`storage` — on-disk state management. `storage::Layout` defines the directory structure. `storage::Storage` wraps per-entity stores with capacity limits. Key files on disk: `settings.json`, `device.json`, `auth/` (private key and token).

### Background workers

`workers/` — three long-running tasks:
- `mqtt` — subscribes to MQTT topics, triggers sync on events.
- `poller` — periodic backend sync on a timer.
- `token_refresh` — rotates JWT before expiry.

All workers receive a broadcast shutdown signal and clean up gracefully.

### Device setup

`installer` — interactive activation flow. Reads activation token from environment, calls backend to register the device, writes device identity and auth credentials to disk. Display helpers in `installer/display`.

### Generated code (workspace siblings)

`libs/openapi-client` and `libs/openapi-server` are auto-generated from OpenAPI specs in `api/`. Never edit these by hand. Regenerate with `make -C api` or `api/regen.sh`.

## Architectural Invariants

- **Installer and runtime are mutually exclusive.** `main.rs` picks one path at startup. They share no runtime state; installer writes the files that runtime later reads.
- **All backend HTTP goes through `http::Client`.** No module uses raw reqwest. The client handles retry logic and attaches auth headers.
- **Shutdown ordering matters.** Syncer shuts down before storage (it writes during sync). Token manager shuts down last. This is enforced in `AppState::shutdown()`.
- **Generated code is never hand-edited.** `libs/openapi-client` and `libs/openapi-server` are overwritten on regeneration.
- **Tests require `--features test` and `--test-threads=1`.** This is a hard constraint. Many test helpers are behind `#[cfg(feature = "test")]` and tests share `/tmp/miru.sock`.
- **The agent has no direct database.** All persistence is file-based via `storage::Layout`. The backend owns the database.

## Cross-Cutting Concerns

**Error handling.** Every module defines its errors in an `errors.rs` file. Leaf errors derive `thiserror::Error` and implement the custom `crate::errors::Error` trait (which provides default implementations for the common case). Aggregating enums use `impl_error!` to forward trait methods to inner variants.

**Graceful shutdown.** `app/run.rs` creates a `tokio::sync::broadcast` channel. All workers and the HTTP server subscribe to it. On SIGTERM/SIGINT/ctrl-c, the channel fires and each component drains in-flight work before exiting. AppState components shut down in dependency order.

**Authentication.** JWT-based. The `TokenManager` runs as a background task, refreshing the token before expiry using the device's RSA private key. `http::Client` reads the current token from `TokenManager` for every request. Token persistence is via `TokenFile` (atomic writes to disk).

**Storage.** `storage::Layout` defines where everything lives on disk (default: `/var/lib/miru/`). `storage::Storage` provides typed stores for devices, deployments, releases, and settings, each with configurable capacity limits.
