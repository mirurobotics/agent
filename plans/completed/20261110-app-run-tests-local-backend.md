# Keep the app run tests off the production control plane

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Point the agent started by `agent/tests/app/run.rs` at a local port nothing listens on. Test-only. |

This plan lives in `agent/plans/` because the only change is to an agent integration test.

## Purpose / Big Picture

The integration tests in `agent/tests/app/run.rs` start the whole agent, and today that agent talks to the production control plane at `api.mirurobotics.com`. Every CI run sends it unauthenticated requests: about 900 HTTP 401s a day on `GET /agent/v1/deployments` and `PATCH /agent/v1/devices/placeholder` since at least 2026-10-26. Since the agent began pulling the latest calibrations at startup (#458, now on `main`), the tests also shut the agent down while that pull is in flight, which production logs as HTTP 499 and which tripped the "[Production] Backend 4xx spike on /agent/v1/calibrations/latest" Datadog monitor.

After this change those tests send their backend requests to `127.0.0.1:1`, where the connection is refused on the CI runner itself. Nothing reaches production. Observable: once this merges and CI has run on `main` (its push and nightly `schedule` triggers), production logs show no new requests for device id `placeholder` on `/agent/v1/deployments`, `/agent/v1/devices/placeholder` or `/agent/v1/calibrations/latest`.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `unreachable_backend_host` (private test helper in `agent/tests/app/run.rs`) | `local_backend_host`, `refused_backend_host` | Says what matters to the test: the host can't be reached. "Local" alone doesn't say nothing answers; "refused" names the mechanism, not the intent. |

The helper's argument to `BackendHost::new` is `"127.0.0.1:1"`, matching the `connection_refused` test in `agent/tests/http/client.rs` (`http::Client::new("http://127.0.0.1:1")`). The newer fixtures in `agent/tests/server/tcp.rs` and `agent/tests/server/sse.rs` use `http://localhost:1`; this plan departs from them on purpose because `127.0.0.1` skips name resolution, so the result never depends on the runner's `/etc/hosts` or IPv6 order.

### Precedent

No new files. The helper sits in the `HELPERS` section of `agent/tests/app/run.rs` next to `lifecycle()`, the file's other small option builder, and follows its shape (a doc comment, then a one-expression body).

### Left out

- The MQTT worker: `AppOptions::default()` keeps `enable_mqtt_worker: true` with the production broker (`MqttHost::default()`), so these tests may still open an MQTT connection to production. Proposed follow-up: point `mqtt_worker` at a loopback broker, or set `enable_mqtt_worker: false`, in the same helpers.
- The Datadog monitor: excluding HTTP 499 (client closed request) from the 4xx monitor is a monitor change in Datadog, not in this repo. Proposed follow-up.
- A shared helper in `agent/tests/test_utils/`: `agent/tests/app/run.rs` is the only test that starts the full agent through `miru_agent::app::run::run`; `agent/tests/app/state.rs` already builds its HTTP client with the non-URL `"doesntmatter"`, which never resolves. One caller doesn't need a shared fixture.
- A new test that asserts no request leaves the machine: the existing tests already exercise startup and shutdown; the change is to their fixture, not to agent behavior.
- Production code: `AppOptions::default()` must keep the production host for real devices.

## Progress

- [x] Milestone 1: set `backend_host` in `agent/tests/app/run.rs` and commit.
- [x] Source written
- [x] Tests written (none new: the six existing tests in `agent/tests/app/run.rs` cover the change and passed in CI)
- [x] Validation: CI green on d7a8313; the final run on this branch's head confirms it.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

(Add entries as work proceeds.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

`agent/src/network/mod.rs` defines `BackendHost`, a validated control plane host with an optional port. `BackendHost::new(raw)` accepts loopback hosts (`localhost`, `127.0.0.1`) and `*.mirurobotics.com`; `BackendHost::as_url()` builds `http://<host>[:port]/agent/v1` for loopback hosts and `https://...` otherwise. `BackendHost::default()` is `api.mirurobotics.com`, the production control plane.

`agent/src/app/options.rs` defines `AppOptions`, the options for one agent run. Its `backend_host` field defaults to `BackendHost::default()`. `agent/src/app/run.rs` builds the agent's one HTTP client from it in `init_app_state` (`http::Client::new(&options.backend_host.as_url())`); the poller, token refresh, device sync and the startup calibrations pull (`init_calibration_pull_task`) all send through that client.

`agent/tests/app/run.rs` holds six tests of `miru_agent::app::run::run`. Two places in it build `AppOptions`, and both end in `..Default::default()`, so both take the production host:

- the helper `options(layout, lifecycle, tcp_port)`, used by the five tests that start an activated agent (`activated_layout()` writes fake keys containing `test` and a `Device::default()` whose id is `placeholder`);
- the test `invalid_app_state_initialization`, which builds `AppOptions` inline and expects `run` to fail during init.

Port 1 on `127.0.0.1` has no listener on CI runners, so a request there fails at once with "connection refused". The startup calibrations pull retries until shutdown, and its task stops on the shutdown signal (`tokio::select!` in `init_calibration_pull_task`), so the refused connections don't slow the tests.

CI is `.github/workflows/ci.yml`. Its `test` job runs `./scripts/covgate.sh` (the suite under coverage, then each module's `.covgate` floor); `windows-check` runs the suite on Windows; `lint` runs the import linter, `cargo fmt --check` and clippy.

## Plan of Work

All edits are in `agent/tests/app/run.rs`.

1. In the `// internal crates` import group, add `use miru_agent::network::BackendHost;` between `use miru_agent::models::Device;` and `use miru_agent::server::{...};` to keep the group sorted.

2. In the `HELPERS` section, directly after `fn lifecycle()`, add:

        /// A backend host nothing listens on, so the agent's requests fail on
        /// this machine instead of reaching the control plane.
        fn unreachable_backend_host() -> BackendHost {
            BackendHost::new("127.0.0.1:1").unwrap()
        }

3. In `fn options(...)`, add `backend_host: unreachable_backend_host(),` to the `AppOptions { ... }` literal, after `server,` and before `..Default::default()`.

4. In `invalid_app_state_initialization`, add `backend_host: unreachable_backend_host(),` to its `AppOptions { ... }` literal, after the `storage` field and before `..Default::default()`. Init fails here before any request, but setting the host keeps a later change to init order from sending this test's requests to production.

No assertion changes, no new tests. The six existing tests are the test coverage for this change: they must still pass with the same assertions, now with the agent pointed at the unreachable host.

## Concrete Steps

Milestone 1, from the agent repo root (`/home/user/agent` in this workspace):

1. Make the four edits in Plan of Work.
2. Check that both `AppOptions` literals set the host:

        grep -n "unreachable_backend_host" agent/tests/app/run.rs

   Expected: three lines, the `fn` definition and the two `backend_host: unreachable_backend_host(),` fields.

3. Check that no other test starts the agent with default options:

        grep -rn "app::run::run\|AppOptions {" agent/tests

   Expected: only `agent/tests/app/run.rs`.

4. Commit (one commit for the milestone):

        git add agent/tests/app/run.rs
        git commit -m "test(app): point the run tests at an unreachable backend host"

Test steps: this change is a test fixture, so there is no new test to write (the `tests` stage of `$implement` has nothing to add). The test step is to leave the six tests' assertions untouched and confirm in CI that they all pass against the unreachable host, as Validation and Acceptance lists.

Do not run `cargo build`, `cargo test`, clippy or fmt locally; CI runs them.

## Validation and Acceptance

Validation runs in CI only. Push the branch and run `$preflight`, which watches `.github/workflows/ci.yml` on the pushed head and fixes failures from the CI job logs.

Acceptance:

- CI job `test` passes on the pushed head, including all six tests in `agent/tests/app/run.rs` (`invalid_app_state_initialization`, `tcp_port_in_use_does_not_abort_startup`, `discovery_write_failure_does_not_abort_startup`, `tcp_requires_bearer_and_cleans_up_discovery_file`, `stale_discovery_file_removed_when_tcp_disabled`, `shutdown_signal_received`) with their assertions unchanged, and every `.covgate` floor still holds.
- CI jobs `lint` and `windows-check` pass on the same head.
- After merge, production logs show no new requests for device `placeholder` from CI runs on `/agent/v1/deployments`, `/agent/v1/devices/placeholder` or `/agent/v1/calibrations/latest`.

`$preflight` must report `CLEAN` (CI green on the pushed branch head) before the PR leaves draft or the task is reported complete.

## Idempotence and Recovery

The edits are additive and rerunnable: if a step is applied twice, the compiler reports a duplicate field or function in CI; remove the copy. To roll back, revert the milestone commit; the tests then talk to production again, as before.
