# Consolidate test mock/stub modules into centralized `tests/mocks/`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All changes are in `agent/agent/tests/`. |

This plan lives in `agent/plans/backlog/` because every file touched is under `agent/agent/tests/`.

Working directory for every command below is `agent/` (the `agent` submodule root) unless stated otherwise. The branch `refactor/services-backend-fetcher` is already checked out; do not switch branches.

## Purpose / Big Picture

Pure organizational refactor with no logic changes. Test mocks and stubs are currently scattered across domain-specific test modules (`authn/mock.rs`, `http/mock.rs`, `mqtt/mock.rs`, `sync/mock.rs`, `services/backend_stub.rs`, `test_utils/token_manager.rs`, `mock.rs`). This makes discovery difficult and creates long import paths.

After the refactor:

- All mocks and stubs live under `agent/tests/mocks/` with descriptive filenames.
- Old mock files are deleted and their module declarations removed.
- Every import path across the test codebase is updated to point at `crate::mocks::*`.
- No mock implementation code changes at all -- only file moves and import path rewrites.
- All existing tests pass without modification to test logic.

## Progress

- [ ] M1 Create `mocks/` module and move all mock files
- [ ] M2 Update all import paths across the test codebase
- [ ] M3 Remove old mock module declarations and delete empty files
- [ ] M4 Preflight validation

Add timestamps and split entries as work proceeds.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

All entries below are dated 2026-04-14, authored by the plan author.

- File naming convention: use descriptive names that indicate what is being mocked, not the domain it came from. `http_client.rs` (not `http_mock.rs`), `mqtt_client.rs` (not `mqtt_mock.rs`), `syncer.rs`, `backend.rs`, `token_manager.rs`, `stub_token_manager.rs`, `error.rs`.
- The `http/mock.rs` file contains both the `MockClient` (unit-test mock) and `Server`/route helpers (integration-test server). Both move together into `mocks/http_client.rs` because they are tightly coupled (same `Call` enum, same type universe).
- The `mqtt/mock.rs` file contains both the `MockClient` and broker helpers (`BrokerGuard`, `run_broker`, `run_rejecting_broker`). Both move together into `mocks/mqtt_client.rs` for the same reason.
- `mock.rs` (root-level) contains `MockMiruError` and `SleepController`. These move to `mocks/error.rs` since both are general test utilities related to error/sleep behavior.
- `mocks/mod.rs` re-exports all submodules as `pub mod` so consumers can reach them via `crate::mocks::*`.
- One import site uses `super::mock` (in `mqtt/device.rs`). This changes to `crate::mocks::mqtt_client`.

## Outcomes & Retrospective

(Add entries as work proceeds.)

## Context and Orientation

### Source mock files (current locations -> new locations)

| Current path | New path | Key exports |
|---|---|---|
| `agent/tests/mock.rs` | `agent/tests/mocks/error.rs` | `MockMiruError`, `SleepController` |
| `agent/tests/authn/mock.rs` | `agent/tests/mocks/token_manager.rs` | `MockTokenManager`, `TokenManagerCall` |
| `agent/tests/http/mock.rs` | `agent/tests/mocks/http_client.rs` | `MockClient`, `CapturedRequest`, `Call`, `Server`, `run_server`, route helpers |
| `agent/tests/mqtt/mock.rs` | `agent/tests/mocks/mqtt_client.rs` | `MockClient`, `MockCall`, `BrokerGuard`, `run_broker`, `run_rejecting_broker` |
| `agent/tests/sync/mock.rs` | `agent/tests/mocks/syncer.rs` | `MockSyncer` |
| `agent/tests/services/backend_stub.rs` | `agent/tests/mocks/backend.rs` | `StubBackend`, `PanicBackend` |
| `agent/tests/test_utils/token_manager.rs` | `agent/tests/mocks/stub_token_manager.rs` | `StubTokenManager` |

### Import sites (current -> new)

Every `use` statement below must be rewritten. Grouped by source mock file.

**`crate::mock::*` (root mock.rs -> `crate::mocks::error`)**:
- `agent/tests/workers/poller.rs:5` — `use crate::mock::SleepController` -> `use crate::mocks::error::SleepController`
- `agent/tests/workers/token_refresh.rs:7` — `use crate::mock::SleepController` -> `use crate::mocks::error::SleepController`

