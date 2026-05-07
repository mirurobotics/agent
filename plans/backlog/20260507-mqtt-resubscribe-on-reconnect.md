# MQTT re-subscribe on reconnect (v0.8.1 hotfix)

## Scope

Single repository: `agent/` only. Target branch is `release/v0.8`; the working branch is `fix/mqtt-resubscribe-on-reconnect`, branched from `release/v0.8`. All edits live under the `agent/` crate (Rust workspace member at `/home/ben/miru/workbench5/repos/agent/agent/`). No other repos are read or written.

## Purpose / Big Picture

A device running the agent loses MQTT command reception after a transient broker-side disconnect, and stays in that broken state until the agent process is restarted. Field log evidence shows v0.7.0 devices stuck like this for 6+ days. The bug exists in v0.6.0, v0.7.0, and current `main`.

Root cause: the agent subscribes to its two MQTT command topics exactly once, inside `init_client`, immediately after the first connect. The underlying `rumqttc` client auto-reconnects on network drops, but it does **not** replay subscribes on the new TCP session. Because the agent uses MQTT's default `clean_session = true`, the broker also discards the device's subscription state on every disconnect. Net effect: after the first reconnect, the agent's TCP session is healthy and the broker is happy, but no command topics are subscribed on either side, so command messages silently disappear.

After this change a user gains:

1. **Sync and ping commands continue to be received after a transient broker disconnect.** The agent re-subscribes on every successful `ConnAck` and also asks the broker to remember its subscriptions (`clean_session = false`). Either fix alone would close the bug; both together are belt-and-suspenders so a future regression in one does not silently re-introduce the problem.
2. **Reconnects are visible in INFO logs.** A connection counter is included in the "Established connection to mqtt broker" log line, so future incidents can be diagnosed from logs alone (e.g. "connection #1" then "connection #2" half a day later, before commands stopped flowing).

User-visible behavior to verify: trigger a sync from the backend, kill the broker connection (e.g. via `tc`/iptables drop, or a broker-side restart), wait for the agent to reconnect, trigger another sync — the second sync command must reach the device.

## Progress

Add entries as work proceeds.

- [ ] Milestone 1: re-subscribe on every ConnAck Success
- [ ] Milestone 2: set `clean_session = false`
- [ ] Milestone 3: connection counter in reconnect log

## Surprises & Discoveries

Add entries as work proceeds.

## Decision Log

Add entries as work proceeds.

## Outcomes & Retrospective

Add entries as work proceeds.

## Context and Orientation

This repo is the Miru device agent: a long-running Rust binary that talks to a backend over HTTPS and to an MQTT broker for command delivery. The Cargo workspace member at `agent/` contains the binary and its modules.

Relevant files (all paths relative to repo root, which is `/home/ben/miru/workbench5/repos/agent`):

- `agent/src/workers/mqtt.rs` — the MQTT worker. Owns the event loop, dispatches incoming events, and handles errors. The bug lives here (no re-subscribe on `ConnAck`).
- `agent/src/mqtt/client.rs` — wrapper around `rumqttc::AsyncClient`. `Client::new` builds the `MqttOptions`. `clean_session` is never set today, so it defaults to `true`.
- `agent/src/mqtt/options.rs` — our `Options` struct. Currently has no `clean_session` field.
- `agent/src/mqtt/device.rs` — helpers `subscribe_sync` and `subscribe_ping`. Used by `init_client` and (after this change) by the ConnAck arm in `handle_event`.
- `agent/src/mqtt/topics.rs` — topic string formatters. `device_sync` is `cmd/devices/{id}/sync` (no version prefix, intentional — see comment in file). `device_ping` is `v1/cmd/devices/{id}/ping`.
- `agent/tests/workers/mqtt.rs` — integration tests for `handle_event`, `handle_error`, `handle_syncer_event`. Pattern for new ConnAck tests is the existing `successful_connack_event` test.
- `agent/tests/mqtt/mock.rs` — `MockClient` records every subscribe call. `num_subscribe_calls_to(topic)` is the assertion helper. `subscribe_fn` is overridable to return errors.
- `agent/tests/mqtt/client.rs` — `Client::new` integration tests against a real `rumqttd` broker via `mock::run_broker`.
- `scripts/test.sh` — canonical test runner: `RUST_LOG=off cargo test --features test --package miru-agent`. Always use this.
- `AGENTS.md` — repo conventions, including the strict 3-block import order (`std`, `crate`, external) used in every source file.

