# Wait for Activation Loop (replace exit-if-not-activated)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All edits live in this repo (`miru-agent` crate at `agent/agent/`). |

All changes are confined to:

- `agent/agent/src/main.rs` (replace one early-return block, add one call site).
- `agent/agent/src/app/wait_for_activation.rs` (new file — pure helper + `should_log` + `WaitOutcome`).
- `agent/agent/src/app/mod.rs` (`pub mod wait_for_activation;` registration).
- `agent/agent/tests/app/wait_for_activation.rs` (new integration test file).
- `agent/agent/tests/app/mod.rs` (`pub mod wait_for_activation;` registration).

DO NOT modify `agent/src/storage/device.rs`, `agent/src/storage/mod.rs`, `build/debian/miru.service`, the `provision` subcommand, or `assert_activated`.

## Purpose / Big Picture

The agent currently early-returns from `run_agent()` when the device is not yet activated:

```rust
// agent/src/main.rs:160-164
if let Err(e) = storage::assert_activated(&layout).await {
    error!("Device is not yet activated: {}", e);
    return;
}
```

Under systemd this causes a fast restart loop until the operator runs `miru provision`. The new behavior: the agent stays running, polls `assert_activated` every second, and proceeds to the rest of `run_agent()` as soon as the auth keys appear on disk. SIGTERM / SIGINT / Ctrl-C during the wait shuts the agent down cleanly. Logs are throttled with exponential backoff so the journal isn't spammed during long waits.

Public observable behavior:

1. Service starts on a fresh device with no `auth/` keys → first log `"Device is not yet activated; waiting for provisioning..."`, then quiet, with periodic `"Still waiting for activation (waited Ns)..."` heartbeats at 2s, 4s, 8s, 16s, 32s, 64s, 128s, 256s, 512s, 1024s, then every 1024s thereafter.
2. Operator runs `miru provision` in a side terminal → the agent prints `"Device activated; starting agent."` within 1 second of the second key appearing, and falls through to the existing `upgrade::reconcile` / settings-load / server-startup code.
3. Operator runs `systemctl stop miru` (or sends SIGINT/SIGTERM) during the wait → agent logs `"Shutdown received while waiting for activation"` and exits 0.

## Progress

- [x] M1: Create `agent/src/app/wait_for_activation.rs` with `WaitOutcome`, `should_log`, and `wait_for_activation` (sleep-fn injected, shutdown-future raced via `tokio::select!`). Register the module in `agent/src/app/mod.rs`. `cargo build` clean. _Done 2026-05-08; 6 unit tests pass._
- [x] M2: Wire `run_agent()` to call `wait_for_activation` instead of early-returning. Pass `await_shutdown_signal()` as the shutdown future and `tokio::time::sleep` as the sleep fn (mirrors the `upgrade::reconcile` call site three lines below). _Done 2026-05-08; build clean. `error!` import retained — still used by 6 other call sites in `main.rs`._
- [ ] M3: Add unit tests for `should_log` covering 0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2047, 2048, 3071, 3072, 4096, plus a sweep of off-cycle values.
- [ ] M4: Add integration tests in `agent/tests/app/wait_for_activation.rs`: activates immediately, activates after N cycles, shutdown during wait, shutdown wins when keys appear simultaneously.
- [ ] M5: Final preflight (`scripts/test.sh`, `scripts/lint.sh`, `scripts/covgate.sh`) clean.

Use timestamps when you complete steps.

## Surprises & Discoveries

(Empty — will be populated during implementation.)

## Decision Log

- Decision: Place `wait_for_activation` in `agent/src/app/` as a sibling to `app::upgrade`, not inside `main.rs`.
  Rationale: `app::upgrade` is the canonical pattern in this crate for "loop-with-injectable-sleep" logic, and integration tests (`agent/tests/app/upgrade.rs`) consume it through the `miru_agent::app::*` re-export. Placing the new helper alongside makes it testable from `agent/tests/app/wait_for_activation.rs` using the same conventions (`Layout::new(temp_dir)`, `WriteOptions::OVERWRITE_ATOMIC` for key files, no real sleep). `main.rs` cannot host `pub` items reachable from integration tests.
  Date/Author: 2026-05-08 / plan author.