**`crate::authn::mock::*` -> `crate::mocks::token_manager`**:
- `agent/tests/workers/token_refresh.rs:6` — `use crate::authn::mock::MockTokenManager` -> `use crate::mocks::token_manager::MockTokenManager`
- `agent/tests/workers/mqtt.rs:2` — `use crate::authn::mock::MockTokenManager` -> `use crate::mocks::token_manager::MockTokenManager`

**`crate::http::mock::*` -> `crate::mocks::http_client`**:
- `agent/tests/sync/deployments.rs:15` — `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`
- `agent/tests/sync/agent_version.rs:2` — `use crate::http::mock::MockClient` -> `use crate::mocks::http_client::MockClient`
- `agent/tests/http/deployments.rs:5` — `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`
- `agent/tests/http/response.rs:2` — `use crate::http::mock` -> `use crate::mocks::http_client as mock` (preserves `mock::` usage in body)
- `agent/tests/http/client.rs:6` — `use crate::http::mock` -> `use crate::mocks::http_client as mock` (preserves `mock::` usage in body)
- `agent/tests/sync/syncer.rs:6` — `use crate::http::mock::{Call, MockClient}` -> `use crate::mocks::http_client::{Call, MockClient}`
- `agent/tests/http/devices.rs:2` — `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`
- `agent/tests/http/config_instances.rs:2` — `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`
- `agent/tests/authn/token_mngr.rs:5` — `use crate::http::mock::MockClient` -> `use crate::mocks::http_client::MockClient`
- `agent/tests/installer/install.rs:2` — `use crate::http::mock::{self, MockClient}` -> `use crate::mocks::http_client::{self as mock, MockClient}` (preserves `mock::Call::*` usage in body)
- `agent/tests/services/backend.rs:5` — `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`
- `agent/tests/server/handlers.rs:71` — `use crate::http::mock::{self, MockClient}` -> `use crate::mocks::http_client::{self as mock, MockClient}` (preserves `mock::Server`, `mock::run_server`, `mock::not_found` usage in body)
- `agent/tests/server/sse.rs:6` — `use crate::http::mock::MockClient` -> `use crate::mocks::http_client::MockClient`

**`crate::mqtt::mock::*` -> `crate::mocks::mqtt_client`**:
- `agent/tests/mqtt/client.rs:6` — `use crate::mqtt::mock` -> `use crate::mocks::mqtt_client as mock` (preserves `mock::run_broker`, `mock::run_rejecting_broker` usage in body)
- `agent/tests/mqtt/device.rs:7` — `use super::mock::{MockCall, MockClient}` -> `use crate::mocks::mqtt_client::{MockCall, MockClient}`
- `agent/tests/workers/mqtt.rs:3` — `use crate::mqtt::mock::MockClient` -> `use crate::mocks::mqtt_client::MockClient`

**`crate::sync::mock::*` -> `crate::mocks::syncer`**:
- `agent/tests/services/device/sync.rs:2` — `use crate::sync::mock::MockSyncer` -> `use crate::mocks::syncer::MockSyncer`
- `agent/tests/workers/poller.rs:6` — `use crate::sync::mock::MockSyncer` -> `use crate::mocks::syncer::MockSyncer`
- `agent/tests/workers/mqtt.rs:4` — `use crate::sync::mock::MockSyncer` -> `use crate::mocks::syncer::MockSyncer`

**`crate::services::backend_stub::*` -> `crate::mocks::backend`**:
- `agent/tests/services/release/get.rs:2` — `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`
- `agent/tests/services/release/current.rs:2` — `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`
- `agent/tests/services/git_commit/get.rs:2` — `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`
- `agent/tests/services/deployment/get.rs:2` — `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`

**`crate::test_utils::token_manager::*` -> `crate::mocks::stub_token_manager`**:
- `agent/tests/services/backend.rs:6` — `use crate::test_utils::token_manager::StubTokenManager` -> `use crate::mocks::stub_token_manager::StubTokenManager`

### Module declarations to update

**`agent/tests/mod.rs`** — Add `pub mod mocks;`, remove `pub mod mock;`. Keep `pub mod test_utils;` (it still contains `testdata`).

**`agent/tests/authn/mod.rs`** — Remove `pub mod mock;`.

