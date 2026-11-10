# Keep the app run tests off the production MQTT broker

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Point the MQTT worker of the agent started by `agent/tests/app/run.rs` at a loopback port nothing listens on, and set both test hosts only in the `options()` helper. Test-only. |

This plan lives in `agent/plans/` because the only change is to an agent integration test.

## Purpose / Big Picture

The integration tests in `agent/tests/app/run.rs` start the whole agent. agent#463 pointed that agent's HTTP client at `127.0.0.1:1`, so its requests no longer reach the production control plane, but it left the MQTT worker on its default broker: `mqtt.mirurobotics.com:443` over SSL. The five tests that start an activated agent therefore still open an MQTT connection to the production broker, with client id `placeholder`, on every CI run.

After this change the MQTT worker still starts and shuts down in those tests, but it connects to `127.0.0.1` port 1 over plain TCP, where the CI runner itself refuses the connection. Nothing the agent sends leaves the machine. Observable: once this merges and CI has run on `main` (its push and nightly `schedule` triggers), the production broker sees no new connections with client id `placeholder`.

A second, smaller gain: both test hosts are set in one place, the `options()` helper, because the last test that built its own `AppOptions` literal now uses that helper too.

## Decisions

### Precedent

No new files. The `mqtt_worker` override copies `build_app_options` in `agent/src/main.rs` (`mqtt_worker: mqtt::Options { broker_address, ..Default::default() }` with `use miru_agent::workers::mqtt;`). The refused broker address copies `agent/tests/mqtt/client.rs` and `agent/tests/mqtt/errors.rs`: `ConnectAddress::new(MqttHost::new("127.0.0.1").unwrap(), Protocol::TCP, 1)`. It uses `127.0.0.1`, like the backend host from agent#463, rather than `localhost`, so the result never depends on the runner's name resolution.

### Left out

- Disabling the worker with `enable_mqtt_worker: false`: the author chose to keep it running so its startup and shutdown inside `run` stay covered by these tests.
- A shared "refused address" helper in `agent/tests/test_utils/`: the same `127.0.0.1` port 1 address is spelled out in `agent/tests/mqtt/client.rs`, `agent/tests/mqtt/errors.rs` and here. Folding them together is a generalization this goal doesn't need. Proposed follow-up.
- The Datadog 4xx monitor change (excluding HTTP 499) deferred by agent#463: it lives in Datadog, not this repo.
- A new test asserting that no connection leaves the machine: the change is to the tests' fixture, not to agent behavior.
- Production code: `AppOptions::default()` and `mqtt::Options::default()` must keep the production hosts for real devices.

### Open questions

- Keep `unreachable_backend_host()` or inline it? Once `invalid_app_state_initialization` uses `options()`, the helper has one caller, and adding a matching `unreachable_broker_address()` would mean two one-caller helpers. Default: inline both hosts into `options()`, delete `unreachable_backend_host()`, and say why in the `options()` doc comment. This is fewer lines and leaves one place that sets the hosts.

## Progress

- [x] Milestone 1: set the MQTT broker address in `options()`, route `invalid_app_state_initialization` through `options()`, inline the backend host, and commit.
- [x] Source written
- [ ] Tests written (none new: the six existing tests in `agent/tests/app/run.rs` are the coverage)
- [ ] Validation: `$preflight` reports `CLEAN`

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

