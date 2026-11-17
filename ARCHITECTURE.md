# Architecture

This document describes the high-level architecture of the Miru Agent. If you want to familiarize yourself with the codebase, you are in the right place. It says what each part does and how the parts fit together; the details live in the code.

## Bird's Eye View

The agent is a Rust binary that runs on customer devices (robots). It solves one core problem: keeping device configurations in sync with what the user defined in the Miru platform. It pulls configuration deployments from the control plane, applies them to a target directory on disk, and reports status back. It also serves a local Device API so that on-device applications can query and act on the device state.

The binary has two mutually exclusive modes, selected at startup:

- **Provision mode** (`--provision`): activates a new device by registering it with the control plane and writing its identity and auth files to disk. `--check` instead reports, offline, whether the device is provisioned.
- **Agent runtime mode** (default): loads settings and shared state (`AppState`), starts the background workers and the Device API server, and runs until a shutdown signal. On Windows it runs as a service unless `--console` is passed.

## Codemap

All source lives under `daemon/src/`. The binary entry point is `main.rs`.

### Core infrastructure

`cli` — command-line argument parsing.

`errors` — the shared `Error` trait and the `impl_error!` macro.

`filesys` — file, directory, and path abstractions.

`logs` — tracing setup with file rotation.

`models` — shared data types (Device, Deployment, Release, etc.).

`platform` — per-OS defaults, such as the data root and log directory.

`shutdown` — the process-wide stop latch and the agent's run outcome.

`windows` — Windows Service Control Manager integration.

`version` — build-time version string.

### Networking

`http` — the HTTP client for the control plane, with retry, backoff, and auth headers.

`mqtt` — MQTT subscriber for real-time events from the control plane, so the agent reacts without waiting for the next poll.

`server` — the local Device API for on-device applications, the CLI, and the frontend. One router is served over a Unix socket and, on Windows by default, a loopback-only TCP listener that requires a bearer token published in a discovery file. Each Device API version (`v02`, `v03`) is its own module over its generated crate, which nothing outside that module imports; code that builds no wire type lives in `server/` and is shared by the versions.

### Security

`authn` — JWT lifecycle: background refresh and persistence of the device token.

`crypt` — RSA key handling and JWT creation and parsing.

### Business logic

`sync` — orchestrates synchronization with the control plane: fetches device state, finds the deployments needed, and drives the deploy pipeline.

`deploy` — deployment state machine: downloads artifacts to staging, applies them to the target config directory, and reports status.

`data_uploads` — the file-upload pipeline: scans configured directories for stable files, uploads them to object storage, and deletes them once their retention expires.

`queue` — a generic, persisted job queue with backoff and a shared retry policy, used by the uploaders.

`calibrations` — calibration reports from the Device API. Reports are buffered on disk and uploaded to the control plane in the background, so callers never wait on the network. A local store keeps the latest calibration for each component, so apps can read it, or mark it invalid, offline; at startup it pulls any newer calibrations from the control plane.

`services` — domain service layer between the server and the stores (calibration, device, deployment, file rule, git commit, release).

`cache` — file-system-backed cache with TTL for control-plane responses.

`cooldown` — exponential backoff.

`clock` — time sources that don't trust the robot's system clock: monotonic time and server time from control-plane responses.

### Observability

`telemetry` — OpenTelemetry integration.

### Persistence

`disk` — on-disk state: the directory layout and typed stores with capacity limits.

### Background workers

`workers` — long-running tasks, such as the MQTT subscriber, the periodic poller, and token refresh. It also drives the `data_uploads` scanner and deleter, which don't schedule themselves. All of them stop on the shared shutdown signal.

### Device setup

`provisioning` — registers the device with the control plane and writes its identity and credentials to disk.

### Generated code (workspace siblings)

`libs/backend-api` and the `libs/device-api-vNN` crates are generated from the OpenAPI specs in `api/specs/`, which are vendored from [`mirurobotics/openapi`](https://github.com/mirurobotics/openapi). Change the specs there, copy them here, and run `api/regen.sh`.

## Architectural Invariants

- **Provision and runtime are mutually exclusive.** They share no runtime state; provision writes the files that runtime later reads.
- **All control-plane HTTP goes through `http::Client`.** No module uses raw reqwest.
- **Shutdown ordering matters.** `AppState::shutdown()` stops each component before the ones it depends on.
- **Generated code is never hand-edited.**
- **Tests exercise ordinary production behavior.** Production code is identical in normal and test builds; tests control dependencies and time. See `AGENTS.md` § Testing.
- **The agent has no direct database.** All persistence is file-based; the control plane owns the database.
- **The agent always runs persistently.** It exits only on a shutdown signal or a fatal startup error.
- **The Device API never listens beyond the device.** TCP binds only to loopback.
- **State that survives an upgrade changes only by adding fields.** Files that `reset` keeps, such as the calibration queue and latest store, stay readable by newer agents and by an agent rolled back to.

## Cross-Cutting Concerns

**Error handling.** Each module defines its errors in an `errors.rs` file, implementing the shared `Error` trait; aggregating enums use `impl_error!`.

**Actors.** Stateful components such as the syncer, token manager, caches and uploaders are actors: a single-threaded `*_impl` module holds the logic, a worker task runs it, and callers talk to it through a cloneable handle.

**Graceful shutdown.** OS signals, and the service stop on Windows, resolve one shutdown future; a broadcast channel then tells every worker and server to drain in-flight work and exit.

**Authentication.** The device signs in with its RSA key and gets a JWT, which `authn` refreshes in the background and `http::Client` attaches to every request.

**Storage.** `disk::Layout` defines where state lives (`/var/lib/miru/` on Linux, `%ProgramData%\Miru\Agent` on Windows). Directory permissions, not per-file modes, keep state and logs private.