Key code anchors as of this checkout:

- `handle_event` in `agent/src/workers/mqtt.rs` lines 214–259. The `Incoming::ConnAck` arm is lines 225–231.
- `init_client` in the same file lines 158–189. The pattern to mirror for re-subscribing is at lines 181 and 184.
- `State` struct lines 298–302; constructed at lines 107–111 of `run_impl`.
- `handle_error` lines 304–358. Only re-creates the client on auth errors. Network errors are absorbed (rumqttc auto-reconnects internally) and the bug surfaces precisely because that internal auto-reconnect bypasses our subscribe code.
- `Client::new` in `agent/src/mqtt/client.rs` lines 44–73. Build site for `MqttOptions`.
- `Options` in `agent/src/mqtt/options.rs` lines 61–112. Builder-style.

Background a beginner needs:

- **MQTT subscribe is a session-state operation.** In MQTT 3.1.1, `clean_session = true` (rumqttc's default) tells the broker to discard everything when the client disconnects, including the subscriptions the client made. `clean_session = false` plus a stable client ID tells the broker to remember subscriptions across disconnects, so messages can be queued for the device while it's offline and delivered on reconnect.
- **rumqttc's auto-reconnect.** `rumqttc::AsyncClient` will internally re-establish the TCP connection if the broker drops it, and you observe each reconnect as a fresh `Event::Incoming(Incoming::ConnAck(..))` from `eventloop.poll()`. It does **not** re-issue subscribe packets for you. You must do that yourself in response to the `ConnAck`.
- **`session_present`.** The `ConnAck` packet carries a `session_present: bool` flag. With `clean_session = false`, on the first connect it is `false`; on a subsequent reconnect where the broker still has session state, it is `true`. We do not branch on this flag — re-subscribing unconditionally on every `ConnAck::Success` is correct and idempotent (the broker will simply replace the existing subscription with itself).

## Plan of Work

Three milestones. Each ends in one signed git commit. The commits should be reviewable in isolation.

**Milestone 1 — defensive re-subscribe.** In `handle_event`, after the existing log line and storage patch in the `ConnAck::Success` path, call `mqtt::device::subscribe_sync` and `mqtt::device::subscribe_ping` exactly the way `init_client` does. Same error strings. Subscribe failures are logged but do not change the returned `err_streak` (no need to drive the cooldown; the next reconnect will retry). This alone fixes the bug per the root-cause analysis in Purpose, even on brokers that do not honor `clean_session = false`.

**Milestone 2 — durable broker session.** In `mqtt::Options`, add a `clean_session: bool` field defaulting to `false`. In `Client::new`, propagate it to `MqttOptions::set_clean_session`. With a stable, non-empty `client_id` (which we always have — it is the device id) this asks the broker to keep our subscriptions and queued messages across disconnects, providing a second line of defense and reducing message loss during the gap.

  rumqttc's `set_clean_session` panics if the client_id is empty *and* clean_session is false. In production `client_id` is always the device id (non-empty); in `Options::default()` the fallback `client_id` is `"miru-agent"` (also non-empty). No call site is at risk.

**Milestone 3 — observable reconnects.** Add a `connect_count: u32` field to `State`, initialized to `0`. Change `handle_event`'s signature to accept `connect_count: &mut u32`, and in the `ConnAck::Success` path increment it via `saturating_add(1)` before logging. Update the log line to `Established connection to mqtt broker (connection #{n})`. Update `run_impl` to pass `&mut state.connect_count`. Mechanically update existing handle_event tests to thread an `&mut counter` argument.

  Counting on every Success ConnAck (not just reconnects) is intentional — connection #1 is the initial connect; counts ≥2 are reconnects. That mapping makes log forensics obvious.

Order rationale: M3 last because its `handle_event` signature change forces edits to every existing test in `agent/tests/workers/mqtt.rs`; doing it last keeps M1 and M2 diffs reviewable.

### Test strategy per milestone

- **Milestone 1 tests** — three new tests in the `handle_connection_events` mod inside `agent/tests/workers/mqtt.rs`:
  - `connack_success_resubscribes_to_sync_and_ping`: send a `ConnAck::Success` through `handle_event` with default `MockClient`. Assert `mock.num_subscribe_calls_to("cmd/devices/device_id/sync") == 1` and `mock.num_subscribe_calls_to("v1/cmd/devices/device_id/ping") == 1`.
  - `connack_non_success_does_not_subscribe`: send a `ConnAck` with `code = ConnectReturnCode::BadUserNamePassword`. Assert no subscribe calls were made on the mock.
  - `connack_subscribe_error_does_not_change_err_streak`: configure `subscribe_fn` to return `MQTTError::MockErr { is_authentication_error: false, is_network_conn_err: false }`. Assert `handle_event` returns `0` (the same `err_streak` it would have returned without the failure) and does not panic.

- **Milestone 2 tests** — one new unit test in `agent/tests/mqtt/options.rs` (or, equivalently, `client.rs`):
  - `default_options_have_persistent_session`: build `Options::new(Credentials::default())` and `Options::default()`. Assert `options.clean_session == false`.

  This is a flag-level assertion rather than a broker-level behavioral assertion. The broker-side check (start a `rumqttd` instance, connect with `clean_session = false`, disconnect, reconnect, assert `session_present == true`) is possible via the existing `mock::run_broker` harness in `agent/tests/mqtt/mock.rs`, but: (a) `rumqttd` 0.19's session persistence behavior is not contractually guaranteed across versions; (b) the hotfix's correctness rests on a single-line `set_clean_session(false)` call in `Client::new`, which is trivially audited by reading the diff. A flag-level test catches the only realistic regression (someone deletes the line). End-to-end reconnect testing is explicitly listed as a non-goal below.

- **Milestone 3 tests** — modify the existing `successful_connack_event` test to also assert that the counter has been incremented (`assert_eq!(connect_count, 1)` after one Success ConnAck; build a second event and verify the counter advances to `2`). All other existing `handle_event` tests need only the mechanical signature update.

### Non-negotiable: import order

Preserve the 3-block import order described in `AGENTS.md` when adding new tests. See `agent/tests/workers/mqtt.rs` lines 1–21 for a canonical example.

## Concrete Steps

All commands assume the working directory is the repo root: `/home/ben/miru/workbench5/repos/agent`. Confirm with `pwd` before starting. The branch `fix/mqtt-resubscribe-on-reconnect` should already be checked out; verify with `git branch --show-current` before each commit.

### Setup

    cd /home/ben/miru/workbench5/repos/agent
    git branch --show-current   # expect: fix/mqtt-resubscribe-on-reconnect
    git status                  # expect: clean working tree
    ./scripts/test.sh           # baseline: must pass before starting

### Milestone 1 — re-subscribe on every successful ConnAck

Edit `agent/src/workers/mqtt.rs`. In `handle_event`, replace the body of the `Event::Incoming(Incoming::ConnAck(connack))` arm so it reads:

    Event::Incoming(Incoming::ConnAck(connack)) => {
        if connack.code != ConnectReturnCode::Success {
            return err_streak;
        }
        info!("Established connection to mqtt broker");
        let _ = device_stor.patch(device::Updates::connected()).await;

        // Re-subscribe on every successful (re)connect. rumqttc auto-reconnects
        // internally without replaying subscribes, and the broker may have
        // discarded session state, so we must restate our subscriptions here.
        if let Err(e) = mqtt::device::subscribe_sync(mqtt_client, device_id).await {
            error!("error subscribing to device synchronization updates: {e:?}");
        };
        if let Err(e) = mqtt::device::subscribe_ping(mqtt_client, device_id).await {
            error!("error subscribing to device ping updates: {e:?}");
        };
    }

(The error strings must match `init_client` verbatim — see `agent/src/workers/mqtt.rs` lines 181–186 for the source of truth.)

Edit `agent/tests/workers/mqtt.rs`. Inside `pub mod handle_connection_events`, add three tests after `successful_connack_event`:

    #[tokio::test]
    async fn connack_success_resubscribes_to_sync_and_ping() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::ConnAck(ConnAck {
            code: ConnectReturnCode::Success,
            session_present: false,
        }));
        let mqtt_client = MockClient::default();
        let syncer = MockSyncer::default();
        let _ = handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file).await;

        assert_eq!(
            mqtt_client.num_subscribe_calls_to(&topics::device_sync("device_id")),
            1
        );
        assert_eq!(
            mqtt_client.num_subscribe_calls_to(&topics::device_ping("device_id")),
            1
        );
    }

    #[tokio::test]
    async fn connack_non_success_does_not_subscribe() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::ConnAck(ConnAck {
            code: ConnectReturnCode::BadUserNamePassword,
            session_present: false,
        }));
        let mqtt_client = MockClient::default();
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file).await;
        assert_eq!(err_streak, 0);

        assert_eq!(
            mqtt_client.num_subscribe_calls_to(&topics::device_sync("device_id")),
            0
        );
        assert_eq!(
            mqtt_client.num_subscribe_calls_to(&topics::device_ping("device_id")),
            0
        );
    }

    #[tokio::test]
    async fn connack_subscribe_error_does_not_change_err_streak() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::ConnAck(ConnAck {
            code: ConnectReturnCode::Success,
            session_present: false,
        }));
        let mut mqtt_client = MockClient::default();
        mqtt_client.subscribe_fn = Box::new(|| {
            Err(Box::new(MQTTError::MockErr(MockErr {
                is_authentication_error: false,
                is_network_conn_err: false,
            })))
        });
        let syncer = MockSyncer::default();
        let err_streak =
            handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file).await;
        assert_eq!(err_streak, 0);
    }

