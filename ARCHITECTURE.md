# Architecture

This document describes the high-level architecture of the Miru Agent. If you want to familiarize yourself with the codebase, you are in the right place.

## Bird's Eye View

The agent is a Rust binary that runs on customer devices (robots). It solves one core problem: keeping device configurations in sync with what the user defined in the Miru platform. It pulls configuration deployments from the Miru backend, applies them to a target directory on disk, and reports status back. It also exposes a local Unix socket server so that on-device applications can query the device state.

The binary has two mutually exclusive modes, selected at startup:

- **Provision mode** (`--provision`): activates a new device by reading a provisioning token from the environment, registering with the backend, and writing device identity and auth files to disk. Adding `--check` short-circuits into a local, offline, read-only probe that reports provisioning state through the exit code instead — `0` provisioned, `3` not provisioned, `1` state could not be determined (message on stderr).
- **Agent runtime mode** (default): reads settings from disk, initializes shared state (AppState), starts background workers (MQTT subscriber, poller, token refresh), serves a local HTTP server, and waits for a shutdown signal. On Windows it runs as service `miru-agent` under the Service Control Manager, as virtual account `NT SERVICE\miru-agent`, unless `--console` is passed, which keeps it in the foreground with ctrl-c shutdown; `--console` is accepted and ignored on Unix. Ctrl-c and SCM STOP share one `Latch` and abort startup upgrade between attempts (never mid-reset).

These modes do not share runtime state.

## Codemap

All source lives under `agent/src/`. The binary entry point is `main.rs`.

### Core infrastructure

`cli` — command-line argument parsing. Determines provision vs runtime mode and parses `--console` (foreground instead of Windows service).

`errors` — custom Error trait with `code()`, `http_status()`, `params()`, `is_network_conn_err()` methods. All error types derive `thiserror::Error`. Aggregating enums use the `impl_error!` macro defined here.

`filesys` — file, directory, and path abstractions. Types `Dir`, `File`, and the `PathExt` trait.

`logs` — tracing-subscriber setup with file rotation. Configured via `logs::Options`.

`models` — shared data types (Device, Deployment, Release, etc.).

`platform` — per-OS defaults (data root, log dir). OS-specific functions compile on every target so they are testable from any host.

`shutdown` — process-wide stop latch (`Latch`) and agent-body outcome (`RunOutcome`). Used on every OS; the Windows SCM maps `RunOutcome` to a service exit code.

`windows` — Windows Service Control Manager integration. `windows::scm` exposes `dispatch` and `run` (`cfg(windows)`); the binary owns `ServiceMain` next to the agent body. `windows::errors` compiles everywhere so Linux can test `NotLaunchedByScm`.

`version` — build-time version string. Embedded by `build.rs` from git commit hash and build date.

### Networking

`http` — reqwest-based HTTP client with configurable retry and backoff. Type `http::Client`. All backend API calls go through this; it handles auth headers automatically.

`mqtt` — rumqttc-based MQTT subscriber. Listens for real-time events from the backend (e.g., new deployment available) so the agent can react immediately instead of waiting for the next poll.

