# Prune the scanner ledger on the same cadence as scanning

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add ledger-pruning to the scan driver worker (`agent/src/workers/scan.rs`), inject a test-friendly `now` source, thread a new retention option through app wiring (`agent/src/app/options.rs`, `agent/src/app/run.rs`), and add worker unit tests. |

This plan lives in `agent/plans/backlog/` because all code changes are in the `agent` crate. (Note: the repo's own `.agents/skills/plan` policy nominally points at `.agents/exec-plans/backlog/`; this task explicitly directs the plan to `agent/plans/backlog/`, so that is where it lives.)

The Rust crate root is `agent/agent/` (the crate's `src/` is at `agent/agent/src/...`). All paths below are given relative to the repo root `/home/user/agent`.

## Purpose / Big Picture

The scanner maintains a per-collection **ledger**: a dedup record of files that have already gone "stable" and been handed off for upload, so the same file is never re-reported. The ledger only ever grows — every newly stable file adds an entry and nothing removes it in production today. Over a long-lived device deployment that record grows without bound (and is persisted to the on-disk snapshot every scan), wasting memory and disk and slowing snapshot writes.

The scanner already knows how to bound the ledger: `ScannerExt::prune(before)` drops ledger entries whose latest observation is older than `before`. It is fully implemented and unit-tested but **nothing in production calls it**. This change wires `prune` into the one place that already drives the scanner on a fixed cadence — the scan driver worker — so the ledger is pruned on every scan tick using a configurable retention window.

After this change, a running agent keeps its scanner ledger bounded automatically: entries whose latest stable file was first observed more than `ledger_retention_secs` ago are dropped on each scan tick, with no new moving parts, no new worker, and no new schedule. Failures to prune are logged and never stop scanning.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Add `ledger_retention_secs` to `workers::scan::Options` + `Default`.
- [ ] Inject a `now_fn` closure into `workers::scan::run` / `run_impl` mirroring the existing `sleep_fn` injection.
- [ ] Compute `before = now_fn() - ledger_retention_secs` and call `scanner.prune(before)` after each scan (initial pass + every loop tick), logging errors and continuing.
- [ ] Pass `chrono::Utc::now` as the production `now_fn` at the single call site in `agent/src/app/run.rs::init_scan_worker`.
- [ ] Confirm the new option flows through `AppOptions::default()` via `Default::default()` (no explicit wiring needed).
- [ ] Add a `#[cfg(test)] mod tests` to `agent/src/workers/scan.rs`: a recording fake `ScannerExt`, a controllable clock, and a driven `sleep_fn`; assert prune is invoked on cadence with the correct `before` cutoff.
- [ ] Run preflight to CLEAN (CI green on pushed branch head) before the PR leaves draft / the task is reported complete.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: The scan driver worker (`agent/src/workers/scan.rs`) currently has **no test module at all** — none of the workers except `mod.rs` do. The fake `ScannerExt` and clock this plan requires must be written fresh; there is nothing to reuse.
  Evidence: `grep -rln "impl ScannerExt" agent/src/` returns only `agent/src/scan/scanner.rs` (the real `Scanner`). `agent/src/workers/scan.rs` is 70 lines with no `#[cfg(test)]`.
- Observation: `AppOptions` builds its scanner options with `scanner: Default::default()` (`agent/src/app/options.rs:82`), so a new field on `workers::scan::Options` with a `Default` value flows through automatically — no change to `AppOptions::default()` is required.
  Evidence: `agent/src/app/options.rs:61-85`.

## Decision Log

- Decision: Retention default is **86_400 seconds (24 hours)**, stored as `ledger_retention_secs: i64` on `workers::scan::Options`.
  Rationale: The ledger's job is dedup — an entry must survive at least as long as the file it records could still be re-observed and mistakenly re-reported. A fixed 24h window comfortably outlives transient conditions (device reboots, network drops, agent restarts, redeploys) while still bounding growth to at most one day of stable files. A fixed wall-clock window is chosen over "a multiple of `scan_interval_secs`" because retention is about *how long dedup memory must live*, which is a property of the workload, not of how often we happen to poll; coupling it to the 60s scan cadence would make a faster scan interval silently shrink dedup memory. `i64` matches the existing `scan_interval_secs: i64` type and the `chrono::Duration::seconds(i64)` API.
  Date/Author: 2026-07-15 / plan author.

- Decision: Prune runs **after** scan on every tick, **including** the initial immediate pass.
  Rationale: (a) Order is correctness-neutral for freshly stabilized files — `scan` stamps their `first_observed_at` at "now", and `before = now - retention` is strictly earlier, so a just-added entry is always retained regardless of whether prune runs before or after. Putting prune after keeps the primary work (scan) first and treats pruning as trailing maintenance. (b) Pruning on the initial pass matters: on boot the scanner restores its ledger from the persisted on-disk snapshot, which may already be large and stale; pruning immediately bounds a restored ledger at startup instead of waiting a full `scan_interval_secs`.
  Date/Author: 2026-07-15 / plan author.

- Decision: `now` is obtained via an **injected `now_fn: Fn() -> DateTime<Utc>` closure** added to `run`/`run_impl`, mirroring the existing `sleep_fn` injection.
  Rationale: The worker has no current time source. The sibling scanner actor already uses exactly this pattern (`ScannerArgs::now_fn: Arc<dyn Fn() -> DateTime<Utc>>`, defaulting to `Utc::now`), and the worker already injects `sleep_fn` "for testing purposes". A closure keeps the cutoff computation deterministic under test (a controllable clock) while production passes `chrono::Utc::now`. This is strictly more testable than calling `Utc::now()` inline.
  Date/Author: 2026-07-15 / plan author.

- Decision: Prune failure is logged with `error!(...)` and the loop continues, mirroring scan-error handling.
  Rationale: Pruning is best-effort maintenance; a transient prune failure (e.g. a snapshot-persist hiccup) must never kill the driver and stop all future scanning. This matches the existing `if let Err(e) = scanner.scan().await { error!(...) }` pattern in the same function.
  Date/Author: 2026-07-15 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

New reader, zero prior context. Here is everything needed.

### The scanner and its ledger

`agent/src/scan/scanner.rs` defines a scanner actor. `SingleThreadScanner` holds a map of per-collection scanners. Each collection scanner keeps a **ledger**: `CollectionState.ledger: HashMap<File, Vec<StableFile>>` (`agent/src/scan/state.rs:19`). When a watched file stops changing for its stability window it becomes a `StableFile`, is emitted to subscribers exactly once, and an entry is recorded in the ledger so it is never re-reported. The ledger is persisted to disk on every scan via `persist_snapshot`.

`StableFile.first_observed_at` is the timestamp when the file was first seen as a candidate. Pruning uses it:

    // agent/src/scan/state.rs:86
    pub(crate) fn prune_ledger(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.ledger.retain(|_, stable_files| {
            stable_files
                .last()
                .is_none_or(|stable_file| stable_file.first_observed_at >= before)
        });
        Ok(())
    }

So `prune(before)` **keeps** entries whose latest stable file has `first_observed_at >= before` and **drops** the rest. To keep the last `retention` seconds of history, pass `before = now - retention`.

The public actor handle exposes this already (`agent/src/scan/scanner.rs`):

    #[allow(async_fn_in_trait)]
    pub trait ScannerExt {
        async fn clear_rules(&self) -> Result<(), ScanErr>;
        async fn update_rules(&self, deployment: Deployment, rules: Vec<UploadRule>) -> Result<(), ScanErr>;
        async fn scan(&self) -> Result<(), ScanErr>;
        async fn subscribe(&self) -> Result<broadcast::Receiver<ScanEvent>, ScanErr>;
        async fn shutdown(&self) -> Result<(), ScanErr>;
        async fn prune(&self, before: DateTime<Utc>) -> Result<(), ScanErr>;
    }

`prune` is fully implemented (routes through `Command::Prune` to `SingleThreadScanner::prune`, which calls `prune_ledger` on each collection and persists) and unit-tested in `scanner.rs` (`mod prune`) and `state.rs`/`collection.rs`. **Nothing in production calls it** — that is the gap this plan closes.

### The scan driver worker (the file we change)

`agent/src/workers/scan.rs` (full current contents):

    // External driver for the `crate::scan` scanner actor. The actor is reactive,
    // not self-scheduling: this worker imposes the cadence that drives repeated
    // scan passes.

    // standard crates
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Duration;

    // internal crates
    use crate::scan::ScannerExt;

    // external crates
    use tracing::{debug, error, info};

    #[derive(Debug, Clone)]
    pub struct Options {
        pub scan_interval_secs: i64,
    }

    impl Default for Options {
        fn default() -> Self {
            Self {
                scan_interval_secs: 60,
            }
        }
    }

    pub async fn run<F, Fut, ScannerT: ScannerExt>(
        options: &Options,
        scanner: &ScannerT,
        sleep_fn: F,
        mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
    ) where
        F: Fn(Duration) -> Fut,
        Fut: Future<Output = ()> + Send,
    {
        tokio::select! {
            _ = shutdown_signal.as_mut() => {
                info!("Scan driver worker shutdown complete");
            }
            // doesn't return but we do need to run it in the background
            _ = run_impl(options, scanner, sleep_fn) => {}
        }
    }

    async fn run_impl<F, Fut, ScannerT: ScannerExt>(
        options: &Options,
        scanner: &ScannerT,
        sleep_fn: F, // for testing purposes
    ) where
        F: Fn(Duration) -> Fut,
        Fut: Future<Output = ()> + Send,
    {
        info!("Running scan driver worker");

        // drive an initial scan immediately, then on a fixed cadence
        if let Err(e) = scanner.scan().await {
            error!("scan driver: initial scan failed: {e:?}");
        }

        let interval = Duration::from_secs(options.scan_interval_secs.max(0) as u64);
        loop {
            debug!("scan driver: sleeping {interval:?} until next scan");
            sleep_fn(interval).await;
            if let Err(e) = scanner.scan().await {
                error!("scan driver: scan failed, continuing: {e:?}");
            }
        }
    }

The `sleep_fn` injection (`F: Fn(Duration) -> Fut`) is the model to copy for `now_fn`: production passes `tokio::time::sleep`, tests pass a controllable fake.

### App wiring (the call site)

`agent/src/app/options.rs`:
- `AppOptions` has a field `pub scanner: scan::Options,` (line 58).
- `AppOptions::default()` sets `scanner: Default::default(),` (line 82). Because the whole `Options` is built via `Default`, adding a new field with a `Default` value requires **no change here** — it flows automatically.

`agent/src/app/run.rs::init_scan_worker` (lines 300-324) is the sole place `workers::scan::run` is invoked:

    async fn init_scan_worker(
        options: crate::workers::scan::Options,
        scanner: Arc<scan::Scanner>,
        shutdown_manager: &mut ShutdownManager,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), ServerErr> {
        info!("Initializing scan driver worker...");
        let scan_handle = tokio::spawn(async move {
            crate::workers::scan::run(
                &options,
                scanner.as_ref(),
                tokio::time::sleep,
                Box::pin(async move {
                    let _ = shutdown_rx.recv().await;
                }),
            )
            .await;
        });
        shutdown_manager.register_handle(
            |mgr| &mut mgr.scan_worker_handle,
            "scan_handle",
            scan_handle,
        )?;
        Ok(())
    }

This is called once from the app bootstrap (`agent/src/app/run.rs:157-164`, inside `if let Some(scanner) = &app_state.scanner`). It passes `options.scanner.clone()`.

`run.rs` does **not** currently import `chrono`; the new `now_fn` argument will be passed as `chrono::Utc::now` (fully-qualified) to avoid adding an import, or add `use chrono::Utc;` if the implementer prefers `Utc::now` — either is fine, keep it consistent with the file's existing style (the file uses fully-qualified `crate::...` freely).

### Terms

- **Ledger**: per-collection dedup record `HashMap<File, Vec<StableFile>>` of already-reported stable files.
- **Retention window**: how long a ledger entry is kept after its file's first observation; entries older than this are pruned. Introduced here as `ledger_retention_secs`.
- **Cutoff / `before`**: the instant `now - retention`; `prune(before)` drops entries whose latest `first_observed_at < before`.
- **`now_fn`**: injected closure returning the current `DateTime<Utc>`, so tests can control time.

## Plan of Work

All edits are in the `agent` crate. Keep changes minimal and idiomatic; match the existing terse worker style (three-section imports, `error!`-then-continue, injected closures "for testing purposes").

### Edit 1 — `agent/src/workers/scan.rs`: add the retention option

In `struct Options`, add a field:

    pub scan_interval_secs: i64,
    pub ledger_retention_secs: i64,

In `impl Default for Options`, add the default:

    scan_interval_secs: 60,
    ledger_retention_secs: 60 * 60 * 24, // 24h; bounds dedup memory, survives reboots/redeploys

Add `use chrono::{DateTime, Utc};` to the "external crates" import group (chrono is already a crate dependency — `scan/scanner.rs` and `scan/state.rs` both `use chrono::{DateTime, Utc}`).

### Edit 2 — `agent/src/workers/scan.rs`: inject `now_fn` into `run` and `run_impl`

Add a generic `now_fn` parameter to both functions, mirroring `sleep_fn`. New `run` signature:

    pub async fn run<F, Fut, N, ScannerT: ScannerExt>(
        options: &Options,
        scanner: &ScannerT,
        sleep_fn: F,
        now_fn: N,
        mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
    ) where
        F: Fn(Duration) -> Fut,
        Fut: Future<Output = ()> + Send,
        N: Fn() -> DateTime<Utc>,

Pass `now_fn` through in the `select!`:

    _ = run_impl(options, scanner, sleep_fn, now_fn) => {}

New `run_impl` signature adds the same `N: Fn() -> DateTime<Utc>` bound and `now_fn: N` parameter (place `now_fn` right after `sleep_fn`).

### Edit 3 — `agent/src/workers/scan.rs`: prune after each scan

Add a small private helper that computes the cutoff and prunes, logging failures (keeps `run_impl` short and avoids duplicating the cutoff math between the initial pass and the loop):

    async fn prune_ledger<N, ScannerT: ScannerExt>(
        scanner: &ScannerT,
        retention_secs: i64,
        now_fn: &N,
    ) where
        N: Fn() -> DateTime<Utc>,
    {
        let before = now_fn() - chrono::Duration::seconds(retention_secs.max(0));
        if let Err(e) = scanner.prune(before).await {
            error!("scan driver: prune failed, continuing: {e:?}");
        }
    }

In `run_impl`, after the initial scan add a prune, and inside the loop add a prune after each scan:

    // initial pass
    if let Err(e) = scanner.scan().await {
        error!("scan driver: initial scan failed: {e:?}");
    }
    prune_ledger(scanner, options.ledger_retention_secs, &now_fn).await;

    let interval = Duration::from_secs(options.scan_interval_secs.max(0) as u64);
    loop {
        debug!("scan driver: sleeping {interval:?} until next scan");
        sleep_fn(interval).await;
        if let Err(e) = scanner.scan().await {
            error!("scan driver: scan failed, continuing: {e:?}");
        }
        prune_ledger(scanner, options.ledger_retention_secs, &now_fn).await;
    }

Notes for the implementer:
- Use `retention_secs.max(0)` so a nonsensical negative config cannot push the cutoff into the future (which would prune everything). `scan_interval_secs` already uses `.max(0)` for the same defensive reason.
- Keep functions under the crate's size limits; the helper keeps `run_impl` compact.

### Edit 4 — `agent/src/app/run.rs`: pass the production `now_fn`

In `init_scan_worker`, add `chrono::Utc::now` as the new argument between `tokio::time::sleep` and the boxed shutdown future:

    crate::workers::scan::run(
        &options,
        scanner.as_ref(),
        tokio::time::sleep,
        chrono::Utc::now,
        Box::pin(async move {
            let _ = shutdown_rx.recv().await;
        }),
    )
    .await;

No other call sites exist (`grep -rn "workers::scan::run" agent/src` returns only this one). `AppOptions::default()` needs no change (Edit rationale above).

## Concrete Steps

All commands run from the crate root `agent/agent/` unless noted. (The workbench/agent submodule layout puts the crate at `agent/agent/`; if `Cargo.toml` is at `agent/Cargo.toml` in this checkout, run from `agent/` instead — locate it with `ls agent/Cargo.toml agent/agent/Cargo.toml`.)

1. Make Edits 1-4 above.

2. Build:

       cargo build

   Expect a clean build. A likely first error is a missing `now_fn` argument at the `init_scan_worker` call site if Edit 4 is skipped — the compiler will point at `crate::workers::scan::run`.

3. Run the worker tests (see next section for the test to add):

       cargo test --features test workers::scan

   Expect the new tests under `agent/src/workers/scan.rs` to pass. (The crate gates some scanner test helpers behind `--features test`; the fake in this plan does not require it, but run with the same feature set CI uses. Confirm the exact feature flag by checking `agent/Cargo.toml` `[features]` and how existing `#[cfg(feature = "test")]` code in `scan/scanner.rs` is exercised.)

4. Lint/format per the crate's conventions before pushing (e.g. `cargo fmt`, `cargo clippy` / the repo's lint script if present under `agent/scripts/`).

## Validation and Acceptance

### Unit test to add (primary acceptance)

Add `#[cfg(test)] mod tests` to `agent/src/workers/scan.rs`. There is **no existing worker test module or fake `ScannerExt`** — build these fresh (kept minimal, only the methods the worker calls need real behavior).

Test-support pieces:

1. **Recording fake scanner** implementing `ScannerExt`. It records each `scan()` and each `prune(before)` call. Since `ScannerExt` methods take `&self`, use interior mutability:

       struct RecordingScanner {
           scans: std::sync::Arc<std::sync::Mutex<usize>>,
           prunes: std::sync::Arc<std::sync::Mutex<Vec<chrono::DateTime<chrono::Utc>>>>,
       }

   Implement all six trait methods. `scan` increments `scans`; `prune` pushes `before` onto `prunes`; the rest (`clear_rules`, `update_rules`, `subscribe`, `shutdown`) return `Ok(())` / an unused value or `unimplemented!()` — the worker never calls them.

2. **Controllable clock** for `now_fn`: a value in an `Arc<Mutex<i64>>` (epoch seconds) with a closure `move || DateTime::from_timestamp(secs.load(...), 0).unwrap()`, so the test knows the exact `now` at each tick and can advance it. (This mirrors the `Clock` helper already living in `scan/scanner.rs` tests; a trimmed local copy is fine — do not try to import it across modules.)

3. **Driven `sleep_fn`**: because `run_impl` loops forever, the test drives it through the public `run` with a shutdown future and a `sleep_fn` that lets the test step ticks deterministically. Recommended handshake: `sleep_fn` sends "a tick elapsed" on an `mpsc` and then awaits a "proceed" `mpsc` from the test; the test receives the tick signal, advances the clock, inspects recorded calls, then releases the next tick; after the target number of ticks the test fires the shutdown signal and releases once more so `run` returns via its `select!`. A simpler alternative is a `sleep_fn` that counts calls and, on reaching N, resolves the shutdown signal itself — acceptable, but the handshake gives exact, race-free counts.

Assertions the tests MUST make:

- **Prune runs on the initial pass**: after the worker has done its immediate scan (before any `sleep_fn` tick), `prunes` has exactly one entry. Because prune runs after scan, `scans == 1` at that point too.
- **Prune runs on cadence**: after N driven ticks, `prunes.len() == N + 1` (initial pass + one per tick) and `scans == prunes.len()` (scan-then-prune each pass).
- **Correct `before` cutoff**: for the tick where the clock reads `now`, the recorded `before` equals `now - ledger_retention_secs`. Set a distinctive non-default `ledger_retention_secs` (e.g. 3600) and a known clock value so the equality is exact: `before == DateTime::from_timestamp(now_secs - 3600, 0).unwrap()`. Advancing the clock between ticks and asserting each recorded `before` tracks the moving `now` proves the cutoff is recomputed per tick from `now_fn`, not fixed.
- **Prune failure does not stop the loop** (mirrors scan-error handling): give the fake a mode where `prune` returns `Err(ScanErr::...)` for the first call; assert the worker still performs subsequent scans and prunes (loop continues). Reuse a cheaply-constructed `ScanErr` variant — check `agent/src/scan/errors.rs` for the simplest one to build in a test (e.g. an internal-error variant with a `trace!()`), matching how other tests construct `ScanErr`.

Optionally add an ordering assertion (scan recorded before prune within a tick) if the fake records a single interleaved event log; this is nice-to-have, not required.

Acceptance phrased as behavior: *Run `cargo test --features test workers::scan` from the crate root. Before this change there are no scan-worker tests. After it, the new tests pass: the fake scanner shows `prune` called once immediately and once per scan tick, each with `before == now - ledger_retention_secs` evaluated from the injected clock, and a forced `prune` error does not stop later ticks.*

### System-level check

There is no separate runtime command to exercise this (it is internal maintenance). The unit tests above are the acceptance surface. A reviewer can additionally read `init_scan_worker` in `agent/src/app/run.rs` and confirm `chrono::Utc::now` is passed as `now_fn`, and read `AppOptions::default()` to confirm `ledger_retention_secs` defaults to 24h via `Default`.

### Preflight / CI gate (mandatory)

Preflight MUST report **CLEAN** — CI green on the pushed branch head — before the PR leaves draft or the task is reported complete. Do not mark the work done on a red or unknown CI state. Heavyweight validation (lint, tests, coverage) runs in GitHub Actions, not locally; drive fixes from the CI job logs until the workflow passes on the pushed head of the working branch `claude/practical-newton-1dsjq9`.

## Idempotence and Recovery

- All edits are additive and safe to re-apply; re-running the build/test commands is non-destructive.
- If `cargo build` fails at the `init_scan_worker` call site, Edit 4 (adding `chrono::Utc::now`) is missing or misplaced — the new `now_fn` argument goes between `sleep_fn` and the boxed shutdown future.
- If a test hangs, the `sleep_fn`/shutdown handshake is deadlocked: ensure the test releases one final tick *after* firing the shutdown signal so `run`'s `select!` can observe shutdown, and that `run` (not `run_impl`) is the entry point under test.
- No migrations, no on-disk format changes: `prune` reuses the existing snapshot-persist path, and pruning an already-pruned ledger is a no-op.
- Do not switch branches; all work happens on `claude/practical-newton-1dsjq9`.

---

Change note (2026-07-15): Initial authoring. Investigated and verified against the working tree: `prune`/`prune_ledger` semantics, the worker's `sleep_fn` injection pattern, the scanner actor's `now_fn` precedent, the single `init_scan_worker` call site, and the `Default`-based flow of `scanner` options through `AppOptions`. Recorded the absence of any existing worker test module (fakes must be written fresh).