Run the tests:

    ./scripts/test.sh

All three new tests must pass. The two pre-existing `connack` tests must continue to pass.

Verify clean_session is not yet set (it's set in milestone 2):

    grep -n "set_clean_session\|clean_session" agent/src/mqtt/client.rs agent/src/mqtt/options.rs   # expect: no matches

Commit:

    git add -A
    git status                                       # review the diff list
    git diff --cached --stat                          # sanity check
    git commit -S -m "fix(mqtt): re-subscribe on every successful ConnAck"

### Milestone 2 — clean_session = false

Edit `agent/src/mqtt/options.rs`. Add a `clean_session: bool` field on `Options`:

    #[derive(Debug, Clone)]
    pub struct Options {
        pub connect_address: ConnectAddress,
        pub credentials: Credentials,
        pub client_id: String,
        pub keep_alive: Duration,
        pub timeouts: Timeouts,
        pub capacity: usize,
        pub clean_session: bool,
    }

In the `impl Options { pub fn new(...) }` constructor, set `clean_session: false`. (Do not add a `with_clean_session` builder unless needed by a test — keep the diff minimal.)

Edit `agent/src/mqtt/client.rs`. In `Client::new`, after `set_credentials` and before the `match options.connect_address.protocol` block, add:

    mqtt_options.set_clean_session(options.clean_session);

Add one new unit test in `agent/tests/mqtt/options.rs` (the file already exists). Place the new test inside `mod opts` (alongside `new_defaults` / `default`), so it inherits the existing `use super::*;` imports. If `Credentials` and `Options` aren't already imported in that mod's scope, add them in the proper import block:

    #[test]
    fn default_options_have_persistent_session() {
        let options = Options::new(Credentials {
            username: "u".to_string(),
            password: "p".to_string(),
        });
        assert!(!options.clean_session, "clean_session must default to false so the broker preserves subscriptions across reconnects");

        let default_options = Options::default();
        assert!(!default_options.clean_session);
    }

Run the tests:

    ./scripts/test.sh

The new test must pass. All previously passing tests must still pass — in particular `test_mqtt_client` (which connects to a real `rumqttd`) must still succeed; with a non-empty `client_id` (`test_user`), `set_clean_session(false)` is fine.

Commit:

    git add -A
    git diff --cached --stat
    git commit -S -m "fix(mqtt): set clean_session=false to preserve subscriptions across reconnects"

### Milestone 3 — observable reconnects

Edit `agent/src/workers/mqtt.rs`:

1. Add a field to `State`:

    pub struct State {
        pub client: mqtt::Client,
        pub eventloop: EventLoop,
        pub err_streak: ErrStreak,
        pub connect_count: u32,
    }

2. Initialize it in `run_impl` where `State` is constructed (around line 107):

    let mut state = State {
        client: mqtt_client,
        eventloop,
        err_streak: 0,
        connect_count: 0,
    };

3. Change `handle_event`'s signature to take a counter:

    pub async fn handle_event<ClientT: ClientI, SyncerT: SyncerExt>(
        event: &Event,
        mqtt_client: &ClientT,
        syncer: &SyncerT,
        device_id: &str,
        device_stor: &storage::Device,
        connect_count: &mut u32,
    ) -> ErrStreak {

4. Update the `ConnAck::Success` path to increment and log:

    if connack.code != ConnectReturnCode::Success {
        return err_streak;
    }
    *connect_count = connect_count.saturating_add(1);
    info!("Established connection to mqtt broker (connection #{count})", count = *connect_count);
    let _ = device_stor.patch(device::Updates::connected()).await;
    // ... existing re-subscribe block unchanged ...

5. Update the `run_impl` call site to pass `&mut state.connect_count`:

    state.err_streak = handle_event(
        &mqtt_event,
        &state.client,
        syncer,
        &device.id,
        device_stor,
        &mut state.connect_count,
    ).await;

Edit `agent/tests/workers/mqtt.rs`. Every existing call site of `handle_event` needs an extra `&mut counter` argument. The mechanical pattern: declare `let mut connect_count: u32 = 0;` before the call, then pass `&mut connect_count`. Touch every test in `handle_connection_events`, `handle_sync_events`, `handle_ping_events`, and the three new milestone-1 tests.

Strengthen `successful_connack_event` (or add a new test `connack_success_increments_connect_count`):

    #[tokio::test]
    async fn connack_success_increments_connect_count() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let event = Event::Incoming(Incoming::ConnAck(ConnAck {
            code: ConnectReturnCode::Success,
            session_present: false,
        }));
        let mqtt_client = MockClient::default();
        let syncer = MockSyncer::default();
        let mut connect_count: u32 = 0;

        let _ = handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file, &mut connect_count).await;
        assert_eq!(connect_count, 1);

        let _ = handle_event(&event, &mqtt_client, &syncer, "device_id", &device_file, &mut connect_count).await;
        assert_eq!(connect_count, 2);
    }