- Decision: Mirror `upgrade::reconcile`'s sleep-injection signature exactly: `F: Fn(Duration) -> Fut, Fut: Future<Output = ()> + Send`.
  Rationale: Identical bound means the call site in `main.rs` is `tokio::time::sleep` (a function pointer that already satisfies that bound — `agent/src/main.rs:180` already does this). Tests pass `|_| async {}` for instant polling, exactly like `agent/tests/app/upgrade.rs`'s `async fn no_sleep(_: StdDuration) {}`.
  Date/Author: 2026-05-08 / plan author.

- Decision: Pass the shutdown future as `impl Future<Output = ()> + Send` (consumed by value), not as a reference.
  Rationale: This matches `app::run::run`'s signature (`agent/src/app/run.rs`: `shutdown_signal: impl Future<Output = ()> + Send + 'static`). The future is consumed exactly once via `tokio::select!`. Tests construct shutdown futures from `tokio::sync::oneshot::channel()`'s receiver mapped through `.map(|_| ())`, or — even simpler — `async move { rx.await.ok(); }`.
  Date/Author: 2026-05-08 / plan author.

- Decision: Use `tokio::pin!` on the shutdown future inside `wait_for_activation`, then `tokio::select!` between `&mut shutdown` and the sleep+check arm.
  Rationale: `tokio::select!` polls each branch once per iteration, but a non-`Unpin` future (the input `impl Future`) needs pinning to be polled multiple times across loop iterations without being moved. The standard pattern in this crate is `tokio::pin!(shutdown);` then `&mut shutdown` inside `select!`.
  Date/Author: 2026-05-08 / plan author.

- Decision: Use a flat 1-second poll interval, NOT exponential. Apply exponential backoff ONLY to log throttling.
  Rationale: Per task constraints — keys are local file existence checks (~µs), so polling every 1s is cheap. The cost we want to manage is journal noise during long waits, which is what `should_log` handles.
  Date/Author: 2026-05-08 / plan author.

- Decision: `should_log(cycle: u64) -> bool` is a `pub` fn inside `wait_for_activation.rs` (not buried in the main poll loop) so integration tests can `use` it.
  Rationale: The task spec says "DO NOT use tracing-subscriber capture" — pure-function unit testing is the only way to validate the throttle schedule without a log capture. Hoisting it to a module-level fn lets the integration test file `use miru_agent::app::wait_for_activation::should_log;` and assert against a wide cycle sweep.
  Date/Author: 2026-05-08 / plan author.

- Decision: After cycle 1024, log every 1024s thereafter (heartbeat). I.e. `should_log` returns `true` for every power-of-two in `[2, 1024]`, plus every multiple of 1024 above 1024. Cycles 0 and 1 are silent (cycle 0 is logged by the caller before the loop; cycle 1 is intentionally silent even though `1.is_power_of_two()` is true).
  Rationale: Task spec: "powers of 2, then every 1024s thereafter as a heartbeat". Implementation: `cycle >= 2 && ((cycle <= 1024 && cycle.is_power_of_two()) || (cycle > 1024 && cycle % 1024 == 0))`. Test cycle 1 → `false` to lock this in.
  Date/Author: 2026-05-08 / plan author.

- Decision: Define `WaitOutcome` as a public, plain enum with two variants: `Activated` and `ShutdownRequested`. No data payload.
  Rationale: The caller (`run_agent`) only needs to branch on which path was taken. Keeping the enum payload-free makes the caller code a 2-arm `match` and the tests trivial pattern-match assertions. Derive `Debug, Clone, Copy, PartialEq, Eq`.
  Date/Author: 2026-05-08 / plan author.

- Decision: Test "activates after N cycles" coordinates by making the injected sleep_fn create the key files on its Nth invocation.
  Rationale: Per `agent/tests/app/upgrade.rs` precedent, no real sleep in tests. The injected sleep_fn is the test's hook into the polling loop — incrementing an `AtomicUsize` and writing the keys on the Nth call lets us deterministically verify the exact number of polls before activation succeeds.
  Date/Author: 2026-05-08 / plan author.

- Decision: Do NOT add `serial_test::serial` annotations to the new tests.
  Rationale: Each test creates its own temp dir via `filesys::Dir::create_temp_dir("...")` and constructs `Layout::new(dir)` against that temp dir — no shared OS resources (no `/tmp/miru.sock`, no fixed paths). Per `AGENTS.md`, `#[serial]` is reserved for tests that bind shared global state.
  Date/Author: 2026-05-08 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