`server` — axum HTTP server exposing device state, health, and action endpoints for on-device applications, the CLI, and the frontend. One router (`server/routes.rs`) is served over two transports: a Unix socket (`/run/miru/miru.sock`, `server/unix.rs`, unix only) and, when `settings.enable_tcp_server` is set, IPv4 loopback TCP (`127.0.0.1:<port>`, `server/tcp.rs`, all platforms; on by default only on Windows; the default port is 6478, and port `0` is OS-assigned). A failed TCP bind is logged and the agent runs without the listener. Nothing is ever bound on a non-loopback interface. The TCP transport returns 403 unless the request host is `127.0.0.1:<port>` or `localhost:<port>`, and when an Origin header is present it must be that same loopback URL, which blocks DNS rebinding and cross-site requests from browsers on the device. After the loopback check, every TCP request (including `/v0.2/health` and `/v0.2/events`) must carry `Authorization: Bearer <token>`, else the agent returns 401 with `WWW-Authenticate: Bearer` (`server/auth.rs`, constant-time compare). The token is 32 random bytes encoded as 43 base64url characters, regenerated at every start, and kept out of `Debug` output and logs (Authorization headers are marked sensitive before request tracing on both transports). The app layer (`app/run.rs`) owns the token and the discovery file: after binding it mints the token, writes `{port, token}` atomically to the discovery file (`disk/device_api.rs`), and passes the token to `tcp::serve`, which only enforces it. The file is `/run/miru/device-api.json` on Linux (mode 0640, beside the socket, so clients do not read the private data root) and `C:\ProgramData\Miru\device-api\device-api.json` on Windows (readable through the installer-set directory ACL by SYSTEM, Administrators, the service, and members of the local `Miru Agent Users` group). The shutdown manager removes the file after the TCP server stops, and startup removes any stale copy, so no file means TCP is not being served. Clients read the port and token from the file and close it right away; they treat a missing file or a refused connection as not serving, and re-read the file on a 401, since the token rotates on restart. On Windows the agent retries the discovery write and removal briefly, because a reader holding the file open without delete sharing blocks both. A token or discovery-file failure is logged and the agent runs without the TCP listener. The Unix socket stays unauthenticated (file mode 0660, group `miru`). On Linux, systemd's `miru.socket` unit owns the Unix socket and hands it to the agent through socket activation (`LISTEN_FDS`); the agent binds the TCP listener itself. Route handlers live in `server/handlers.rs`.

### Security

`authn` — JWT token lifecycle. Type `TokenManager` handles background refresh and persistence via `TokenFile`. Spawns as a background task; communicates via channels.

`crypt` — RSA key handling and JWT creation/parsing. Types `jwt::Claims`, RSA key loading functions.

### Business logic

`sync` — orchestrates full state synchronization with the backend. Type `Syncer` is the main coordination point; it fetches device state, identifies needed deployments, and drives the deploy pipeline.

`deploy` — deployment state machine. The FSM in `deploy/fsm` manages the lifecycle: download artifacts to a staging directory, apply to the target config directory, report status. `deploy/apply` handles the actual file operations.

`data_uploads/` — parent module for the file-upload pipeline. Submodule `scan` watches rule-configured directories and emits stable files to sinks (the `StableFileSink` seam in `scan/sink.rs`); submodule `upload` queues and executes uploads to object storage; submodule `retention` queues files for deletion and deletes each once its TTL passes. In `retention`, the single-threaded queue state and sweep logic live in `DeleterImpl` (`deleter_impl.rs`), and `deleter.rs` wraps it with the actor worker and the `Deleter` handle. In `upload`, a non-terminal failure after an attempt of `LONG_ATTEMPT` or more, most likely from storage credentials that expired mid-transfer, is retried without counting as an attempt (`queue::RetryPolicy::on_uncounted_failure`). The drivers that schedule the scanner live in `workers/` (`scan.rs`, `sync_scan_bridge.rs`), not here.

`queue` — the generic persisted job queue. Type `FileQueue<J: QueueJob>` (`queue/file.rs`) is a FIFO with entry ids, attempt counts, `next_attempt_at` backoff and a capacity limit, kept in memory and saved as one atomically written JSON snapshot file. The upload and retention workers in `data_uploads/` each instantiate it with their own job type, and the calibration uploader uses a durable one. `FileQueue::next_attempt` gives a worker the ready entry, `Empty`, or how long to wait for the earliest deadline, after pulling in any deadline past a horizon. `queue::RetryPolicy` (`queue/retry.rs`) is a retry rule for uploaders, used by `data_uploads::upload` and `calibrations`: a network error isn't counted and waits a flat `base_secs`, while any other failure counts, backs off exponentially, and drops the job when terminal or out of attempts. Any failing job older than `max_age` is dropped. `on_failure` returns the `Verdict`; the uploader does its own logging and side effects.