(Add entries as work proceeds.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

`agent/src/app/options.rs` defines `AppOptions`, the options for one agent run. Two of its fields name a remote host. `backend_host: BackendHost` (from `agent/src/network/mod.rs`) is the control plane's HTTP host and defaults to `api.mirurobotics.com`. `mqtt_worker: mqtt::Options` is the `Options` struct in `agent/src/workers/mqtt.rs` (not the client `Options` in `agent/src/mqtt/options.rs`); it has two fields, `backoff` and `broker_address: ConnectAddress`. `enable_mqtt_worker` defaults to `true`.

`ConnectAddress` (in `agent/src/mqtt/options.rs`) is a protocol, an `MqttHost` and a port. Its `Default` is `Protocol::SSL`, `MqttHost::default()` (`mqtt.mirurobotics.com`, from `agent/src/network/mod.rs`) and port `443`, which is the production broker. `ConnectAddress::new(broker, protocol, port)` returns an error only when a non-loopback broker uses `Protocol::TCP`, so a loopback broker with `Protocol::TCP` always succeeds.

`agent/src/app/run.rs` holds `run(options, shutdown_signal)`. Its `init` first removes a stale discovery file, then calls `init_app_state`, which builds the HTTP client from `options.backend_host` and loads the device's keys and files from `options.storage.layout`. Only after that does it start workers and, in `init_optional_services`, the socket and TCP servers and the MQTT worker (`init_mqtt_worker`). The MQTT worker's loop (`workers::mqtt::run`) is wrapped in a `tokio::select!` on the shutdown signal, so a refused connection and its retry backoff stop at once on shutdown.

`agent/tests/app/run.rs` holds six tests of `run`. Today two places build `AppOptions`:

- the helper `options(layout, lifecycle, tcp_port)`, used by the five tests that start an activated agent (`activated_layout()` writes fake keys and a `Device::default()` whose id is `placeholder`). It sets `backend_host: unreachable_backend_host()` and takes `mqtt_worker` from `..Default::default()`, so it gets the production broker.
- the test `invalid_app_state_initialization`, which builds its own literal with only `storage` and `backend_host: unreachable_backend_host()` set, and expects `run` to fail. Its layout is empty, so `init_app_state` fails before any server binds or any worker starts. Switching it to `options(&layout, lifecycle(), None)` changes only fields that are never reached: the socket path (`/tmp/miru.sock`, never bound because init fails first, so the test needs no `#[serial]`), `enable_tcp_server` (never started), and `max_shutdown_delay` (a watchdog; shutdown with nothing registered returns at once). The test's outcome and assertion stay the same.

`unreachable_backend_host()` is a private helper in the same file returning `BackendHost::new("127.0.0.1:1").unwrap()`.

CI is `.github/workflows/ci.yml`. Its `test` job runs `./scripts/covgate.sh` (the suite under coverage, then each module's `.covgate` floor); `windows-check` runs the suite on Windows; `lint` runs the import linter, `cargo fmt --check` (default rustfmt, 100-column lines) and clippy.

## Plan of Work

All edits are in `agent/tests/app/run.rs`.

1. Imports, in the `// internal crates` group, kept sorted: add `use miru_agent::mqtt::options::{ConnectAddress, Protocol};` after `use miru_agent::models::Device;`; change `use miru_agent::network::BackendHost;` to `use miru_agent::network::{BackendHost, MqttHost};`; add `use miru_agent::workers::mqtt;` after `use miru_agent::server::{tcp, Options, ServerErr};`. The file already imports `server::Options` as `Options`, so the worker's options are named through the module, `mqtt::Options`, as `agent/src/main.rs` does.

2. `fn options(...)`: before the `AppOptions` literal add `let broker = MqttHost::new("127.0.0.1").unwrap();`. In the literal, replace `backend_host: unreachable_backend_host(),` with

        backend_host: BackendHost::new("127.0.0.1:1").unwrap(),
        mqtt_worker: mqtt::Options {
            broker_address: ConnectAddress::new(broker, Protocol::TCP, 1).unwrap(),
            ..Default::default()
        },

   and extend its doc comment to:

        /// Options for `layout` with the socket under /tmp. `tcp_port` enables the
        /// TCP server on that port. The backend and MQTT broker addresses refuse
        /// connections, so nothing the agent sends leaves this machine.

3. Delete `fn unreachable_backend_host()` and its doc comment from the `HELPERS` section.

4. `invalid_app_state_initialization`: delete the `let options = AppOptions { ... };` literal and call `run_to_exit(options(&layout, lifecycle(), None)).await.unwrap_err();`. Keep the stale discovery file setup and the final assertion unchanged.

No assertion changes and no new tests.

## Concrete Steps

Milestone 1, from the agent repo root (`/home/user/agent` in this workspace):

1. Make the four edits in Plan of Work.

2. Check the hosts are set once and the old helper is gone:

        grep -n "unreachable_backend_host\|AppOptions {\|127.0.0.1" agent/tests/app/run.rs

   Expected: no `unreachable_backend_host`; one `AppOptions {` (inside `options()`); three `127.0.0.1` lines (the `MqttHost`, the `BackendHost`, and the existing health URL in `tcp_requires_bearer_and_cleans_up_discovery_file`).

3. Check no other test starts the agent:

        grep -rln "app::run::run" agent/tests

   Expected: only `agent/tests/app/run.rs`.

4. Commit (one commit for the milestone):

        git add agent/tests/app/run.rs
        git commit -m "test(app): point the app run tests at an unreachable MQTT broker"

Test steps: this change is to a test fixture, so there is no new test to write (the `tests` stage of `$implement` has nothing to add). The test step is to leave all six tests' assertions untouched and confirm in CI that they pass with the MQTT worker on the refused address, as Validation and Acceptance lists.

Do not run `cargo build`, `cargo test`, clippy or fmt locally; CI runs them.

## Validation and Acceptance

Validation runs in CI only. Push the branch and run `$preflight`, which watches `.github/workflows/ci.yml` on the pushed head and fixes failures from the CI job logs.

Acceptance:

- CI job `test` passes on the pushed head, including all six tests in `agent/tests/app/run.rs` (`invalid_app_state_initialization`, `tcp_port_in_use_does_not_abort_startup`, `discovery_write_failure_does_not_abort_startup`, `tcp_requires_bearer_and_cleans_up_discovery_file`, `stale_discovery_file_removed_when_tcp_disabled`, `shutdown_signal_received`) with their assertions unchanged, and every `.covgate` floor still holds.
- CI jobs `lint` and `windows-check` pass on the same head.
- After merge, the production MQTT broker sees no new connections with client id `placeholder` from CI runs.

`$preflight` must report `CLEAN` (CI green on the pushed branch head) before the PR leaves draft or the task is reported complete.

## Idempotence and Recovery

The edits are rerunnable: applying one twice shows up in CI as a duplicate field or import; remove the copy. To roll back, revert the milestone commit; the tests then connect to the production broker again, as before.