**Crate**: `miru-agent` at `agent/agent/` (manifest `agent/agent/Cargo.toml`). `[features] test = []` exists; `scripts/test.sh` invokes `cargo test --features test --package miru-agent`.

**Current code under change — `agent/src/main.rs:146-231`**:

```rust
async fn run_agent() {
    let layout = storage::Layout::default();

    let log_guard = match logs::init(logs::Options::default()) { /* ... */ };

    // check the agent has been activated  <-- LINES 160-164: the block to replace
    if let Err(e) = storage::assert_activated(&layout).await {
        error!("Device is not yet activated: {}", e);
        return;
    }

    // reconcile, settings-read, log-level apply, server-startup ...
}
```

**Helpers already in `main.rs`**:

- `get_bootstrap_base_url() -> BackendUrl` (line 233) — used after activation; not relevant to the wait.
- `await_shutdown_signal()` (line 242) — `async fn` that resolves on SIGTERM / SIGINT / Ctrl-C. Already wired into the post-activation path via `run(options, await_shutdown_signal()).await`. We will pass a fresh `await_shutdown_signal()` future to `wait_for_activation`.

**Sleep-injection pattern to mirror — `agent/src/app/upgrade.rs`**:

```rust
pub async fn reconcile<F, Fut, HTTPClientT: ClientI>(
    layout: &Layout,
    http_client: &HTTPClientT,
    version: &str,
    sleep_fn: F,
) -> Result<Outcome, UpgradeErr>
where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{ /* loop { ... sleep_fn(Duration::from_secs(...)).await; ... } */ }
```

The `main.rs` call site (line 176-182) passes `tokio::time::sleep` directly:

```rust
upgrade::reconcile(&layout, &bootstrap_http_client, version::VERSION, tokio::time::sleep).await
```

The new `wait_for_activation` call must follow the same shape so `tokio::time::sleep` is accepted as the function pointer.

**`storage::assert_activated` — `agent/src/storage/device.rs:14-30`** (DO NOT modify): Returns `Ok(())` iff both `auth/private_key` and `auth/public_key` exist; otherwise `Err(StorageErr::DeviceNotActivatedErr(_))`. Pure file-existence check, ~µs cost.

**`Layout` test pattern — see `agent/tests/storage/device.rs`**:

```rust
async fn fresh_layout() -> (Layout, filesys::Dir) {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir.clone());
    layout.auth().root.create_if_absent().await.unwrap();
    (layout, dir)
}
```

Hold the `filesys::Dir` for the test's lifetime so the temp dir isn't reaped.

To create a key file:

```rust
layout.auth().private_key()
    .write_string("private", filesys::WriteOptions::OVERWRITE_ATOMIC)
    .await.unwrap();
layout.auth().public_key()
    .write_string("public",  filesys::WriteOptions::OVERWRITE_ATOMIC)
    .await.unwrap();
```

**Testing conventions — `AGENTS.md`**:

- All test invocations go through `./scripts/test.sh` (which sets `--features test`).
- Test files live at `agent/agent/tests/<module>/...` mirroring `agent/agent/src/<module>/...`.
- Each `agent/agent/tests/<module>/mod.rs` lists `pub mod <name>;` for every test sub-file.
- `#[serial]` only for tests that touch shared OS state.

**Coverage gate**: `agent/agent/src/app/.covgate` declares the floor. Adding well-tested code should keep coverage above this; if the new module's coverage actually computed by `scripts/covgate.sh` would push the directory under, document the new actual percentage in Outcomes & Retrospective and bump the gate in a separate commit only if necessary.

## Plan of Work

### M1 — Create `agent/src/app/wait_for_activation.rs`

New file. Required imports per `AGENTS.md` import-ordering convention:

```rust
// standard crates
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::storage::{self, Layout};

// external crates
use tokio::pin;
use tracing::info;
```