`calibrations/` — calibration reports accepted from the Device API. `POST /v0.2/calibrations` (`server/handlers.rs`) checks only the request's shape and the reserved `null` role and aspect (else 422 `invalid_request`) and body size (a body over `routes::MAX_REQUEST_BYTES`, about 2 MiB and the limit for every route, is 413 `calibration_too_large`), then hands it to `services::calibration` and returns the report's `report_id` and `reported_at` without waiting on Miru. Each report's metadata (`ReportMetadata`, including the data's `InstanceFormat`) is queued in a durable `calibrations::Queue` (a `FileQueue`) at `calibrations/queue.json`, and its data is its own file, `calibrations/reports/<entry id>`, kept exactly as given, so large data is never rewritten with the queue. At most 1,000 reports of up to 1 MiB of data each (`UploaderOptions`). The queue entry is written before the report file and removed before it, so a crash leaves either an entry without a file, which the uploader drops when it reaches it, or a file without an entry, which `prune_orphans` deletes at startup. A report file that isn't UTF-8 is dropped like a report Miru rejects. `CalibrationsErr` maps a full queue to 507 `calibration_buffer_full` and an oversized report to 413 `calibration_too_large`. `calibrations::Uploader` is an actor that owns the buffer: callers send it reports, and it uploads them to Miru one at a time, oldest due first, serving new reports during an upload or a backoff so callers never wait on the network. Its single-threaded queue state, retry logic and `Sender` live in `UploaderImpl` (`uploader_impl.rs`), which the actor's worker runs. A failed report is retried at the tail of the queue after its own backoff, as in `data_uploads::upload`, so one failing report never blocks the others. Retries follow the default `queue::RetryPolicy` (10 s doubling to 1 h, at most 30 attempts, with network errors waiting a flat 10 s and not counted), except that a report is dropped only once it is still failing 90 days after `reported_at`, not 7: a calibration is costly to redo, and the queue cap already bounds its storage. Reports Miru permanently rejects are dropped with an error log. Without a queue file (`UploaderArgs::snapshot_file` is `None`) the queue lives in memory, so reports queued during that run are lost at a restart, and `prune_orphans` deletes nothing, so the report files that `queue.json` still names survive until a boot can open it; a corrupt `queue.json` is copied aside and started empty, as for every state file, and `prune_orphans` then deletes the report files it named. `HttpSender` (behind the `Sender` seam) sends one report as a single `POST /calibrations`. Each report's metadata carries a `report_id` minted at intake, sent on every retry and after a restart, so Miru records the calibration once; a token failure is never terminal, and a 2xx whose body cannot be parsed counts as sent. `AppState` spawns the calibration uploader at startup with an `HttpSender`, falling back to an in-memory queue if `queue.json` cannot be opened, and shuts it down first.