Run the tests:

    ./scripts/test.sh

All tests must pass.

Manual sanity check (optional but recommended): start the binary against a local broker and observe the log line at INFO level:

    Established connection to mqtt broker (connection #1)

Commit:

    git add -A
    git diff --cached --stat
    git commit -S -m "feat(mqtt): log mqtt reconnects with a connection counter"

### Final preflight

Run preflight (the `preflight` skill) and confirm it reports `clean` before pushing or opening a PR. Do not push if preflight is not clean — fix findings first.

## Validation and Acceptance

**Acceptance is behavioral, not just "tests pass":**

1. After milestone 1, the `Incoming::ConnAck(Success)` arm of `handle_event` invokes `subscribe_sync` and `subscribe_ping` exactly once per call — observable as `MockClient::num_subscribe_calls_to("cmd/devices/device_id/sync") == 1` and `... ("v1/cmd/devices/device_id/ping") == 1` after a single Success ConnAck (test `connack_success_resubscribes_to_sync_and_ping`). Non-Success ConnAcks produce zero subscribe calls (test `connack_non_success_does_not_subscribe`). A subscribe failure on the ConnAck arm does not change `err_streak` (test `connack_subscribe_error_does_not_change_err_streak`). Code audit: `init_client`'s startup subscribes at `agent/src/workers/mqtt.rs:181,184` remain in place, so a running device performs two subscribes per topic on the second connect (one at startup via `init_client`, one per ConnAck via the new arm).
2. After milestone 2, both `Options::new(...)` and `Options::default()` produce a struct with `clean_session == false`, and `Client::new` propagates that to `MqttOptions::set_clean_session(false)` (audit by reading the diff in `agent/src/mqtt/client.rs`).
3. After milestone 3, on two successive `ConnAck::Success` events through `handle_event`, the threaded `connect_count` advances `0 → 1 → 2` and the INFO log line reads `Established connection to mqtt broker (connection #1)` then `... (connection #2)`.
4. `./scripts/test.sh` exits zero with all new tests included.
5. Preflight reports `clean`.
6. Manual reconnect smoke test (optional but strongly recommended before tagging v0.8.1): run the agent against a local `rumqttd` (or staging broker), confirm "connection #1" log, kill the broker connection, confirm a "connection #2" log appears with no agent restart, then trigger a sync command from the backend and confirm the agent processes it.

The full integration check — disconnect-and-reconnect over a real broker — is **not** automated as part of this PR. The harness exists (`mock::run_broker`) and a follow-up plan can add it; doing so here would inflate the diff beyond the hotfix's scope.

## Idempotence and Recovery

Each milestone is a single commit and each test run is independent. Re-running `./scripts/test.sh` is safe and idempotent.

If a milestone's tests fail after the edit, the recovery path is:

    git diff                  # inspect uncommitted changes
    git restore <file>        # discard changes to one file
    # or, after a bad commit:
    git reset --soft HEAD~1   # uncommit but keep changes staged

The signature change in milestone 3 is the only edit that breaks compilation across many files — fix all `handle_event` call sites in `agent/tests/workers/mqtt.rs` together (Cargo's compile error list will name every site). There is no risky migration here: no on-disk state changes, no API contract changes, no schema changes. The only behavioral change visible to the broker is `clean_session = false`, which is renegotiated automatically on the next connect.

If a hotfix needs to be reverted in the field, `git revert` of any of the three commits independently is safe; reverting milestone 1 while keeping milestone 2 leaves the original bug only partially mitigated (broker-side session might survive but the agent still doesn't replay subscribes if the broker forgets), so revert in reverse order if reverting more than one.

## Non-goals

Stated explicitly so the implementer does not gold-plate:

- **Not** bumping the Cargo workspace version. The release engineer owns versioning and the v0.8.1 tag.
- **Not** shortening the 12 h poller interval in `agent/src/workers/poller.rs`. That is a separate config decision tracked elsewhere.
- **Not** refactoring `init_client` to remove its now-redundant subscribe calls. They become duplicates of the ConnAck-arm subscribes after milestone 1, but removing them changes the no-broker startup path's logging and is out of hotfix scope.
- **Not** adding rumqttd-based end-to-end reconnect tests. The harness exists in `agent/tests/mqtt/mock.rs` (`run_broker`); use it in a follow-up if desired.
- **Not** cherry-picking to `main`. That is a separate PR and is out of this plan's scope.
- **Not** branching on `ConnAck.session_present`. Re-subscribing unconditionally on every Success ConnAck is correct and idempotent, and avoids a class of bugs where the broker reports `session_present = true` but has actually forgotten the subscriptions.