Public types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Activated,
    ShutdownRequested,
}
```

Public function:

```rust
/// Block until the device is activated (auth keys exist on disk) or a shutdown
/// signal is received. Polls `storage::assert_activated` once per cycle using
/// the injected `sleep_fn` (1-second cycles in production), throttling logs
/// via `should_log`. The shutdown future is consumed once and races every
/// poll cycle via `tokio::select!`.
pub async fn wait_for_activation<F, Fut, S>(
    layout: &Layout,
    sleep_fn: F,
    shutdown: S,
) -> WaitOutcome
where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
    S: Future<Output = ()> + Send,
{
    // Fast path: already activated.
    if storage::assert_activated(layout).await.is_ok() {
        info!("Device activated; starting agent.");
        return WaitOutcome::Activated;
    }

    info!("Device is not yet activated; waiting for provisioning...");

    pin!(shutdown);
    let mut cycle: u64 = 1;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                info!("Shutdown received while waiting for activation");
                return WaitOutcome::ShutdownRequested;
            }
            _ = sleep_fn(Duration::from_secs(1)) => {
                if storage::assert_activated(layout).await.is_ok() {
                    info!("Device activated; starting agent.");
                    return WaitOutcome::Activated;
                }
                if should_log(cycle) {
                    info!("Still waiting for activation (waited {cycle}s)...");
                }
                cycle = cycle.saturating_add(1);
            }
        }
    }
}
```

Notes on the shape above:

- The first `info!` (cycle 0) fires unconditionally before the loop, satisfying "Log at cycle 0 (first miss)".
- The loop counter starts at 1 (we've already done one check + one log). `should_log(cycle)` is what gates subsequent logs at cycles 2, 4, 8, ... 1024, 2048, 3072, ....
- `biased;` makes shutdown win on a tie (keys appear and shutdown fires in the same poll iteration), which is the safer behavior for a service shutdown.
- `pin!(shutdown)` is required because `S: Future` has no `Unpin` bound; without pinning, polling `&mut shutdown` across loop iterations would not compile.
- `saturating_add` on the cycle counter is defensive; in practice 1s × `u64::MAX` is ~580 billion years.

Helper:

```rust
/// Returns `true` if the wait-for-activation loop should emit a heartbeat log
/// at the given 1-based cycle counter (i.e. seconds elapsed since the
/// initial "not yet activated" log).
///
/// Logs fire at cycles 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, then every
/// 1024 cycles thereafter. Cycle 0's first log is emitted by the caller
/// before entering the loop, so this fn returns `false` for 0; cycle 1 is
/// intentionally silent — `1.is_power_of_two()` is `true` but the spec
/// excludes it.
pub fn should_log(cycle: u64) -> bool {
    if cycle < 2 {
        return false;
    }
    if cycle <= 1024 {
        return cycle.is_power_of_two();
    }
    cycle % 1024 == 0
}
```

`should_log` is the contract for the logging schedule; cycle 0's first log is a separate `info!` outside the loop.

Register in `agent/src/app/mod.rs`:

```rust
pub mod errors;
pub mod options;
pub mod run;
pub mod state;
pub mod upgrade;
pub mod wait_for_activation;  // <-- ADD THIS
```

(Adjust to match the actual existing list in that file.)

### M2 — Wire `run_agent()` in `agent/src/main.rs`

Add to the `internal crates` import group:

```rust
use miru_agent::app::wait_for_activation::{wait_for_activation, WaitOutcome};
```

Replace lines 160-164:

```rust
// check the agent has been activated
if let Err(e) = storage::assert_activated(&layout).await {
    error!("Device is not yet activated: {}", e);
    return;
}
```

with:

```rust
// wait for the device to be activated (or a shutdown signal)
match wait_for_activation(&layout, tokio::time::sleep, await_shutdown_signal()).await {
    WaitOutcome::Activated => {}
    WaitOutcome::ShutdownRequested => return,
}
```

Everything below (lines 166+) is unchanged. The `await_shutdown_signal()` later in `run_agent()` (line 227) creates a *fresh* signal subscription, which is what we want — the wait phase consumes its own subscription and the server phase consumes another. (`tokio::signal::unix::signal` registers a new handler per call; multiple registrations all receive each signal.)

If `error!` is no longer used after this change, drop it from the `use tracing::{error, info};` import. Verify with grep first — it's likely still referenced by other arms in `run_agent`.

### M3 — Unit tests for `should_log`

Add a `#[cfg(test)] mod tests` block at the bottom of `agent/src/app/wait_for_activation.rs`. Per `AGENTS.md` convention, integration tests live under `agent/tests/`, but a pure-function predicate is naturally a unit test alongside the source. The integration test file in M4 also `use`s `should_log` to lock in the public-API expectation.