The calibration uploader's worker also writes the latest calibrations store (`calibrations::LatestStore`) beside `UploaderImpl`, and keeps the two in step itself, so `UploaderImpl` knows nothing of the store. The store is its own actor: its single-threaded logic lives in `LatestStoreImpl` (`latest_impl.rs`), and `latest.rs` wraps it with the worker and the cloneable `LatestStore` handle, as `uploader.rs` does for `UploaderImpl`. Reads go to the store directly, from `AppState::latest_cals`: `GET /v0.2/calibrations/latest` lists the entries its `component`, `role` and `aspect` filters match, and `GET /v0.2/calibrations/latest/{report_id}/content` returns an entry's data with its format's `Content-Type`, neither calling Miru. The store holds the newest report for each component, role and aspect in its own files, not the shared caches: `calibrations/latest/index.json` lists one entry per key under `entries`, with the store's `needs_pull` mark, an object as `queue.json` is, and `contents/` holds each entry's data in a file named by its `report_id`. It reads the index from disk on every call, retrying a failed read with a state file's delays unless the file is missing or unparseable. The worker is the store's only writer and serves one command at a time, and a put writes the content before the index. Intake writes a report through to the store before `POST` returns: the queue first, the store second, and a failed store write removes the queued report. An entry is the report's metadata; it does not track the upload. A newer report for the key replaces the entry and deletes its content. The store holds at most 1,000 keys (`calibrations::latest::DEFAULT_CAPACITY`); a report for a new key past that is 507 `calibration_store_full`, and its queued report is removed. When the store's worker starts, before it serves a command, it deletes the content no entry names, which a crash or a failed write or delete leaves; if the index can't be read, it deletes nothing. `AppState` spawns the store at startup, before the syncer, and shuts it down after the calibration uploader and the syncer. The store sits beside the upload queue, outside `resources/`, so `reset` keeps it and an upgrade leaves it as it is; its on-disk format must stay readable by newer agents, as `queue.json`'s must, and change only by adding fields, so an agent rolled back to still reads it. `null` is a reserved role and aspect, because the filters use it to match a missing one. Reports are created only on the device, but the store can still miss some: reprovision changes which device's data it should hold, and an agent that doesn't write it (an older one, or one rolled back to) takes reports that only Miru then has. So reprovision and the syncer pull it from Miru. A pull (`calibrations::pull` module) lists Miru's latest calibration for every key (`GET /calibrations/latest`), downloads each one's content, and puts it with `LatestStore::put_if_newer`, which the worker skips when the key's entry was reported at the same time or later, so reports Miru hasn't received yet are kept. Provision assumes a new device has no data, so it neither loads nor clears the store. Reprovision (`provisioning::reprovision`), after `bootstrap` installs the new keys, deletes `calibrations/`, upload queue included, so reports not yet uploaded are dropped and the previous device's calibrations never survive. It then issues a device token from the new keys and runs `calibrations::pull_latest` into the empty store. Like provisioning, it expects the agent stopped, as the install scripts' activation step does, because a running uploader would write its in-memory queue back. Nothing is skipped: a failed token, list or download, or more keys than the store holds, fails reprovision (`ProvisionErr::CalibrationsErr` for the pull), leaving the device reprovisioned with an empty or partial store until reprovision is rerun. Upgrade reconcile (`app::upgrade::reconcile_impl`), before `reset` writes the version marker, marks the store with `calibrations::mark_needs_pull`, which sets `needs_pull` in `index.json`, and merges in the reports still in the upload queue with `calibrations::merge_queued`, by the same rule, since the agent that queued them may not have written the store and Miru doesn't have them yet. Failures are logged, so they never keep the agent from starting. Each sync that reaches Miru then runs `calibrations::pull_if_needed`, best-effort as the system metadata sync is: if the store is marked, it pulls and clears the mark, and a failed pull leaves the mark for the next sync. Until that pull succeeds, the store can lack reports that only Miru has. Nothing else pulls the store.

`services/` — domain service layer. Submodules: `calibration` (calibration intake and the latest calibrations: stamps a report with the time and deployed deployment and queues it with the calibration uploader, and reads the latest store), `device` (device status sync), `deployment` (deployment management), `file_rule` (cached file-rule lookup), `git_commit` (commit tracking), `release` (release management).

`cache` — file-system-backed cache with TTL. Used for caching backend responses.

`cooldown` — exponential backoff. Type `Backoff` with configurable base, growth factor, and max.

`clock` — time sources that do not trust the robot's system clock. `Clock` gives monotonic elapsed time, server time anchored to the `Date` header of backend responses (`server_now`; `http::Client` parses the header with `http::date` and records it), and the system clock as-is for logs (`system_now`).

### Observability

`telemetry` — OpenTelemetry integration.

### Persistence

`disk` — on-disk state management. `disk::Layout` defines the directory structure. `disk::Storage` wraps per-entity stores with capacity limits. Key files on disk: `settings.json`, `device.json`, `auth/` (private key and token). The TCP discovery file (port and bearer token) is `/run/miru/device-api.json` on Linux and `%ProgramData%\Miru\device-api\device-api.json` on Windows.