**`agent/tests/http/mod.rs`** — Remove `pub mod mock;`.

**`agent/tests/mqtt/mod.rs`** — Remove `pub mod mock;`.

**`agent/tests/sync/mod.rs`** — Remove `pub mod mock;`.

**`agent/tests/services/mod.rs`** — Remove `pub mod backend_stub;`.

**`agent/tests/test_utils/mod.rs`** — Remove `pub mod token_manager;`. Keep `pub mod testdata;`.

## Plan of Work

Three milestones plus a validation step. M1 through M3 can be done in a single commit (pure file moves + import rewrites with no intermediate broken state), but splitting makes review easier. M1+M2+M3 may be combined into one commit if preferred.

### M1 — Create `mocks/` module and move all mock files

Create the directory `agent/tests/mocks/` with:

1. **`agent/tests/mocks/mod.rs`** with contents:

        pub mod backend;
        pub mod error;
        pub mod http_client;
        pub mod mqtt_client;
        pub mod stub_token_manager;
        pub mod syncer;
        pub mod token_manager;

2. Move files (content-identical copies, no edits to file bodies):
   - `agent/tests/mock.rs` -> `agent/tests/mocks/error.rs`
   - `agent/tests/authn/mock.rs` -> `agent/tests/mocks/token_manager.rs`
   - `agent/tests/http/mock.rs` -> `agent/tests/mocks/http_client.rs`
   - `agent/tests/mqtt/mock.rs` -> `agent/tests/mocks/mqtt_client.rs`
   - `agent/tests/sync/mock.rs` -> `agent/tests/mocks/syncer.rs`
   - `agent/tests/services/backend_stub.rs` -> `agent/tests/mocks/backend.rs`
   - `agent/tests/test_utils/token_manager.rs` -> `agent/tests/mocks/stub_token_manager.rs`

3. Add `pub mod mocks;` to `agent/tests/mod.rs` (alphabetically between `models` and `mqtt`).

### M2 — Update all import paths across the test codebase

Rewrite every `use` statement listed in the "Import sites" section above. The exact old -> new pairs are:

**`agent/tests/workers/poller.rs`**:
- `use crate::mock::SleepController` -> `use crate::mocks::error::SleepController`
- `use crate::sync::mock::MockSyncer` -> `use crate::mocks::syncer::MockSyncer`

**`agent/tests/workers/token_refresh.rs`**:
- `use crate::authn::mock::MockTokenManager` -> `use crate::mocks::token_manager::MockTokenManager`
- `use crate::mock::SleepController` -> `use crate::mocks::error::SleepController`

**`agent/tests/workers/mqtt.rs`**:
- `use crate::authn::mock::MockTokenManager` -> `use crate::mocks::token_manager::MockTokenManager`
- `use crate::mqtt::mock::MockClient` -> `use crate::mocks::mqtt_client::MockClient`
- `use crate::sync::mock::MockSyncer` -> `use crate::mocks::syncer::MockSyncer`

**`agent/tests/sync/deployments.rs`**:
- `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`

**`agent/tests/sync/agent_version.rs`**:
- `use crate::http::mock::MockClient` -> `use crate::mocks::http_client::MockClient`

**`agent/tests/sync/syncer.rs`**:
- `use crate::http::mock::{Call, MockClient}` -> `use crate::mocks::http_client::{Call, MockClient}`

**`agent/tests/http/deployments.rs`**:
- `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`

**`agent/tests/http/response.rs`**:
- `use crate::http::mock` -> `use crate::mocks::http_client as mock`

**`agent/tests/http/client.rs`**:
- `use crate::http::mock` -> `use crate::mocks::http_client as mock`

**`agent/tests/http/devices.rs`**:
- `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`

**`agent/tests/http/config_instances.rs`**:
- `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`

**`agent/tests/authn/token_mngr.rs`**:
- `use crate::http::mock::MockClient` -> `use crate::mocks::http_client::MockClient`

**`agent/tests/installer/install.rs`**:
- `use crate::http::mock::{self, MockClient}` -> `use crate::mocks::http_client::{self as mock, MockClient}`

**`agent/tests/services/backend.rs`**:
- `use crate::http::mock::{Call, CapturedRequest, MockClient}` -> `use crate::mocks::http_client::{Call, CapturedRequest, MockClient}`
- `use crate::test_utils::token_manager::StubTokenManager` -> `use crate::mocks::stub_token_manager::StubTokenManager`