```rust
#[cfg(test)]
mod tests {
    use super::should_log;

    #[test]
    fn cycle_zero_is_silent_caller_logs_first_miss() {
        // Cycle 0 logging happens outside the loop in `wait_for_activation`,
        // so `should_log(0) == false` — otherwise we'd double-log.
        assert!(!should_log(0));
    }

    #[test]
    fn cycle_one_is_silent() {
        assert!(!should_log(1));
    }

    #[test]
    fn powers_of_two_through_1024_log() {
        for n in [2u64, 4, 8, 16, 32, 64, 128, 256, 512, 1024] {
            assert!(should_log(n), "expected log at cycle {n}");
        }
    }

    #[test]
    fn multiples_of_1024_above_cap_log() {
        for n in [2048u64, 3072, 4096, 5120, 102_400, 1_048_576] {
            assert!(should_log(n), "expected heartbeat at cycle {n}");
        }
    }

    #[test]
    fn off_cycle_values_are_silent() {
        for n in [3u64, 5, 7, 9, 17, 31, 63, 127, 255, 511, 513, 1023, 1025, 2047, 3071] {
            assert!(!should_log(n), "expected silent at cycle {n}");
        }
    }

    #[test]
    fn powers_of_two_above_1024_only_log_when_also_multiple_of_1024() {
        // 2048, 4096, 8192 are all multiples of 1024 AND powers of 2 — they
        // log via the heartbeat branch. But e.g. cycle 2049 is silent even
        // though we're past the cap.
        assert!(!should_log(2049));
        assert!(!should_log(4097));
    }
}
```

### M4 — Integration tests in `agent/tests/app/wait_for_activation.rs`

New file. Register in `agent/tests/app/mod.rs`:

```rust
// existing modules ...
pub mod wait_for_activation;
```

File content:

```rust
// standard crates
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration as StdDuration;

// internal crates
use miru_agent::app::wait_for_activation::{wait_for_activation, should_log, WaitOutcome};
use miru_agent::filesys::{self, WriteOptions};
use miru_agent::storage::Layout;

// external crates
// (none required — stdlib + tokio macros)

// ============================ TEST HARNESS ============================ //

async fn fresh_layout(name: &str) -> (Layout, filesys::Dir) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let layout = Layout::new(dir.clone());
    layout.auth().root.create_if_absent().await.unwrap();
    (layout, dir)
}

async fn write_keys(layout: &Layout) {
    let auth = layout.auth();
    auth.private_key()
        .write_string("private", WriteOptions::OVERWRITE_ATOMIC)
        .await.unwrap();
    auth.public_key()
        .write_string("public", WriteOptions::OVERWRITE_ATOMIC)
        .await.unwrap();
}

// ============================ TESTS ============================ //

#[tokio::test]
async fn activates_immediately_when_keys_already_present() {
    let (layout, _dir) = fresh_layout("wait_activates_immediately").await;
    write_keys(&layout).await;

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let sleep_fn = move |_: StdDuration| {
        let counter = counter.clone();
        async move { counter.fetch_add(1, Ordering::SeqCst); }
    };

    let shutdown = std::future::pending::<()>();

    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::Activated);
    assert_eq!(
        sleep_count.load(Ordering::SeqCst), 0,
        "should not sleep even once when activation is already complete"
    );
}

#[tokio::test]
async fn activates_after_n_cycles() {
    let (layout, dir) = fresh_layout("wait_activates_after_n").await;

    let activate_after: usize = 3;
    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let layout_for_sleep = Layout::new(dir.clone());

    // The sleep_fn writes the keys on its Nth invocation. Ordering inside the
    // production loop is: assert_activated → sleep → assert_activated → sleep
    // → ... so writing keys during the Nth sleep means the (N+1)th
    // assert_activated check sees them.
    let sleep_fn = move |_: StdDuration| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let layout = layout_for_sleep.clone();
        async move {
            if n + 1 == activate_after {
                let auth = layout.auth();
                auth.private_key()
                    .write_string("private", WriteOptions::OVERWRITE_ATOMIC)
                    .await.unwrap();
                auth.public_key()
                    .write_string("public", WriteOptions::OVERWRITE_ATOMIC)
                    .await.unwrap();
            }
        }
    };

    let shutdown = std::future::pending::<()>();
    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::Activated);
    assert_eq!(
        sleep_count.load(Ordering::SeqCst), activate_after,
        "should sleep exactly N times: assert_activated misses N times, sleep injects keys on Nth, next check succeeds"
    );
}

#[tokio::test]
async fn shutdown_during_wait_returns_shutdown_requested() {
    let (layout, _dir) = fresh_layout("wait_shutdown_during").await;
    // No keys are ever created.

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let shutdown_tx = Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx)));
    let shutdown_tx_for_sleep = shutdown_tx.clone();

    // After 5 sleeps, fire shutdown.
    let sleep_fn = move |_: StdDuration| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let tx = shutdown_tx_for_sleep.clone();
        async move {
            if n + 1 == 5 {
                if let Some(tx) = tx.lock().await.take() {
                    let _ = tx.send(());
                }
            }
        }
    };

    let shutdown = async move { let _ = shutdown_rx.await; };

    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::ShutdownRequested);
    // Sleep count is "≥ 5" not "== 5" because tokio::select! polls both
    // arms; a final sleep_fn invocation may have started before shutdown won.
    assert!(
        sleep_count.load(Ordering::SeqCst) >= 5,
        "expected at least 5 sleeps before shutdown fired"
    );
}

#[tokio::test]
async fn shutdown_wins_when_already_signaled_at_entry() {
    // No keys, shutdown future is already-resolved when we enter the loop.
    let (layout, _dir) = fresh_layout("wait_shutdown_immediate").await;

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let sleep_fn = move |_: StdDuration| {
        let counter = counter.clone();
        async move { counter.fetch_add(1, Ordering::SeqCst); }
    };

    let shutdown = std::future::ready(());

    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::ShutdownRequested);
    // `biased; shutdown` first means we should not have slept at all:
    // the first `tokio::select!` iteration sees `shutdown` ready and wins.
    assert_eq!(sleep_count.load(Ordering::SeqCst), 0);
}

#[test]
fn should_log_is_publicly_reachable_and_matches_unit_schedule() {
    // The full schedule is covered in unit tests next to the source.
    // This test just locks in the public-API path.
    assert!(!should_log(0));
    assert!(should_log(2));
    assert!(should_log(1024));
    assert!(should_log(2048));
    assert!(!should_log(2049));
}
```