### Background workers

`workers/` — three long-running tasks:
- `mqtt` — subscribes to MQTT topics, triggers sync on events.
- `poller` — periodic backend sync on a timer.
- `token_refresh` — rotates JWT before expiry.

All workers receive a broadcast shutdown signal and clean up gracefully.

### Device setup

`provision` — interactive provisioning flow. Reads activation token from environment, calls backend to register the device, writes device identity and auth credentials to disk. Display helpers in `provision/display`.

### Generated code (workspace siblings)

`libs/backend-api` and `libs/device-api` are auto-generated from OpenAPI specs in `api/specs/`. Never edit these by hand. The specs themselves are sourced from [`mirurobotics/openapi`](https://github.com/mirurobotics/openapi) — to change schemas, modify the source repo, regenerate the bundle there, then copy the updated spec here and run `api/regen.sh`. The vendored specs are the openapi release-stamped artifacts (they carry `x-release-version` and `x-git-commit`), not raw bundles.

## Architectural Invariants

- **Provision and runtime are mutually exclusive.** `main.rs` picks one path at startup. They share no runtime state; provision writes the files that runtime later reads.
- **All backend HTTP goes through `http::Client`.** No module uses raw reqwest. The client handles retry logic and attaches auth headers.
- **Shutdown ordering matters.** Syncer shuts down before storage (it writes during sync). The latest calibrations store shuts down after the calibration uploader and the syncer, which write it. Token manager shuts down last. This is enforced in `AppState::shutdown()`.
- **Generated code is never hand-edited.** `libs/backend-api` and `libs/device-api` are overwritten on regeneration.
- **Tests exercise ordinary production behavior.** `cargo test` needs no custom feature or logging environment; production algorithms and client construction are identical in normal and test builds, and tests control dependencies and time. Test layout conventions live in `AGENTS.md` § Testing.
- **The agent has no direct database.** All persistence is file-based via `disk::Layout`. The backend owns the database.
- **The agent always runs persistently.** It exits only on a shutdown signal or a fatal startup error. A `settings.json` that still contains `is_persistent` loads normally; the key is ignored.
- **Windows has no Unix socket.** On Windows the local device API is the TCP listener, which is on by default (`enable_tcp_server`). Startup warns and nothing listens when that flag is turned off.

## Cross-Cutting Concerns

**Error handling.** Every module defines its errors in an `errors.rs` file. Leaf errors derive `thiserror::Error` and implement the custom `crate::errors::Error` trait (which provides default implementations for the common case). Aggregating enums use `impl_error!` to forward trait methods to inner variants.

**Graceful shutdown.** `app/run.rs` creates a `tokio::sync::broadcast` channel. All workers and the HTTP server subscribe to it. SIGTERM/SIGINT/ctrl-c on Unix, ctrl-c in Windows console mode, and `SERVICE_CONTROL_STOP`/`SERVICE_CONTROL_SHUTDOWN` in Windows service mode (relayed through `shutdown::Latch`) all resolve the shutdown future `app/run.rs` awaits; the channel then fires and each component drains in-flight work before exiting. AppState components shut down in dependency order.

**Authentication.** JWT-based. The `TokenManager` runs as a background task, refreshing the token before expiry using the device's RSA private key. `http::Client` reads the current token from `TokenManager` for every request. Token persistence is via `TokenFile` (atomic writes to disk).

**Storage.** `disk::Layout` defines where everything lives on disk (default: `/var/lib/miru/`; `%ProgramData%\Miru\Agent` on Windows, with logs in `Agent\logs`). On Linux the data root is `0700` and `/var/log/miru` is `0750`; the folders, not per-file modes, keep state and logs private (see `build/debian/README.md`). The TCP discovery file is `/run/miru/device-api.json` on Linux and `%ProgramData%\Miru\device-api\device-api.json` on Windows, a sibling of the private data root. `disk::Storage` provides typed stores for devices, deployments, releases, and settings, each with configurable capacity limits.