**`agent/tests/server/handlers.rs`**:
- `use crate::http::mock::{self, MockClient}` -> `use crate::mocks::http_client::{self as mock, MockClient}`

**`agent/tests/server/sse.rs`**:
- `use crate::http::mock::MockClient` -> `use crate::mocks::http_client::MockClient`

**`agent/tests/mqtt/client.rs`**:
- `use crate::mqtt::mock` -> `use crate::mocks::mqtt_client as mock`

**`agent/tests/mqtt/device.rs`**:
- `use super::mock::{MockCall, MockClient}` -> `use crate::mocks::mqtt_client::{MockCall, MockClient}`

**`agent/tests/services/device/sync.rs`**:
- `use crate::sync::mock::MockSyncer` -> `use crate::mocks::syncer::MockSyncer`

**`agent/tests/services/release/get.rs`**:
- `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`

**`agent/tests/services/release/current.rs`**:
- `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`

**`agent/tests/services/git_commit/get.rs`**:
- `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`

**`agent/tests/services/deployment/get.rs`**:
- `use crate::services::backend_stub::{PanicBackend, StubBackend}` -> `use crate::mocks::backend::{PanicBackend, StubBackend}`

### M3 — Remove old mock module declarations and delete old files

1. **`agent/tests/mod.rs`** — Remove `pub mod mock;` line. (The `pub mod mocks;` was already added in M1.)

2. **`agent/tests/authn/mod.rs`** — Remove `pub mod mock;`. File becomes:

        pub mod token;
        pub mod token_mngr;

3. **`agent/tests/http/mod.rs`** — Remove `pub mod mock;`. File becomes:

        pub mod client;
        pub mod config_instances;
        pub mod deployments;
        pub mod devices;
        pub mod errors;
        pub mod query;
        pub mod request;
        pub mod response;
        pub mod retry;

4. **`agent/tests/mqtt/mod.rs`** — Remove `pub mod mock;`. File becomes:

        pub mod client;
        pub mod device;
        pub mod errors;
        pub mod options;
        pub mod topic;

5. **`agent/tests/sync/mod.rs`** — Remove `pub mod mock;`. File becomes:

        pub mod agent_version;
        pub mod deployments;
        pub mod errors;
        pub mod helpers;
        pub mod syncer;

6. **`agent/tests/services/mod.rs`** — Remove `pub mod backend_stub;`. File becomes:

        pub mod backend;
        pub mod deployment;
        pub mod device;
        pub mod errors;
        pub mod events;
        pub mod git_commit;
        pub mod release;

7. **`agent/tests/test_utils/mod.rs`** — Remove `pub mod token_manager;`. File becomes:

        pub mod testdata;

8. Delete the old files (now unreferenced):
   - `agent/tests/mock.rs`
   - `agent/tests/authn/mock.rs`
   - `agent/tests/http/mock.rs`
   - `agent/tests/mqtt/mock.rs`
   - `agent/tests/sync/mock.rs`
   - `agent/tests/services/backend_stub.rs`
   - `agent/tests/test_utils/token_manager.rs`

### M4 — Preflight validation

Run the full preflight suite and fix any issues.

## Concrete Steps

All commands run from the `agent/` submodule root.

### M1

    mkdir -p agent/tests/mocks
    # Create agent/tests/mocks/mod.rs with the 7 pub mod declarations
    cp agent/tests/mock.rs agent/tests/mocks/error.rs
    cp agent/tests/authn/mock.rs agent/tests/mocks/token_manager.rs
    cp agent/tests/http/mock.rs agent/tests/mocks/http_client.rs
    cp agent/tests/mqtt/mock.rs agent/tests/mocks/mqtt_client.rs
    cp agent/tests/sync/mock.rs agent/tests/mocks/syncer.rs
    cp agent/tests/services/backend_stub.rs agent/tests/mocks/backend.rs
    cp agent/tests/test_utils/token_manager.rs agent/tests/mocks/stub_token_manager.rs
    # Add `pub mod mocks;` to agent/tests/mod.rs (between `models` and `mqtt`)

### M2

    # Rewrite all import statements listed in the M2 section above.
    # Build to verify compilation:
    cargo build --features test --tests