Notes:

- `std::future::pending::<()>()` is a Future that never resolves — used when the test wants the shutdown branch to never fire.
- `std::future::ready(())` is the immediately-ready Future — used to assert the `biased; shutdown` ordering wins on entry.
- The oneshot-coordinated test exercises the realistic interleaving where shutdown fires during the wait. The `>= 5` assertion accounts for `tokio::select!`'s pre-poll: a started sleep_fn future may have incremented the counter in the same iteration shutdown wins.

### M5 — Final preflight

Per `AGENTS.md`:

```bash
./scripts/test.sh           # cargo test --package miru-agent --features test, exits 0
./scripts/lint.sh           # import linter, fmt, machete, audit, clippy -D warnings
./scripts/covgate.sh        # all per-module coverage gates pass
```

If `covgate.sh` flags `agent/src/app/.covgate` as below its declared floor, investigate: the new module is essentially three small functions, all of which are exercised by either unit tests (`should_log`) or integration tests (`wait_for_activation`'s four branches). If actual coverage genuinely dropped, the cause is most likely the early-return guard in `should_log` not being hit by integration tests (only unit tests). The fix is to extend the integration test sweep, not to bump the gate.

Open the PR only after all three commands exit `0`.

## Concrete Steps

### Step 0 — Confirm test harness layout and dev-deps

From the agent repo:

```bash
ls agent/tests/app/                  # confirm test dir layout (agent/agent/tests/app/ from outside)
cat agent/src/app/.covgate           # baseline floor
grep -n "fresh_layout\|create_temp_dir" agent/tests/storage/device.rs | head
```

The plan assumes `Layout::new(filesys::Dir)` and `auth().root.create_if_absent().await` are correct — verify against `agent/src/storage/layout.rs` (or wherever `Layout` is defined) before writing tests. Adjust the `fresh_layout` helper if the actual API differs (e.g. if `Layout::new` takes a `PathBuf` instead).

### Step 1 — M1 (create the helper module)

Create `agent/src/app/wait_for_activation.rs` with the contents listed in M1 (including the inner `#[cfg(test)] mod tests`). Add `pub mod wait_for_activation;` to `agent/src/app/mod.rs`.

```bash
cargo build -p miru-agent --features test
cargo test  -p miru-agent --features test --lib app::wait_for_activation
```

Expected: builds clean; the 6 unit tests in `mod tests` pass.

### Step 2 — M2 (wire run_agent)

Edit `agent/src/main.rs`:

1. Add `use miru_agent::app::wait_for_activation::{wait_for_activation, WaitOutcome};` to the `// internal crates` group, in the alphabetical position consistent with the existing `miru_agent::app::*` imports.
2. Replace lines 160-164 with the `match wait_for_activation(...) { ... }` block from M2.
3. Verify `tracing::{error, info}` import is still needed; drop `error` if unused.

```bash
cargo build -p miru-agent --features test
```

Expected: builds clean. (No new tests yet at this step — `main.rs` is the binary entry point and is not unit-tested directly.)

### Step 3 — M3 (unit tests for `should_log`)

`should_log` unit tests already landed in M1's source file. If M1 was committed without them (e.g. if implementer staged the helper but skipped the test mod), add them now and run:

```bash
./scripts/test.sh -- app::wait_for_activation::tests
```

Expected: 6 unit tests pass.

### Step 4 — M4 (integration tests)

Create `agent/tests/app/wait_for_activation.rs` with the contents from M4. Add `pub mod wait_for_activation;` to `agent/tests/app/mod.rs`.

```bash
./scripts/test.sh -- app::wait_for_activation
```

Expected: 5 integration tests + 6 unit tests = 11 passing tests, all under the `app::wait_for_activation` test path.

### Step 5 — M5 (full preflight)

From the agent repo root:

```bash
./scripts/test.sh
./scripts/lint.sh
./scripts/covgate.sh
```

Expected: all three exit `0`. Record the new `agent/src/app/` directory coverage percentage in Outcomes & Retrospective.

If lint flags an unused import in `main.rs` after M2, drop it. If clippy flags the new module, fix forward in the same commit (the new code is the source of the warning).

If preflight passes, the branch is ready to PR.

## Validation and Acceptance

The following test names must all pass under `./scripts/test.sh`:

- Unit (`agent/src/app/wait_for_activation.rs::tests`):
  - `cycle_zero_is_silent_caller_logs_first_miss`
  - `cycle_one_is_silent`
  - `powers_of_two_through_1024_log`
  - `multiples_of_1024_above_cap_log`
  - `off_cycle_values_are_silent`
  - `powers_of_two_above_1024_only_log_when_also_multiple_of_1024`
- Integration (`agent/tests/app/wait_for_activation.rs`):
  - `activates_immediately_when_keys_already_present`
  - `activates_after_n_cycles`
  - `shutdown_during_wait_returns_shutdown_requested`
  - `shutdown_wins_when_already_signaled_at_entry`
  - `should_log_is_publicly_reachable_and_matches_unit_schedule`

**Preflight must report `clean` before changes are published.** From the agent repo:

- `./scripts/test.sh` — exits `0`, no failed/ignored tests.
- `./scripts/lint.sh` — exits `0`, no clippy warnings, no fmt diffs, no import-linter findings, no unused deps, no security advisories.
- `./scripts/covgate.sh` — every per-module covgate ≥ its declared floor; in particular `agent/src/app/.covgate` must still pass.

Open the PR only after this preflight is clean.

Acceptance: a novice running `./scripts/test.sh` after pulling this branch sees the 11 new tests pass, and manually starting the agent on a device with no `auth/` keys observes the agent stay running and log "Device is not yet activated; waiting for provisioning..." rather than exiting. Running `miru provision` in a side terminal causes the running agent to advance within ~1 second and log "Device activated; starting agent." Sending SIGTERM during the wait causes a clean shutdown (exit 0) with the log line "Shutdown received while waiting for activation".

## Idempotence and Recovery

- All edits are safe to re-run; M1-M4 are pure additions to new files plus one block-replace in `main.rs`. Re-applying is a no-op once the files match the plan.
- If a test in M4 hangs, the cause is almost certainly the shutdown future never resolving and the keys never being written — i.e. a missing branch in the test's coordinating closure. Recovery: `Ctrl-C`, locate the test via `./scripts/test.sh -- --nocapture app::wait_for_activation`, and ensure the shutdown future or the key-creation closure has a definite trigger. Do **not** add `tokio::time::timeout` — fix the test fixture instead (no real sleep).
- If `covgate.sh` fails, re-read the new module — three pure functions and one async loop should hit ≥95% line coverage easily; if a branch is missing, extend a test rather than bumping the gate.
- Each milestone ends in its own commit. If preflight fails, fix forward in a new commit — do not amend.