### M3

    # Remove old `pub mod mock;` / `pub mod backend_stub;` / `pub mod token_manager;` declarations
    # Delete old mock files
    rm agent/tests/mock.rs
    rm agent/tests/authn/mock.rs
    rm agent/tests/http/mock.rs
    rm agent/tests/mqtt/mock.rs
    rm agent/tests/sync/mock.rs
    rm agent/tests/services/backend_stub.rs
    rm agent/tests/test_utils/token_manager.rs
    # Build and test:
    cargo build --features test --tests
    cargo test --features test

### M4

    ./scripts/preflight.sh

Fix any fmt/clippy/lint drift until preflight reports `clean`.

    git add -A
    git commit -m "refactor(tests): consolidate mock modules under centralized tests/mocks/"

## Test Steps

These steps verify correctness at each milestone.

### After M1 (files copied, `pub mod mocks;` added)

    cargo build --features test --tests

Expected: compiles successfully. Both old and new module paths exist (old ones still have consumers; new ones are declared but not yet imported).

### After M2 (all imports rewritten)

    cargo build --features test --tests

Expected: compiles successfully. Old mock modules are still declared (so Rust sees them as unused modules, but that does not block compilation).

### After M3 (old declarations and files removed)

    cargo build --features test --tests
    cargo test --features test

Expected: compiles, all tests pass. No references to old paths remain.

Verification grep -- all of these must return zero matches:

    grep -r 'use crate::authn::mock' agent/tests/
    grep -r 'use crate::http::mock' agent/tests/
    grep -r 'use crate::mqtt::mock' agent/tests/
    grep -r 'use crate::sync::mock' agent/tests/
    grep -r 'use crate::services::backend_stub' agent/tests/
    grep -r 'use crate::test_utils::token_manager' agent/tests/
    grep -r 'use crate::mock::' agent/tests/
    grep -r 'use super::mock' agent/tests/

All must return empty (exit code 1).

### After M4 (preflight)

    ./scripts/preflight.sh

Expected: reports `clean`.

## Validation and Acceptance

1. `cargo test --features test` from `agent/` passes all existing tests with no failures.
2. `grep -r 'use crate::authn::mock\|use crate::http::mock\|use crate::mqtt::mock\|use crate::sync::mock\|use crate::services::backend_stub\|use crate::test_utils::token_manager\|use crate::mock::' agent/tests/` returns zero matches.
3. `grep -r 'use super::mock' agent/tests/` returns zero matches.
4. The seven old mock files no longer exist:
   - `agent/tests/mock.rs`
   - `agent/tests/authn/mock.rs`
   - `agent/tests/http/mock.rs`
   - `agent/tests/mqtt/mock.rs`
   - `agent/tests/sync/mock.rs`
   - `agent/tests/services/backend_stub.rs`
   - `agent/tests/test_utils/token_manager.rs`
5. The new directory `agent/tests/mocks/` contains exactly 8 files: `mod.rs`, `error.rs`, `token_manager.rs`, `http_client.rs`, `mqtt_client.rs`, `syncer.rs`, `backend.rs`, `stub_token_manager.rs`.
6. **Preflight (`./scripts/preflight.sh`) must report `clean` before changes are published.** This is a hard gate.

## Out of Scope

- Any change to mock implementation logic. File contents are moved verbatim.
- Any change to production source code (`agent/src/`).
- Any change to `Cargo.toml` or dependencies.
- Any new tests or test logic changes.
- Coverage gate changes (this refactor does not affect coverage).

## Idempotence and Recovery

- All milestones are safe to re-run. The file copies in M1 are idempotent. The import rewrites in M2 are find-and-replace operations. The deletions in M3 are idempotent (deleting already-deleted files is a no-op).
- If interrupted between M1 and M3, both old and new paths exist. Resume from wherever you stopped.
- If the build fails after M2, the most likely cause is a missed import rewrite. Run `grep -r 'use crate::authn::mock\|use crate::http::mock\|use crate::mqtt::mock\|use crate::sync::mock\|use crate::services::backend_stub\|use crate::test_utils::token_manager\|use crate::mock::' agent/tests/` to find stragglers.
- If the build fails after M3 with "unresolved module", a `pub mod mock;` declaration was not removed, or the file was not deleted. Check the module declarations listed in M3.
